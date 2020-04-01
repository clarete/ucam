#[macro_use]
extern crate log;
#[macro_use]
extern crate anyhow;

use anyhow::Error;
// use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

use actix::io::SinkWrite;
use actix::prelude::*;
use actix::{
    Actor,
    ActorContext,
    Arbiter,
    AsyncContext,
    Context,
    Handler,
    Message,
    StreamHandler,
    // Supervised,
    System,
};

use actix_codec::Framed;
use awc::{
    error::WsProtocolError,
    ws::{Codec, Frame, Message as WsMessage},
    BoxedSocket,
};
use bytes::Bytes;
use futures::stream::{SplitSink, StreamExt};
//use futures::channel::oneshot;

extern crate tokio;

use tokio::sync::oneshot;

use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};

use gst::gst_element_error;
use gst::prelude::*;

const STUN_SERVER: &str = "stun://stun.l.google.com:19302";
const TURN_SERVER: &str = "turn://foo:bar@webrtc.nirbheek.in:3478";
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

// ---- GST Related functions ----

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
    webrtcbin.set_property_from_str("turn-server", TURN_SERVER);
    webrtcbin.set_property_from_str("bundle-policy", "max-bundle");
    // Return the application with newly created gst stuff and
    // client picked up from parameters
    Ok((pipeline, webrtcbin))
}

// ---- Websocket Related Functions ----

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
    ::std::env::set_var(
        "RUST_LOG",
        "capture=debug,awc=debug,actix=debug,actix_web=debug",
    );
    env_logger::init();
    // Initialize Gstreamer stuff
    gst::init()?;
    check_plugins()?;
    // Build instances of websocket client and then gstreamer
    // application
    let wsclient = get_ws_client().await?;

    Arbiter::spawn(async move {
        let (sink, stream) = wsclient.split();
        let gst_application = GstApp::default().start();
        let addr = ChatClient::create(|ctx| {
            ChatClient::add_stream(stream, ctx);
            ChatClient {
                framed: SinkWrite::new(sink, ctx),
                gst_application: gst_application.recipient(),
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

#[derive(Message)]
#[rtype(result = "Result<String, Error>")]
struct GstAppCmd {
    from_jid: String,
    message: ClientMessage,
}

struct GstElements {
    pipeline: gst::Pipeline,
    webrtcbin: gst::Element,
}

// type GstAppClients = HashMap<String, Arc<GstElements>>;
type GstAppClients = Arc<GstElements>;

struct GstApp {
    gst_elements: Option<GstAppClients>,
    chat_client: Option<Recipient<ClientCommand>>,
}

type WeakAppClients = Weak<GstElements>;

async fn gst_handle_sdp(
    elements: WeakAppClients,
    from_jid: &str,
    type_: &str,
    sdp: &str,
) -> Result<String, Error> {
    info!("Handle SDP {}", type_);

    if type_ == "offer" {
        let ret = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes())
            .map_err(|_| anyhow!("Failed to parse SDP offer"))?;

        debug!("{}", ret);

        let (answer_sent_tx, answer_sent_rx) = oneshot::channel::<String>();

        let strong = elements.upgrade().expect("Can't get reference");

        strong.pipeline.call_async(move |pipeline| {
            println!("00002");

            let offer = gst_webrtc::WebRTCSessionDescription::new(
                gst_webrtc::WebRTCSDPType::Offer,
                ret,
            );

            let strong = elements.upgrade().expect("Can't get reference");

            println!("00003");

            strong
                .webrtcbin
                .emit("set-remote-description", &[&offer, &None::<gst::Promise>])
                .unwrap();

            // Promise that manages the reply tothe `create-element`
            // event on the webrtcbin element right below this
            // declaration.
            let promise = gst::Promise::new_with_change_func(move |reply| {

                println!("00005");

                let strong = elements.upgrade().expect("Can't get reference");

                let reply = reply.unwrap();
                let answer = reply
                    .get_value("answer")
                    .unwrap()
                    .get::<gst_webrtc::WebRTCSessionDescription>()
                    .expect("Invalid argument")
                    .unwrap();
                strong.webrtcbin
                    .emit("set-local-description", &[&answer, &None::<gst::Promise>])
                    .unwrap();

                println!("SEEEEND");
                answer_sent_tx.send("Stuff".to_string()).unwrap();
            });

            strong
                .webrtcbin
                .emit("create-answer", &[&None::<gst::Structure>, &promise])
                .unwrap();
        });
        println!("00001");
        let res: String = answer_sent_rx.await.unwrap();
        Ok(res)
    } else {
        bail!("Don't know what I'm doing")
    }
}

async fn gst_handle_message(
    elements: WeakAppClients,
    envelope: GstAppCmd,
) -> Result<String, Error> {
    match envelope.message {
        ClientMessage::Sdp { type_, sdp } => Ok(gst_handle_sdp(
            elements,
            envelope.from_jid.as_str(),
            type_.as_str(),
            sdp.as_str(),
        )
        .await?),
        ClientMessage::Ice {
            candidate,
            sdp_mline_index,
        } => {
            Ok("ICE FOO".to_string()) // self.handle_ice(envelope.from_jid.as_str(), candidate.as_str(), sdp_mline_index);
        }
    }
}

impl GstApp {
    fn add_client(&mut self, client_id: &String) -> Result<(), Error> {
        let (pipeline, webrtcbin) = get_gst_elements()?;

        pipeline.call_async(|pipeline| {
            pipeline
                .set_state(gst::State::Playing)
                .expect("Couldn't set pipeline to Playing");
        });

        self.gst_elements = Some(Arc::new(GstElements {
            pipeline: pipeline,
            webrtcbin: webrtcbin,
        }));

        // self.gst_elements.insert(client_id, Arc::new(Mutex::new(GstElements {
        //     pipeline: pipeline,
        //     webrtcbin: webrtcbin,
        // })));

        info!("Set pipeline for {} to PLAYING state", client_id);

        Ok(())
    }

    fn set_chat_client(&mut self, client: Recipient<ClientCommand>) {
        self.chat_client = Some(client);
    }

    fn send_to_client(&self, msg: Value) -> Result<(), SendError<ClientCommand>> {
        if let Some(client) = &self.chat_client {
            client.do_send(ClientCommand(msg.to_string()))
        } else {
            Err(SendError::<ClientCommand>::Full(ClientCommand(
                "Client not conneced".to_string(),
            )))
        }
    }
}

impl Default for GstApp {
    fn default() -> Self {
        GstApp {
            chat_client: None,
            gst_elements: None,
        }
    }
}


impl Actor for GstApp {
    type Context = Context<Self>;
}

struct ChatClient {
    framed: SinkWrite<WsMessage, SplitSink<Framed<BoxedSocket, Codec>, WsMessage>>,
    gst_application: Recipient<GstAppCmd>,
}

impl Handler<GstAppCmd> for GstApp {
    type Result = ResponseActFuture<Self, Result<String, Error>>;

    fn handle(&mut self, envelope: GstAppCmd, _ctx: &mut Context<Self>) -> Self::Result {
        match self.add_client(&envelope.from_jid) {
            Ok(_) => info!("Client registered"),
            Err(error) => return Box::new(async move { bail!(error) }.into_actor(self)),
        }

        if let Some(elements) = &self.gst_elements {
            let weak = Arc::downgrade(&elements);

            Box::new(gst_handle_message(weak, envelope).into_actor(self).map(
                |res: Result<String, Error>, _act, _ctx| {
                    let stuff = res.unwrap();
                    println!("{}", stuff);
                    Ok(stuff)
                },
            ))
        } else {
            Box::new(async { bail!("GST Elements not set") }.into_actor(self))
        }
    }
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
        match msg {
            Ok(Frame::Ping(_)) => Ok(()),
            Ok(Frame::Pong(_)) => Ok(()),
            Ok(Frame::Binary(_)) => {
                debug!("Websocket binary?!");
                Ok(())
            }
            Ok(Frame::Close(_)) => {
                debug!("Websocket close");
                Ok(())
            }
            Ok(Frame::Text(txt)) => {
                let utf8 = std::str::from_utf8(&txt)?;
                let envelope: Envelope = serde_json::from_str(utf8)?;
                let message: ClientMessage = serde_json::from_str(envelope.message.as_str())?;
                match message {
                    ClientMessage::Sdp { type_, sdp } => {
                        self.gst_application.do_send(GstAppCmd {
                            from_jid: envelope.from_jid.clone(),
                            message: ClientMessage::Sdp {
                                type_: type_,
                                sdp: sdp,
                            },
                        })?;
                    }
                    ClientMessage::Ice {
                        sdp_mline_index,
                        candidate,
                    } => self.gst_application.do_send(GstAppCmd {
                        from_jid: envelope.from_jid.clone(),
                        message: ClientMessage::Ice {
                            sdp_mline_index,
                            candidate,
                        },
                    })?,
                }
                Ok(())
            }
            _ => bail!("Can't parse websocket message"),
        }
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
        info!("Websocket actor disconnected, bringing it all down.");
        // Stop application on disconnect
        System::current().stop();
    }
}

/// Handle outgoing websocket messages
impl Handler<ClientCommand> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: ClientCommand, _ctx: &mut Context<Self>) {
        self.framed.write(WsMessage::Text(msg.0)).unwrap();
    }
}

/// Receive incoming websocket messages from server
impl StreamHandler<Result<Frame, WsProtocolError>> for ChatClient {
    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, _: &mut Context<Self>) {
        if let Err(e) = self.handle_websocket_message(&msg) {
            error!("Can't handle websocket message: {}", e)
        }
    }

    fn started(&mut self, _ctx: &mut Context<Self>) {
        info!("Websocket stream handler connected");
    }

    fn finished(&mut self, ctx: &mut Context<Self>) {
        info!("Websocket server disconnected");
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
