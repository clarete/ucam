#[macro_use]
extern crate log;
#[macro_use]
extern crate anyhow;

use anyhow::Error;

use std::time::Duration;
// use std::{io, thread};

use actix::io::SinkWrite;
use actix::prelude::Recipient;
use actix::{
    Actor, ActorContext, Arbiter, AsyncContext, Context, Handler, Message, StreamHandler, System,
};

use actix_codec::Framed;
use awc::{
    error::{WsProtocolError},
    ws::{Codec, Frame, Message as WsMessage},
    BoxedSocket,
};
use bytes::Bytes;
use futures::stream::{SplitSink, StreamExt};

use serde_derive::{Deserialize, Serialize};
use serde_json::json;

use gst::gst_element_error;
use gst::prelude::*;

const STUN_SERVER: &str = "stun://stun.l.google.com:19302";
const JSWS_SERVER: &str = "ws://127.0.0.1:7070/ws";

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

/// Create the Gstreamer pipeline & grab the webrtcbin element
fn get_gst_elements() -> Result<(gst::Pipeline, gst::Element), Error> {
    let pipeline = gst::parse_launch(
        "videotestsrc pattern=ball is-live=true ! vp8enc deadline=1 ! rtpvp8pay pt=96 ! webrtcbin. \
         audiotestsrc is-live=true ! opusenc ! rtpopuspay pt=97 ! webrtcbin. \
         webrtcbin name=webrtcbin"
    )?;
    // Downcast from gst::Element to gst::Pipeline
    let pipeline = pipeline
        .downcast::<gst::Pipeline>()
        .expect("not a pipeline");
    // Get access to the webrtcbin by name
    let webrtcbin = pipeline
        .get_by_name("webrtcbin")
        .expect("can't find webrtcbin");
    // Set webrtcbin properties
    webrtcbin.set_property_from_str("stun-server", STUN_SERVER);
    webrtcbin.set_property_from_str("bundle-policy", "max-bundle");
    // Return the application with newly created gst stuff and
    // client picked up from parameters
    Ok((pipeline, webrtcbin))
}

/// Create the HTTP client and connect it to the websocket server.
/// Then return the framed response
async fn get_ws_client() -> Result<Framed<BoxedSocket, Codec>, Error> {
    let client = awc::Client::new()
        .ws(JSWS_SERVER)
        .header("Authorization", "Bearer cam001@studio.loc")
        .connect()
        .await;
    match client {
        Ok((_, framed)) => Ok(framed),
        Err(e) => bail!("Can't connect to server: {}", e),
    }
}

#[actix_rt::main]
async fn main() -> Result<(), Error> {
    // Initialize the logging stuff
    ::std::env::set_var("RUST_LOG", "awc=debug,actix=debug,actix_web=debug");
    env_logger::init();
    // Initialize Gstreamer stuff
    gst::init()?;
    check_plugins()?;
    // Build instances of websocket client and then gstreamer stuff
    let wsclient = get_ws_client().await?;
    let (pipeline, webrtcbin) = get_gst_elements()?;

    Arbiter::spawn(async move {
        let (sink, stream) = wsclient.split();
        let addr = ChatClient::create(|ctx| {
            ChatClient::add_stream(stream, ctx);
            ChatClient {
                framed: SinkWrite::new(sink, ctx),
            }
        });

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
    });

    actix_rt::signal::ctrl_c()
        .await
        .expect("failed to listen for event");

    Ok(())
}

struct ChatClient {
    framed: SinkWrite<WsMessage, SplitSink<Framed<BoxedSocket, Codec>, WsMessage>>,
}

impl ChatClient {
    fn hb(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(1, 0), |act, ctx| {
            act.framed
                .write(WsMessage::Ping(Bytes::from_static(b"")))
                .unwrap();
            act.hb(ctx);

            // client should also check for a timeout here, similar to the
            // server code
        });
    }

    fn handle_websocket_message(&self, msg: &Result<Frame, WsProtocolError>) -> Result<(), Error> {
        if let Ok(Frame::Text(txt)) = msg {
            let utf8 = std::str::from_utf8(&txt)?;
            let envelope: Envelope = serde_json::from_str(utf8)?;
            let message: ClientMessage = serde_json::from_str(envelope.message.as_str())?;
            match message {
                ClientMessage::Sdp { type_, sdp } => self.sdp(&type_, &sdp)?,
                ClientMessage::Ice {
                    sdp_mline_index,
                    candidate,
                } => self.ice(sdp_mline_index, &candidate),
            }
            return Ok(());
        }
        bail!("Not Text")
    }

    fn sdp(&self, type_: &str, sdp: &str) -> Result<(), Error> {
        if type_ == "offer" {
            println!("Received SDP offer:\n{}\n", sdp);

            if let Ok(ret) = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) {}

            // .map_err(|_| bail!("Failed to parse SDP offer"));
        }
        Ok(())
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

/// Handle sending messages
impl Handler<ClientCommand> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: ClientCommand, _ctx: &mut Context<Self>) {
        self.framed.write(WsMessage::Text(msg.0)).unwrap();
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

fn check_plugins() -> Result<(), Error> {
    let needed = [
        "videotestsrc",
        "audiotestsrc",
        "videoconvert",
        "audioconvert",
        "autodetect",
        "opus",
        "vpx",
        "webrtc",
        "nice",
        "dtls",
        "srtp",
        "rtpmanager",
        "rtp",
        "playback",
        "videoscale",
        "audioresample",
    ];

    let registry = gst::Registry::get();
    let missing = needed
        .iter()
        .filter(|n| registry.find_plugin(n).is_none())
        .cloned()
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        bail!("Missing plugins: {:?}", missing);
    } else {
        Ok(())
    }
}
