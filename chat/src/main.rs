use std::io;
use std::time::{Duration, Instant};
use std::collections::HashMap;

use actix_rt;
use actix::prelude::*;
use actix_web;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;

use serde_derive::Deserialize;

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
    port: u32,
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
#[derive(Message)]
#[rtype(result = "()")]
struct Message(String);

/// New client just connected
#[derive(Message)]
#[rtype(result = "()")]
struct Connect {
    jid: String,
    addr: Recipient<Message>,
}

/// Client disconnected
#[derive(Message)]
#[rtype(result = "()")]
struct Disconnect { jid: String }

// ----- Server Implementation ----

/// The server keeps track of all connected clients and all the open
/// rooms.  Clients are registered in the server when they hit the
/// websocket endpoint.
#[derive(Clone)]
struct ChatServer {
    clients: HashMap<String, Recipient<Message>>,
}

impl ChatServer {
    fn new(_config: Config) -> Self {
        Self {
            clients: HashMap::new(),
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
        self.clients.insert(msg.jid, msg.addr);
    }
}

impl Handler<Disconnect> for ChatServer {
    type Result = ();

    /// Remove client a connection from the clients hash table.
    fn handle(&mut self, msg: Disconnect, _ctx: &mut Self::Context) {
        self.clients.remove(&msg.jid);
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
        self.server.do_send(Connect {
            jid: self.jid.clone(),
            addr: ctx.address().recipient(),
        });
    }

    /// Get itself removed from the ChatServer actor
    fn deregister(&self) {
        self.server.do_send(Disconnect {
            jid: self.jid.clone(),
        });
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
}

/// Define the handler for messages from ChatServer.
impl Handler<Message> for ChatConnection {
    type Result = ();

    fn handle(&mut self, msg: Message, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

/// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChatConnection {
    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        println!("msg: {:?}", msg);

        match msg {
            Ok(ws::Message::Ping(msg))   => { self.heartbeat_update(); ctx.pong(&msg) },
            Ok(ws::Message::Pong(_))     => { self.heartbeat_update(); },
            Ok(ws::Message::Text(text))  => ctx.text(text),
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            Ok(ws::Message::Close(_))    => { self.deregister(); ctx.stop(); },
            _ => { self.deregister(); ctx.stop(); },
        }
    }
}

// ---- HTTP Handling ----

/// Represents the data that arrives from the authentication form.
#[derive(Debug, Deserialize)]
struct AuthForm {
    user: String,
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
        if *email == body.user {
            // TODO: Generate token for user
            // TODO: Send token via email
            return HttpResponse::Ok().finish();
        }
    }
    HttpResponse::Unauthorized().body("Unknown Address")
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
        println!("jid: {}", jid);
        ws::start(ChatConnection::new(jid, server.get_ref().clone()), &req, stream)
    } else {
        HttpResponse::Unauthorized().body("Unknown Address").await
    }
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

/// Decode & Check JWT token from HTTP header
fn read_jid_from_request(req: &HttpRequest) -> Option<String> {
    let header = get_auth_header(req)?;
    Some(decode_token_from_header(header))
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
    let args: Vec<String> = std::env::args().collect();
    let config: Config = load_config(&args)?;
    let addr = format!("{}:{}", config.http.host, config.http.port);
    let server = ChatServer::new(config).start();
    let app = move || App::new()
        .data(server.clone())
        .route("/auth", web::post().to(auth))
        .route("/ws/", web::get().to(ws));
    HttpServer::new(app)
        .bind(addr)?
        .run()
        .await
}
