#[macro_use]
extern crate log;
#[macro_use]
extern crate anyhow;
extern crate tokio;

use anyhow::Error;
// use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

use actix::io::SinkWrite;
use actix::prelude::*;
use actix_codec::Framed;
use awc::{
    error::WsProtocolError,
    ws::{Codec, Frame, Message as WsMessage},
    BoxedSocket,
};
use bytes::Bytes;
use futures::stream::{SplitSink, StreamExt};
use tokio::sync::oneshot;

use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};

// use gst::gst_element_error;
use gst::prelude::*;
use gst_webrtc::{WebRTCSDPType, WebRTCSessionDescription};

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
fn gst_create_elements() -> Result<(gst::Pipeline, gst::Element), Error> {
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

fn gst_on_answer_created(
    elements: WeakAppClients,
    reply: Result<&gst::StructureRef, gst::PromiseError>,
) -> Result<String, Error> {
    let reply = match reply {
        Ok(reply) => reply,
        Err(err) => {
            bail!("Answer creation future got no reponse: {:?}", err);
        }
    };

    let elements = elements.upgrade().expect("Can't get reference");

    let answer = reply
        .get_value("answer")?
        .get::<WebRTCSessionDescription>()?
        .expect("Invalid argument");
    elements
        .webrtcbin
        .emit("set-local-description", &[&answer, &None::<gst::Promise>])?;

    Ok(serde_json::to_string(&ClientMessage::Sdp {
        type_: "answer".to_string(),
        sdp: answer.get_sdp().as_text().unwrap(),
    })?)
}

fn gst_on_offer_created(
    elements: WeakAppClients,
    reply: Result<&gst::StructureRef, gst::PromiseError>,
) -> Result<String, Error> {
    let reply = match reply {
        Ok(reply) => reply,
        Err(err) => {
            bail!("Answer creation future got no reponse: {:?}", err);
        }
    };

    let elements = elements.upgrade().expect("Can't get reference");

    let offer = reply
        .get_value("offer")
        .unwrap()
        .get::<WebRTCSessionDescription>()
        .expect("Invalid argument")
        .unwrap();
    elements
        .webrtcbin
        .emit("set-local-description", &[&offer, &None::<gst::Promise>])
        .unwrap();

    debug!(
        "sending SDP offer to peer: {}",
        offer.get_sdp().as_text().unwrap()
    );

    Ok(serde_json::to_string(&ClientMessage::Sdp {
        type_: "offer".to_string(),
        sdp: offer.get_sdp().as_text().unwrap(),
    })?)
}

async fn gst_on_negotiation_needed(elements: WeakAppClients) -> Result<String, Error> {
    let (sender, receiver) = oneshot::channel::<Result<String, Error>>();
    let strong = elements.upgrade().expect("Can't get reference");
    let promise = gst::Promise::new_with_change_func(move |reply| {
        match gst_on_offer_created(elements, reply) {
            Ok(answer) => sender.send(Ok(answer)).unwrap(),
            Err(error) => error!("Couldn't create answer: {}", error),
        }
    });
    strong
        .webrtcbin
        .emit("create-offer", &[&None::<gst::Structure>, &promise])?;
    receiver.await?
}

async fn gst_handle_sdp(
    elements: WeakAppClients,
    type_: &str,
    sdp: &str,
) -> Result<Option<String>, Error> {
    info!("Handle SDP {}", type_);
    let sdp = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes())
        .map_err(|_| anyhow!("Failed to parse SDP"))?;
    let strong = elements.upgrade().expect("Can't get reference");

    if type_ == "answer" {
        let answer = WebRTCSessionDescription::new(WebRTCSDPType::Answer, sdp);
        strong
            .webrtcbin
            .emit("set-remote-description", &[&answer, &None::<gst::Promise>])?;
        Ok(None)
    } else if type_ == "offer" {
        let (sender, receiver) = oneshot::channel::<Result<String, Error>>();

        strong.pipeline.call_async(move |_pipeline| {
            let offer = WebRTCSessionDescription::new(WebRTCSDPType::Offer, sdp);
            let strong = elements.upgrade().expect("Can't get reference");
            strong
                .webrtcbin
                .emit("set-remote-description", &[&offer, &None::<gst::Promise>])
                .unwrap();
            // Promise that manages the reply tothe `create-element`
            // event on the webrtcbin element right below this
            // declaration.
            let promise = gst::Promise::new_with_change_func(move |reply| {
                match gst_on_answer_created(elements, reply) {
                    Ok(answer) => sender.send(Ok(answer)).unwrap(),
                    Err(error) => error!("Couldn't create answer: {}", error),
                }
            });
            strong
                .webrtcbin
                .emit("create-answer", &[&None::<gst::Structure>, &promise])
                .unwrap();
        });
        Ok(Some(receiver.await??))
    } else {
        bail!("Don't know what I'm doing")
    }
}

fn gst_handle_ice(
    elements: WeakAppClients,
    candidate: String,
    sdp_mline_index: u32,
) -> Result<Option<String>, Error> {
    info!("Handle ICE message: {}", candidate);
    let elements = elements.upgrade().expect("Can't get reference");
    elements
        .webrtcbin
        .emit("add-ice-candidate", &[&sdp_mline_index, &candidate])?;
    Ok(None)
}

async fn gst_handle_message(
    elements: WeakAppClients,
    envelope: GstAppCmd,
) -> Result<Option<String>, Error> {
    match envelope.message {
        ClientMessage::Sdp { type_, sdp } => {
            gst_handle_sdp(elements, type_.as_str(), sdp.as_str()).await
        }
        ClientMessage::Ice {
            candidate,
            sdp_mline_index,
        } => gst_handle_ice(elements, candidate, sdp_mline_index),
    }
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

// ---- Actors ----

#[derive(Message)]
#[rtype(result = "Result<Option<String>, Error>")]
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
type WeakAppClients = Weak<GstElements>;

struct GstApp {
    gst_elements: Option<GstAppClients>,
    chat_client: Option<Recipient<ClientCommand>>,
}

impl GstApp {
    fn add_client(&mut self, client_id: &String) -> Result<(), Error> {
        let (pipeline, webrtcbin) = gst_create_elements()?;

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

impl Handler<GstAppCmd> for GstApp {
    type Result = ResponseActFuture<Self, Result<Option<String>, Error>>;

    fn handle(&mut self, envelope: GstAppCmd, _ctx: &mut Context<Self>) -> Self::Result {
        let sender = envelope.from_jid.clone();

        match self.add_client(&sender) {
            Ok(_) => info!("Client registered"),
            Err(error) => return Box::new(async move { bail!(error) }.into_actor(self)),
        }

        if let Some(elements) = &self.gst_elements {
            let weak = Arc::downgrade(&elements);

            Box::new(gst_handle_message(weak, envelope).into_actor(self).map(
                move |res: Result<Option<String>, Error>, _act, _ctx| match res.unwrap() {
                    None => Ok(None),
                    Some(data) => {
                        println!("TO BE SENT to {}: {}", sender, data);
                        Ok(None)
                    }
                },
            ))
        } else {
            Box::new(async { bail!("GST Elements not set") }.into_actor(self))
        }
    }
}

struct ChatClient {
    framed: SinkWrite<WsMessage, SplitSink<Framed<BoxedSocket, Codec>, WsMessage>>,
    gst_application: Recipient<GstAppCmd>,
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
