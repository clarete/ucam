#[macro_use]
extern crate log;

use std::collections::{HashMap, HashSet};
use std::io;
use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_rt;
use actix_web;
use actix_web::{middleware, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;

use serde_derive::{Deserialize, Serialize};
use serde_json::json;

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
    allowed_emails: Vec<String>,
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

/// Client disconnected
#[derive(Message)]
#[rtype(result = "()")]
struct Disconnect {
    jid: String,
}

/// List connections
struct ListClients;

impl Message for ListClients {
    type Result = HashMap<String, HashSet<String>>;
}

/// Configure a specific client
#[derive(Message, Debug)]
#[rtype(result = "()")]
struct ConfigClient {
    jid: String,
    caps: HashSet<String>,
}

/// Relay message to another user
#[derive(Message, Debug)]
#[rtype(result = "()")]
struct RelayMessage {
    /// From which jid the message is coming from
    from_jid: String,
    /// To which JID this message should be sent
    to_jid: String,
    /// The message to be sent. It's a JSON message but the client
    /// receiving it should decode it, not the protocol server.
    message: String,
}

/// Relay message format
#[derive(Deserialize, Debug)]
struct RelayMsg {
    to_jid: String,
    message: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum UserListUpdateAction {
    Connected,
    Disconnected,
}

/// Inform users of updates to the roster
#[derive(Serialize, Deserialize, Debug)]
struct UserListUpdate {
    action: UserListUpdateAction,
    jid: String,
}

/// All the message types a ChatConnection can receive
#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum ClientMessages {
    Caps(HashSet<String>),
    Send(RelayMsg),
}

// ----- Server Implementation ----

/// Client data the server needs to keep track of
#[derive(Clone)]
struct ClientInfo {
    addr: Recipient<ProtoMessage>,
    caps: HashSet<String>,
}

impl ClientInfo {
    /// Construct an instance of the ClientInfo taking the address of
    /// the client received during connection time.
    fn new(addr: Recipient<ProtoMessage>) -> Self {
        ClientInfo {
            addr: addr,
            caps: HashSet::<String>::new(),
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
        Self {
            clients: HashMap::new(),
        }
    }

    fn broadcast(&mut self, msg: ProtoMessage) {
        for (key, client) in &self.clients {
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

/// Define the handler for Connect messages from ChatConnection
/// actors.
impl Handler<Connect> for ChatServer {
    type Result = ();

    /// Insert the newly connected client into the clients hash table.
    fn handle(&mut self, msg: Connect, _ctx: &mut Self::Context) {
        // First inform the currently connected clients about the
        // event
        let user_msg = UserListUpdate {
            action: UserListUpdateAction::Connected,
            jid: msg.jid.clone(),
        };
        let user_msg_str = serde_json::to_string(&user_msg).unwrap();
        self.broadcast(ProtoMessage(user_msg_str));
        // Finally update the clients list
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
        let user_msg = UserListUpdate {
            action: UserListUpdateAction::Disconnected,
            jid: msg.jid.clone(),
        };
        let user_msg_str = serde_json::to_string(&user_msg).unwrap();
        self.broadcast(ProtoMessage(user_msg_str));
    }
}

impl Handler<ListClients> for ChatServer {
    type Result = MessageResult<ListClients>;

    /// Return a list with the JIDs of all currently connected clients
    fn handle(&mut self, _: ListClients, _ctx: &mut Self::Context) -> Self::Result {
        let mut output: HashMap<String, HashSet<String>> = HashMap::new();
        for (key, client_info) in &self.clients {
            output.insert(key.clone(), client_info.caps.clone());
        }
        MessageResult(output)
    }
}

impl Handler<ConfigClient> for ChatServer {
    type Result = ();

    /// Save the capabilities received via ChatConnection
    fn handle(&mut self, msg: ConfigClient, _ctx: &mut Self::Context) {
        for cap in &msg.caps {
            self.clients
                .get_mut(&msg.jid)
                .unwrap()
                .caps
                .insert(cap.clone());
        }
    }
}

impl Handler<RelayMessage> for ChatServer {
    type Result = ();

    /// Relay received message to a given client
    fn handle(&mut self, msg: RelayMessage, _ctx: &mut Self::Context) {
        match self.clients.get(&msg.to_jid) {
            None => error!("Client {} not connected", msg.to_jid),
            Some(client) => client
                .addr
                .do_send(ProtoMessage(
                    json!({
                        "from_jid": msg.from_jid,
                        "message": msg.message
                    })
                    .to_string(),
                ))
                .unwrap(),
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
        return Self {
            jid: jid,
            server: server,
            heartbeat: Instant::now(),
        };
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

    /// Handle incoming messages from clients
    fn handle_message(&self, msg: String) {
        info!("Message from {}: {}", self.jid, msg);
        let deserialize: Result<ClientMessages, serde_json::Error> =
            serde_json::from_str(msg.as_str());
        match deserialize {
            Err(err) => error!("error parsing message from client: {:?}", err),
            Ok(as_json) => match as_json {
                ClientMessages::Caps(caps) => self.server.do_send(ConfigClient {
                    jid: self.jid.clone(),
                    caps: caps,
                }),
                ClientMessages::Send(send) => self.server.do_send(RelayMessage {
                    from_jid: self.jid.clone(),
                    to_jid: send.to_jid,
                    message: send.message,
                }),
            },
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
            _ => {
                ctx.stop();
            }
        }
    }
}

// ---- HTTP API ----

/// List currently connected clients
async fn http_api_clients(
    server: web::Data<Addr<ChatServer>>,
) -> Result<HttpResponse, actix_web::Error> {
    let clients = server.send(ListClients {}).await.unwrap();
    Ok(HttpResponse::Ok().json(clients))
}

// ---- HTTP Server Handling ----

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
    for email in &config.userauth.allowed_emails {
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
    if let Some(jid) = read_jid_from_request(&req) {
        ws::start(
            ChatConnection::new(jid, server.get_ref().clone()),
            &req,
            stream,
        )
    } else {
        Ok(HttpResponse::Unauthorized().finish())
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
fn decode_token_from_header(authorization: &str) -> String {
    (&authorization[7..]).to_string()
}

/// Decode & Check JWT token from HTTP header or QueryString
fn read_jid_from_request(req: &HttpRequest) -> Option<String> {
    let token = match get_auth_token(req) {
        Ok(token) => token,
        _ => decode_token_from_header(get_auth_header(req)?),
    };
    Some(token)
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
    let addr = format!("{}:{}", config.http.host, config.http.port);
    let address = ChatServer::new().start();
    let app = move || {
        App::new()
            .wrap(middleware::Logger::default())
            .data(config.clone())
            .data(address.clone())
            .route("/ws", web::get().to(ws))
            .route("/auth", web::post().to(auth))
            .route("/clients", web::get().to(http_api_clients))
    };
    HttpServer::new(app).bind(addr)?.run().await
}
