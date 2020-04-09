use std::{
    sync::mpsc::{channel, Sender},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[macro_use]
extern crate log;

use bytes::Bytes;
#[macro_use]
extern crate failure;
use failure::{Error, Fail};
use futures::stream::{SplitSink, StreamExt};

//use glib;
use gst::{self, prelude::*};
use lazy_static::lazy_static;
use serde_derive::{Deserialize, Serialize};
use serde_json::json;

use actix::io::SinkWrite;
use actix::*;
use actix_codec::Framed;
use awc::{
    error::WsProtocolError,
    ws::{Codec, Frame, Message},
    BoxedSocket, Client,
};

// const STUN_SERVER: &str = "stun://stun.l.google.com:19302";
// const TURN_SERVER: &str = "turn://foo:bar@webrtc.nirbheek.in:3478";
const JSWS_SERVER: &str = "ws://127.0.0.1:7070/ws";

lazy_static! {
    static ref RTP_CAPS_VP8: gst::Caps = {
        gst::Caps::new_simple(
            "application/x-rtp",
            &[
                ("media", &"video"),
                ("encoding-name", &"VP8"),
                ("payload", &(96i32)),
            ],
        )
    };
}

#[derive(Debug, Fail)]
#[fail(display = "Missing elements {:?}", _0)]
struct MissingElements(Vec<&'static str>);

#[derive(Debug, Fail)]
#[fail(display = "Failed to create answer")]
struct NullAnswer;

#[derive(Debug, Fail)]
#[fail(display = "Failed to get bus")]
struct NullBus;

#[derive(Debug, Fail)]
#[fail(display = "Failed to create element \"{}\"", _0)]
struct NullElement(&'static str);

#[derive(Debug, Fail)]
#[fail(display = "Failed to create offer")]
struct NullOffer;

#[derive(Debug, Fail)]
#[fail(display = "Failed to create pad \"{}\"", _0)]
struct NullPad(&'static str);

#[derive(Debug, Fail)]
#[fail(display = "Failed to create reply")]
struct NullReply;

#[derive(Debug, Fail)]
#[fail(display = "Failed to create session description")]
struct NullSessionDescription;

#[derive(Debug, Deserialize, Serialize)]
struct Sdp {
    #[serde(rename = "type")]
    type_: String,
    #[serde(rename = "sdp")]
    data: String,
}

macro_rules! pipeline {
    ($name:expr) => {
        $name
            .0
            .lock()
            .unwrap()
            .downcast_ref::<gst::Pipeline>()
            .unwrap()
    };
}

fn on_answer_created(
    context: StrongContext,
    peer_id: String,
    reply: Result<&gst::StructureRef, gst::PromiseError>,
    answer_sender: Sender<gst_webrtc::WebRTCSessionDescription>,
) -> Result<(), Error> {
    let answer = reply
        .unwrap()
        .get_value("answer")?
        .get::<gst_webrtc::WebRTCSessionDescription>()?
        .ok_or(NullSessionDescription)?;
    let webrtcbin = pipeline!(context)
        .get_by_name(peer_id.as_str())
        .ok_or(NullElement("webrtcbin"))?;
    webrtcbin.emit("set-local-description", &[&answer, &None::<gst::Promise>])?;
    answer_sender.send(answer)?;

    Ok(())
}

fn add_peer_to_pipeline(context: StrongContext, peer_id: &str) -> Result<(), Error> {
    let queue = gst::ElementFactory::make("queue", None)?;
    let webrtcbin = gst::ElementFactory::make("webrtcbin", Some(peer_id))?;
    pipeline!(context).add_many(&[&queue, &webrtcbin])?;

    let queue_src = queue.get_static_pad("src").ok_or(NullPad("queue_src"))?;
    let webrtc_sink = webrtcbin
        .get_request_pad("sink_%u")
        .ok_or(NullPad("webrtc_sink"))?;
    queue_src.link(&webrtc_sink)?;

    let tee = pipeline!(context)
        .get_by_name("videotee")
        .ok_or(NullElement("videotee"))?;
    let tee_src = tee.get_request_pad("src_%u").ok_or(NullPad("tee_src"))?;
    let queue_sink = queue.get_static_pad("sink").ok_or(NullPad("queue_sink"))?;
    tee_src.link(&queue_sink)?;

    queue.sync_state_with_parent()?;
    webrtcbin.sync_state_with_parent()?;

    Ok(())
}

#[derive(Clone)]
struct StrongContext(Arc<Mutex<dyn std::any::Any + 'static>>);

unsafe impl Send for StrongContext {}

fn handle_ice(
    context: StrongContext,
    peer_id: &String,
    candidate: String,
    sdp_mline_index: u32,
) -> Result<Option<String>, Error> {
    info!("Handle ICE from {}", peer_id);
    let webrtcbin = pipeline!(context)
        .get_by_name(peer_id)
        .ok_or(NullElement("webrtcbin"))?;
    webrtcbin.emit("add-ice-candidate", &[&sdp_mline_index, &candidate])?;
    Ok(None)
}

fn handle_sdp(
    pipeline: StrongContext,
    peer_id: &String,
    type_: &str,
    sdp: &str,
) -> Result<Option<String>, Error> {
    info!("Handle SDP {} from {}", type_, peer_id);

    add_peer_to_pipeline(pipeline.clone(), peer_id)?;

    if type_ == "offer" {
        Ok(Some(handle_sdp_offer(pipeline, sdp, peer_id)?))
    } else {
        println!(r#"Sdp type is not "offer""#);
        Ok(None)
    }
}

fn handle_sdp_offer(context: StrongContext, sdp: &str, peer_id: &String) -> Result<String, Error> {
    let msg = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()).unwrap();
    let offer = gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Offer, msg);
    let webrtcbin = pipeline!(context).get_by_name(peer_id).unwrap();
    webrtcbin.emit("set-remote-description", &[&offer, &None::<gst::Promise>])?;

    let (answer_sender, answer_receiver) = channel();
    let peer_id = peer_id.clone();
    let promise = &gst::Promise::new_with_change_func(move |reply| {
        on_answer_created(context, peer_id, reply, answer_sender).unwrap();
    });
    webrtcbin.emit("create-answer", &[&None::<gst::Structure>, &promise])?;

    let answer: gst_webrtc::WebRTCSessionDescription = answer_receiver.recv()?;
    let msg = serde_json::to_string(&Sdp {
        type_: "answer".to_string(),
        data: answer.get_sdp().as_text()?,
    })?;

    Ok(msg)
}

fn check_plugins() -> Result<(), Error> {
    let needed = [
        "opus",
        "vpx",
        "nice",
        "webrtc",
        "dtls",
        "srtp",
        "rtpmanager",
        "videotestsrc",
        "audiotestsrc",
    ];

    let registry = gst::Registry::get();
    let missing = needed
        .iter()
        .filter(|n| registry.find_plugin(n).is_none())
        .map(|n| *n)
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        Err(MissingElements(missing))?
    } else {
        Ok(())
    }
}

fn init_pipeline(pipeline: &gst::Pipeline) -> Result<(), Error> {
    let videotestsrc = gst::ElementFactory::make("videotestsrc", None)?;
    videotestsrc.set_property_from_str("pattern", "ball");
    videotestsrc.set_property("is-live", &true)?;

    let videoconvert = gst::ElementFactory::make("videoconvert", None)?;
    let queue = gst::ElementFactory::make("queue", None)?;

    let vp8enc = gst::ElementFactory::make("vp8enc", None)?;
    vp8enc.set_property("deadline", &1i64)?;

    let rtpvp8pay = gst::ElementFactory::make("rtpvp8pay", None)?;
    let queue2 = gst::ElementFactory::make("queue", None)?;
    let tee = gst::ElementFactory::make("tee", Some("videotee"))?;
    let queue3 = gst::ElementFactory::make("queue", None)?;
    let sink = gst::ElementFactory::make("fakesink", None)?;

    pipeline.add_many(&[
        &videotestsrc,
        &videoconvert,
        &queue,
        &vp8enc,
        &rtpvp8pay,
        &queue2,
        &tee,
        &queue3,
        &sink,
    ])?;

    gst::Element::link_many(&[
        &videotestsrc,
        &videoconvert,
        &queue,
        &vp8enc,
        &rtpvp8pay,
        &queue2,
    ])?;

    queue2.link_filtered(&tee, Some(&*RTP_CAPS_VP8))?;

    gst::Element::link_many(&[&tee, &queue3, &sink])?;

    pipeline.set_state(gst::State::Playing)?;

    Ok(())
}

/// Create the HTTP client and connect it to the websocket server.
/// Then return the framed response
async fn get_ws_client() -> Result<Framed<BoxedSocket, Codec>, Error> {
    let client = Client::new()
        .ws(JSWS_SERVER)
        .bearer_auth("cam001@studio.loc")
        .connect()
        .await;
    match client {
        Ok((_, framed)) => Ok(framed),
        Err(e) => bail!("Can't connect to server: {}", e),
    }
}

fn main() -> Result<(), Error> {
    ::std::env::set_var("RUST_LOG", "capture=debug,actix=debug");
    env_logger::init();

    gst::init()?;
    check_plugins()?;

    let main_loop = glib::MainLoop::new(None, false);
    let pipeline = gst::Pipeline::new(Some("main"));

    init_pipeline(&pipeline)?;

    thread::spawn(|| {
        let sys = System::new("websocket-client");

        Arbiter::spawn(async {
            let framed = get_ws_client().await.unwrap();
            let (sink, stream) = framed.split();
            let addr = ChatClient::create(|ctx| {
                ChatClient::add_stream(stream, ctx);
                ChatClient {
                    framed: SinkWrite::new(sink, ctx),
                    pipeline: StrongContext(Arc::new(Mutex::new(pipeline))),
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

        sys.run().unwrap();
    });

    main_loop.run();

    Ok(())
}

/// JSON messages exchanged with other clients
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ProtocolMessage {
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

#[derive(Debug, Deserialize, Serialize)]
struct Envelope {
    from_jid: String,
    message: ProtocolMessage,
}

struct ChatClient {
    framed: SinkWrite<Message, SplitSink<Framed<BoxedSocket, Codec>, Message>>,
    pipeline: StrongContext,
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
            act.framed
                .write(Message::Ping(Bytes::from_static(b"")))
                .unwrap();
            act.hb(ctx);

            // client should also check for a timeout here, similar to the
            // server code
        });
    }

    fn handle_text_message(&mut self, txt: &Bytes) -> Result<(), Error> {
        let utf8 = std::str::from_utf8(txt)?;
        let envelope: Envelope = serde_json::from_str(utf8)?;
        let pipeline = self.pipeline.clone();
        let answer = match envelope.message {
            ProtocolMessage::Sdp { type_, sdp } => {
                handle_sdp(pipeline, &envelope.from_jid, &type_, &sdp)?
            }
            ProtocolMessage::Ice {
                sdp_mline_index,
                candidate,
            } => handle_ice(pipeline, &envelope.from_jid, candidate, sdp_mline_index)?,
        };

        if let Some(text) = answer {
            println!("ANSWER THAT SHOULD GO OUT: {}", text);
            self.framed.write(Message::Text(text)).unwrap();
        }

        Ok(())
    }

    fn handle_websocket_message(
        &mut self,
        msg: &Result<Frame, WsProtocolError>,
    ) -> Result<(), Error> {
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
                debug!("Websocket text");
                self.handle_text_message(txt)
            }
            _ => bail!("Can't parse websocket message"),
        }
    }
}

/// Handle stdin commands
impl Handler<ClientCommand> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: ClientCommand, _ctx: &mut Context<Self>) {
        self.framed.write(Message::Text(msg.0)).unwrap();
    }
}

/// Handle server websocket messages
impl StreamHandler<Result<Frame, WsProtocolError>> for ChatClient {
    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, _: &mut Context<Self>) {
        if let Err(e) = self.handle_websocket_message(&msg) {
            error!("Can't handle websocket message: {}", e)
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
