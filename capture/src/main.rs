#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;

use failure::Error;

use std::time::Duration;
// use std::{io, thread};

use actix::io::SinkWrite;
use actix::*;
use actix_codec::Framed;
use awc::{
    error::WsProtocolError,
    ws::{Codec, Frame, Message},
    BoxedSocket, Client,
};
use bytes::Bytes;
use futures::stream::{SplitSink, StreamExt};

use serde_derive::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, Debug)]
struct Envelope {
    from_jid: String,
    message: String,
}

// JSON messages exchanged with other clients
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ClientMessage {
    Ice {
        candidate: String,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: u32,
    },
    Sdp {
        #[serde(rename = "type")]
        type_: String,
        sdp: String,
    },
}

const JSWS_SERVER: &str = "ws://127.0.0.1:7070/ws";

fn main() {
    ::std::env::set_var("RUST_LOG", "awc=debug,actix=debug,actix_web=debug");
    env_logger::init();

    let sys = System::new("websocket-client");

    Arbiter::spawn(async {
        let error_handler = |e| {
            println!("Error: {}", e);
        };

        let (_, framed) = Client::new()
            .ws(JSWS_SERVER)
            .header("Authorization", "Bearer cam001@studio.loc")
            .connect()
            .await
            .map_err(error_handler)
            .unwrap();

        let (sink, stream) = framed.split();
        let addr = ChatClient::create(|ctx| {
            ChatClient::add_stream(stream, ctx);
            ChatClient(SinkWrite::new(sink, ctx))
        });

        // Send the initial message with the capabilities of this
        // client.
        addr.do_send(ClientCommand(
            json!({
                "caps": [
                    "s:video",
                    "s:audio",
                    "r:audio",
                ]
            })
            .to_string(),
        ));

        println!("Not really sure what I'm doing");

        // Wait indefinitely for new messages from either the
        // websocket connection or the gstreamer bus
    });
    sys.run().unwrap();
}

struct ChatClient(SinkWrite<Message, SplitSink<Framed<BoxedSocket, Codec>, Message>>);

impl ChatClient {
    fn handle_websocket_message(&self, msg: &Result<Frame, WsProtocolError>) -> Result<(), Error> {
        if let Ok(Frame::Text(txt)) = msg {
            let utf8 = std::str::from_utf8(&txt)?;
            let envelope: Envelope = serde_json::from_str(utf8)?;
            let message: ClientMessage = serde_json::from_str(envelope.message.as_str())?;
            match message {
                ClientMessage::Sdp { type_, sdp } => self.sdp(&type_, &sdp),
                ClientMessage::Ice {
                    sdp_mline_index,
                    candidate,
                } => self.ice(sdp_mline_index, &candidate),
            }
            return Ok(());
        }
        bail!("Not Text")
    }

    fn sdp(&self, type_: &str, sdp: &str) {
        println!("Received SDP: {} {}", type_, sdp);
    }

    fn ice(&self, sdp_mline_index: u32, candidate: &str) {
        println!("Received ICE: {} {}", sdp_mline_index, candidate);
    }
}

#[derive(Message)]
#[rtype(result = "()")]
struct ClientCommand(String);

impl Actor for ChatClient {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        // start heartbeats otherwise server will disconnect after 10 seconds
        self.hb(ctx)
    }

    fn stopped(&mut self, _: &mut Context<Self>) {
        println!("Disconnected");

        // Stop application on disconnect
        System::current().stop();
    }
}

impl ChatClient {
    fn hb(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(1, 0), |act, ctx| {
            act.0.write(Message::Ping(Bytes::from_static(b""))).unwrap();
            act.hb(ctx);

            // client should also check for a timeout here, similar to the
            // server code
        });
    }
}

/// Handle sending messages
impl Handler<ClientCommand> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: ClientCommand, _ctx: &mut Context<Self>) {
        self.0.write(Message::Text(msg.0)).unwrap();
    }
}

/// Handle server websocket messages
impl StreamHandler<Result<Frame, WsProtocolError>> for ChatClient {
    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, _: &mut Context<Self>) {
        match self.handle_websocket_message(&msg) {
            Ok(_) => debug!("Successfuly parsed websocket message"),
            Err(e) => error!("Can't handle websocket message: {}", e),
        }
    }

    fn started(&mut self, _ctx: &mut Context<Self>) {
        println!("Connected");
    }

    fn finished(&mut self, ctx: &mut Context<Self>) {
        println!("Server disconnected");
        ctx.stop()
    }
}

impl actix::io::WriteHandler<WsProtocolError> for ChatClient {}
