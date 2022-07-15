#[macro_use]
extern crate log;

use std::collections::{HashMap, HashSet};
use std::io;
use std::time::{Duration, Instant};

use actix::prelude::*;

use actix_web::{middleware, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;

use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde_derive::{Deserialize, Serialize};

// ---- Constants ----

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

// ---- Define the shape of the configuration object ----

#[derive(Clone, Debug, Deserialize)]
struct ConfigLogging {
    actix_server: String,
    actix_web: String,
    chat: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ConfigUserAuth {
    allowed_jids: Vec<String>,
    token_validity: u8,
}

#[derive(Clone, Debug, Deserialize)]
struct ConfigLocation {
    devices: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ConfigHTTP {
    host: String,
    port: u16,
    key: String,
    cert: String,
    cacert: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Config {
    http: ConfigHTTP,
    logging: Option<ConfigLogging>,
    userauth: ConfigUserAuth,
    locations: HashMap<String, ConfigLocation>,
}

// ---- Protocol messages for chat client-server communication ----

/// Chat server sends this messages to session
#[derive(Clone, Debug, Message)]
#[rtype(result = "()")]
struct ProtoMessage(String);

/// Implement formatting so ProtoMessage can be printed out
impl std::fmt::Display for ProtoMessage {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        fmt.write_str(self.0.as_str())?;
        Ok(())
    }
}

/// New client just connected
#[derive(Message)]
#[rtype(result = "()")]
struct Connect {
    jid: String,
    addr: Recipient<ProtoMessage>,
}

/// Client that recently connected sends its capabilities
#[derive(Message)]
#[rtype(result = "()")]
struct Capabilities {
    jid: String,
    capabilities: HashSet<String>,
}

/// Client disconnected
#[derive(Message)]
#[rtype(result = "()")]
struct Disconnect {
    jid: String,
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
struct Peer {
    online: bool,
    capabilities: HashSet<String>,
}

impl Peer {
    fn new(online: bool, capabilities: HashSet<String>) -> Self {
        Self { online, capabilities }
    }
}

/// List clients connected to the server excluding sender's own JID
struct ListPeers {
    jid: String,
}

impl Message for ListPeers {
    type Result = HashMap<String, Peer>;
}

/// Relay message to another user
#[derive(Debug, Message, Serialize)]
#[rtype(result = "()")]
struct RelayMessage {
    /// From which jid the message is coming from
    from_jid: String,
    /// To which JID this message should be sent
    to_jid: String,
    /// The message to be sent. It's a JSON message but the client
    /// receiving it should decode it, not the protocol server.
    message: protocol::Message,
}

// ----- Server Implementation ----

/// Client data the server needs to keep track of
#[derive(Clone)]
struct ClientInfo {
    addr: Recipient<ProtoMessage>,
    capabilities: HashSet<String>,
}

impl ClientInfo {
    /// Construct an instance of the ClientInfo taking the address of
    /// the client received during connection time.
    fn new(addr: Recipient<ProtoMessage>) -> Self {
        ClientInfo {
            addr,
            capabilities: HashSet::<String>::new(),
        }
    }
}

/// The server keeps track of all connected clients.  Clients are
/// registered in the server when they hit the websocket endpoint.
#[derive(Clone)]
struct ChatServer {
    clients: HashMap<String, ClientInfo>,
}

impl ChatServer {
    fn new() -> Self {
        debug!("New ProtocolServer created");
        Self {
            clients: HashMap::new(),
        }
    }

    fn broadcast(&mut self, msg: ProtoMessage, exclude_jid: Option<&String>) {
        for (key, client) in &self.clients {
            if let Some(excluded) = &exclude_jid {
                if *key == **excluded {
                    continue;
                }
            }

            match client.addr.do_send(msg.clone()) {
                Ok(_) => debug!("Broadcast to client {}: {}", key, msg.clone()),
                Err(e) => error!("Couldn't message client {}: {}", key, e),
            }
        }
    }
}

/// Make actor from `ChatServer`
impl Actor for ChatServer {
    /// We are going to use simple Context, we just need ability to
    /// communicate with other actors.
    type Context = Context<Self>;
}

/// Handler for Connect messages from ChatConnection actors.
impl Handler<Connect> for ChatServer {
    type Result = ();

    /// Insert the newly connected client into the clients hash table.
    fn handle(&mut self, msg: Connect, _ctx: &mut Self::Context) {
        self.clients.insert(msg.jid, ClientInfo::new(msg.addr));
    }
}

impl Handler<Disconnect> for ChatServer {
    type Result = ();

    /// Remove client a connection from the clients hash table.
    fn handle(&mut self, msg: Disconnect, _ctx: &mut Self::Context) {
        // First update the clients list
        self.clients.remove(&msg.jid);
        // Then finally inform the currently connected clients about
        // the event
        let forward = protocol::Envelope {
            from_jid: msg.jid.clone(),
            to_jid: "".to_string(),
            message: protocol::Message::PeerOffline,
        };
        let forward_str = serde_json::to_string(&forward).unwrap();
        self.broadcast(ProtoMessage(forward_str), Some(&msg.jid));
    }
}

/// Handler for Connect messages from ChatConnection actors.
impl Handler<Capabilities> for ChatServer {
    type Result = ();

    /// Insert the newly connected client into the clients hash table.
    fn handle(&mut self, msg: Capabilities, _ctx: &mut Self::Context) {
        match self.clients.get_mut(&msg.jid) {
            None => error!("Can't set capabilities, client `{}' not connected", msg.jid),
            Some(client) => {
                client.capabilities = msg.capabilities.clone();
                let forward = protocol::Envelope {
                    from_jid: msg.jid.clone(),
                    to_jid: "".to_string(),
                    message: protocol::Message::PeerOnline {
                        capabilities: msg.capabilities,
                    },
                };
                let forward_str = serde_json::to_string(&forward).unwrap();
                self.broadcast(ProtoMessage(forward_str), Some(&msg.jid));
            }
        }
    }
}

impl Handler<ListPeers> for ChatServer {
    type Result = MessageResult<ListPeers>;

    /// Return a list with the JIDs of all currently connected clients
    fn handle(&mut self, msg: ListPeers, _ctx: &mut Self::Context) -> Self::Result {
        let mut output: HashMap<String, Peer> = HashMap::new();
        for (key, client_info) in &self.clients {
            // we don't report the JID of whoever asked for this list
            if *key == msg.jid {
                continue;
            }

            let peer = Peer::new(true, client_info.capabilities.clone());

            output.insert(key.clone(), peer);
        }
        MessageResult(output)
    }
}

impl Handler<RelayMessage> for ChatServer {
    type Result = ();

    /// Relay received message to a given client
    fn handle(&mut self, msg: RelayMessage, _ctx: &mut Self::Context) {
        let message = serde_json::to_string(&msg).unwrap();
        match self.clients.get(&msg.to_jid) {
            None => error!("Client `{}' not connected", msg.to_jid),
            Some(client) => client.addr.do_send(ProtoMessage(message)).unwrap(),
        }
    }
}

// ---- Chat Connection implementation ----

/// Each new client instantiates a ChatConnection.  The `jid'
/// identifies either device or person.  The `heartbeat' tracks the
/// health of the connection.
struct ChatConnection {
    jid: String,
    heartbeat: Instant,
    server: Addr<ChatServer>,
}

/// Define the methods needed for a ChatConnection object
impl ChatConnection {
    fn new(jid: String, server: Addr<ChatServer>) -> Self {
        Self {
            jid,
            server,
            heartbeat: Instant::now(),
        }
    }

    /// Update the heartbeat of the connection to right now
    fn heartbeat_update(&mut self) {
        self.heartbeat = Instant::now();
    }

    /// Check heartbeat on intervals (HEARTBEAT_INTERVAL)
    fn heartbeat_check(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                println!("Websocket Client heartbeat failed, disconnecting!");
                ctx.stop();
            } else {
                ctx.ping(b"");
            }
        });
    }

    /// Send a message to the ChatServer actor in order to register
    /// the ChatConnection within the server.
    fn register(&self, ctx: &mut ws::WebsocketContext<Self>) {
        info!("Client connected: {}", self.jid);
        self.server.do_send(Connect {
            jid: self.jid.clone(),
            addr: ctx.address().recipient(),
        });
    }

    /// Get itself removed from the ChatServer actor
    fn deregister(&self) {
        info!("Client disconnected: {}", self.jid);
        self.server.do_send(Disconnect {
            jid: self.jid.clone(),
        });
    }

    fn _handle_message(&self, msg: &String) -> Result<(), serde_json::Error> {
        let deserialized: protocol::Envelope = serde_json::from_str(msg.as_str())?;

        match deserialized.message {
            protocol::Message::PeerCaps(capabilities) => {
                self.server.do_send(Capabilities {
                    jid: deserialized.from_jid,
                    capabilities,
                });
            }
            relay => {
                self.server.do_send(RelayMessage {
                    from_jid: deserialized.from_jid,
                    to_jid: deserialized.to_jid,
                    message: relay,
                });
            }
        }

        Ok(())
    }

    /// Handle incoming messages from clients
    fn handle_message(&self, msg: String) {
        debug!("Message from {}: {}", self.jid, msg);

        if let Err(err) = self._handle_message(&msg) {
            error!("error parsing message from client {}: {:?}", self.jid, err);
        }
    }
}

/// Define HTTP Actor for the ChatConnection struct
impl Actor for ChatConnection {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start and perform some
    /// initialization tasks like a) sending a message to the actor
    /// server to register itself and b) initialize heartbeat
    /// watchdog.
    fn started(&mut self, ctx: &mut <Self as Actor>::Context) {
        self.heartbeat_check(ctx);
        self.register(ctx);
    }

    /// Called when te actor is shutting down, it will notify the
    /// server that this client is gone so the server can keep the
    /// list of connected clients accurate.
    fn stopping(&mut self, _: &mut ws::WebsocketContext<Self>) -> Running {
        self.deregister();
        Running::Stop
    }
}

/// Define the handler for messages from ChatServer.
impl Handler<ProtoMessage> for ChatConnection {
    type Result = ();

    fn handle(&mut self, msg: ProtoMessage, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

/// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChatConnection {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.heartbeat_update();
                ctx.pong(&msg)
            }
            Ok(ws::Message::Pong(_)) => {
                self.heartbeat_update();
            }
            Ok(ws::Message::Text(text)) => {
                self.handle_message(text);
            }
            Ok(ws::Message::Close(_)) => {
                // close connection as the client has already disconnected
                ctx.stop();
            }
            xxx => {
                error!("Something unexpected came along: {:?}", xxx);
                ctx.stop();
            }
        }
    }
}

// ---- HTTP API ----

/// List currently connected clients
async fn http_api_peers(
    req: HttpRequest,
    server: web::Data<Addr<ChatServer>>,
) -> Result<HttpResponse, actix_web::Error> {
    match read_jid_from_request(&req) {
        None => Ok(HttpResponse::Unauthorized().finish()),
        Some(Err(_e)) => Ok(HttpResponse::BadRequest().finish()),
        Some(Ok(jid)) => {
            let peers = server.send(ListPeers { jid }).await?;
            Ok(HttpResponse::Ok().json(peers))
        }
    }
}

// ---- HTTP Server Handling ----

#[derive(Debug)]
struct Error {}

impl Error {
    fn new() -> Self {
        Self {}
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Error")
    }
}

impl From<base64::DecodeError> for Error {
    fn from(_err: base64::DecodeError) -> Self {
        Error::new()
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(_err: std::str::Utf8Error) -> Self {
        Error::new()
    }
}

/// Represents the data that arrives from the authentication form.
#[derive(Debug, Deserialize)]
struct AuthForm {
    jid: String,
}

/// Authenticate the user.  It takes the user from the request body
/// and perform the following process:
///
///   1. check if the admin has allowed that user to log in.
///   2. Generate a JWT token with the server's private key.
///   3. Send an email to the user with a link to the application with
///      the token embedded on it.
///
/// This method *DOES NOT* require authentication.  This is in fact,
/// the only entry point of the web application that doesn't require
/// authentication because that's the door to the street.
async fn auth(config: web::Data<Config>, body: web::Json<AuthForm>) -> impl Responder {
    for email in &config.userauth.allowed_jids {
        // Authentication doesn't need full JID
        let jid: Vec<&str> = body.jid.split('/').collect();
        if *email == jid[0] {
            // TODO: Generate token for user
            // TODO: Send token via email
            return HttpResponse::Ok().finish();
        }
    }
    HttpResponse::Unauthorized().finish()
}

/// This endpoint starts the WebSocket connection for a new client.
///
/// Welcoming a new client takes the following steps:
///  1. Pull the JWT from HTTP header
///  2. Decode & Check for its validity
///  3. Create a ClientConnection instance from the JID read from the
///     JWT token
///  4. Register newly created client connection into the server
///  4. Start a WebSocket session
async fn ws(
    req: HttpRequest,
    stream: web::Payload,
    server: web::Data<Addr<ChatServer>>,
) -> Result<HttpResponse, actix_web::Error> {
    match read_jid_from_request(&req) {
        None => Ok(HttpResponse::Unauthorized().finish()),
        Some(Err(_e)) => Ok(HttpResponse::BadRequest().finish()),
        Some(Ok(jid)) => {
            let client = ChatConnection::new(jid, server.get_ref().clone());
            ws::start(client, &req, stream)
        }
    }
}

#[derive(Deserialize)]
struct QueryAuthParams {
    token: String,
}

/// Retrieve the auth token from the query string
fn get_auth_token(req: &HttpRequest) -> Result<String, serde_qs::Error> {
    let qs: QueryAuthParams = serde_qs::from_str(req.query_string())?;
    Ok(qs.token)
}

/// Retrieve the `Authorization' header from the request's headers
fn get_auth_header<'a>(req: &'a HttpRequest) -> Option<&'a str> {
    req.headers().get("Authorization")?.to_str().ok()
}

/// Parse token from within auth header and extract
///
/// TODO: Right now, this function returns a string with the JID
/// itself instead returning the struct with the claims and stuff.
/// Mostly because we actually don't have tokens yet.
fn decode_token_from_header(authorization: &str) -> Result<String, Error> {
    let decoded = base64::decode(&authorization[7..])?;
    Ok(std::str::from_utf8(&decoded)?.to_string())
}

/// Decode & Check JWT token from HTTP header or QueryString
fn read_jid_from_request(req: &HttpRequest) -> Option<Result<String, Error>> {
    if let Ok(token) = get_auth_token(req) {
        Some(Ok(token))
    } else {
        get_auth_header(req).map(|header| decode_token_from_header(header))
    }
}

/// Applicatio error types.
#[derive(Debug)]
enum ChatError {
    IO(io::Error),
    Config(toml::de::Error),
    // Web(actix_web::Error),
}

impl From<io::Error> for ChatError {
    fn from(error: io::Error) -> Self {
        ChatError::IO(error)
    }
}

impl From<toml::de::Error> for ChatError {
    fn from(error: toml::de::Error) -> Self {
        ChatError::Config(error)
    }
}

impl From<ChatError> for io::Error {
    fn from(error: ChatError) -> Self {
        match error {
            ChatError::IO(e) => e,
            _ => io::Error::from(error),
        }
    }
}

fn load_config(args: &Vec<String>) -> Result<Config, ChatError> {
    if args.len() != 2 {
        // Can't move on without the configuration file
        let msg = format!("Usage: {} CONFIG-FILE", args[0]);
        Err(ChatError::IO(io::Error::new(io::ErrorKind::Other, msg)))
    } else {
        // Let's try to read the file contents
        let path = std::path::Path::new(&args[1]);
        let contents = std::fs::read_to_string(path)?;
        // If file contents are readable, we then try to parse the
        // TOML string that was read from it.
        let strcontent = contents.as_str();
        let config: Config = toml::from_str(strcontent)?;
        Ok(config)
    }
}

#[actix_rt::main]
async fn main() -> Result<(), io::Error> {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    let config: Config = load_config(&args)?;
    let bind_addr = format!("{}:{}", config.http.host, config.http.port);

    // Build SSL context
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;
    builder.set_private_key_file(&config.http.key, SslFiletype::PEM)?;
    builder.set_certificate_chain_file(&config.http.cert)?;
    builder.set_ca_file(&config.http.cacert)?;

    // Address for the server actor
    let server_actor = ChatServer::new().start();

    // Spin it all up
    let app = move || {
        App::new()
            .wrap(middleware::Logger::default())
            .data(config.clone())
            .data(server_actor.clone())
            .route("/ws", web::get().to(ws))
            .route("/auth", web::post().to(auth))
            .route("/peers", web::get().to(http_api_peers))
    };
    HttpServer::new(app)
        .bind_openssl(bind_addr, builder)?
        .run()
        .await
}
