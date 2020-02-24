use std::io;
use std::time::{Duration, Instant};

use actix_rt;
use actix::prelude::*;
use actix_web;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;

use serde_derive::Deserialize;


/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

struct ChatSocket {
    heartbeat: Instant,
}

impl ChatSocket {
    fn new() -> Self {
        return Self { heartbeat: Instant::now() };
    }

    fn alive(&mut self) {
        self.heartbeat = Instant::now();
    }
}

/// Define http actor for the ChatSocket struct
impl Actor for ChatSocket {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start.  We start the heartbeat
    /// process here.
    fn started(&mut self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                println!("Websocket Client heartbeat failed, disconnecting!");
                ctx.stop();
            } else {
                ctx.ping(b"");
            }
        });
    }
}

/// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChatSocket {
    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        println!("msg: {:?}", msg);

        match msg {
            Ok(ws::Message::Ping(msg))   => { self.alive(); ctx.pong(&msg) },
            Ok(ws::Message::Pong(_))     => { self.alive(); },
            Ok(ws::Message::Text(text))  => ctx.text(text),
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            Ok(ws::Message::Close(_))    => { ctx.stop(); },
            _ => ctx.stop(),
        }
    }
}

async fn auth() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

async fn ws(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, actix_web::Error> {
    ws::start(ChatSocket::new(), &req, stream)
}

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
struct Config {
    logging: Option<ConfigLogging>,
    userauth: Option<ConfigUserAuth>,
    locations: std::collections::HashMap<String, ConfigLocation>,
}

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

    HttpServer::new(move || {
        App::new()
            .data(config.clone())
            .route("/auth", web::get().to(auth))
            .route("/ws/", web::get().to(ws))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
