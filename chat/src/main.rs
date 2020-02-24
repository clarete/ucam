use std::time::{Duration, Instant};

use actix_rt;
use actix::prelude::*;
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;

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

async fn ws(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    ws::start(ChatSocket::new(), &req, stream)
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info");
    HttpServer::new(|| {
        App::new()
            .route("/auth", web::get().to(auth))
            .route("/ws/", web::get().to(ws))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
