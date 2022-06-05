use base64;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[macro_use]
extern crate log;

use bytes::Bytes;
#[macro_use]
extern crate failure;
use failure::{Error, Fail};
use futures::channel::mpsc;
use futures::stream::{SplitSink, StreamExt};
use openssl::ssl::{SslConnector, SslMethod};

use gst::{self, prelude::*};
use lazy_static::lazy_static;
use serde_derive::Deserialize;

use actix::io::SinkWrite;
use actix::*;
use actix_codec::Framed;
use awc::{
    error::WsProtocolError,
    ws::{Codec, Frame, Message},
    BoxedSocket, Client, Connector,
};

use protocol;

const STUN_SERVER: &str = "stun://stun.l.google.com:19302";
const TURN_SERVER: &str = "turn://foo:bar@webrtc.nirbheek.in:3478";
// const JSWS_SERVER: &str = "wss://guinho.home:7070/ws";

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

// #[derive(Debug, Fail)]
// #[fail(display = "Failed to create answer")]
// struct NullAnswer;

// #[derive(Debug, Fail)]
// #[fail(display = "Failed to get bus")]
// struct NullBus;

#[derive(Debug, Fail)]
#[fail(display = "Failed to retrieve element \"{}\"", _0)]
struct NullElement(&'static str);

// #[derive(Debug, Fail)]
// #[fail(display = "Failed to create offer")]
// struct NullOffer;

#[derive(Debug, Fail)]
#[fail(display = "Failed to create pad \"{}\"", _0)]
struct NullPad(&'static str);

// #[derive(Debug, Fail)]
// #[fail(display = "Failed to create reply")]
// struct NullReply;

// #[derive(Debug, Fail)]
// #[fail(display = "Failed to create session description")]
// struct NullSessionDescription;

#[derive(Clone, Debug, Deserialize)]
struct ConfigHTTP {
    server: String,
    jid: String,
    key: String,
    cert: String,
    cacert: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ConfigLogging {
    actix_server: String,
    actix_web: String,
    capture: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Config {
    http: ConfigHTTP,
    logging: Option<ConfigLogging>,
}

/// Create the HTTP client and connect it to the websocket server.
/// Then return the framed response
async fn get_ws_client(config: &Config) -> Result<Framed<BoxedSocket, Codec>, Error> {
    let mut ssl_builder = SslConnector::builder(SslMethod::tls())?;
    ssl_builder.set_ca_file(&config.http.cacert)?;
    let token = base64::encode(&config.http.jid);
    let connector = Connector::new()
        .timeout(Duration::from_secs(15))
        .ssl(ssl_builder.build())
        .finish();
    let client = Client::build()
        .connector(connector)
        .finish()
        .ws(&config.http.server)
        .bearer_auth(&token)
        .connect()
        .await;
    match client {
        Ok((_, framed)) => Ok(framed),
        Err(e) => bail!("Can't connect to server: {}", e),
    }
}

fn load_config(args: &Vec<String>) -> Result<Config, Error> {
    if args.len() != 2 {
        // Can't move on without the configuration file
        bail!(format!("Usage: {} CONFIG-FILE", args[0]))
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
async fn main() -> Result<(), Error> {
    // initialize config
    let args: Vec<String> = std::env::args().collect();
    let config: Config = load_config(&args)?;

    // initialize libraries we depend on
    env_logger::init();
    gst::init()?;

    // create communication pipes between our gst application and the websocket connection
    let (wsproxy_sender, wsproxy_receiver) = mpsc::unbounded::<protocol::Envelope>();
    let wsproxy_receiver = wsproxy_receiver.fuse();
    let framed = get_ws_client(&config)
        .await
        .expect("Can't initialize client");

    // Initialize our gstreamer handler
    let mut gstapp = GstWebRTCApp::new(config.clone(), Arc::new(Mutex::new(wsproxy_sender)));
    gstapp.init()?;

    let (sink, stream) = framed.split();
    CaptureActor::create(|ctx| {
        CaptureActor::add_stream(stream, ctx);
        CaptureActor::add_stream(wsproxy_receiver, ctx);
        CaptureActor {
            config: config,
            gstapp: gstapp,
            framed: SinkWrite::new(sink, ctx),
        }
    });

    let _ = actix_rt::signal::ctrl_c().await?;

    Ok(())
}

#[derive(Debug)]
struct GstWebRTCApp {
    config: Config,
    pipeline: gst::Pipeline,
    wsproxy: Arc<Mutex<mpsc::UnboundedSender<protocol::Envelope>>>,
}

impl GstWebRTCApp {
    fn new(config: Config, wsproxy: Arc<Mutex<mpsc::UnboundedSender<protocol::Envelope>>>) -> Self {
        let pipeline = gst::Pipeline::new(Some("main"));

        GstWebRTCApp {
            config,
            pipeline,
            wsproxy,
        }
    }

    fn init(&mut self) -> Result<(), Error> {
        check_plugins()?;
        self.init_pipeline()?;
        Ok(())
    }

    fn init_pipeline(&mut self) -> Result<(), Error> {
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

        self.pipeline.add_many(&[
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

        self.pipeline.call_async(|p| {
            p.set_state(gst::State::Playing).unwrap();
        });

        Ok(())
    }

    fn add_peer_to_pipeline(&mut self, peer_id: &str) -> Result<(), Error> {
        let queue = gst::ElementFactory::make("queue", None)?;
        let webrtcbin = gst::ElementFactory::make("webrtcbin", Some(&peer_id))?;
        webrtcbin.set_property_from_str("stun-server", STUN_SERVER);
        webrtcbin.set_property_from_str("turn-server", TURN_SERVER);
        webrtcbin.set_property_from_str("bundle-policy", "max-bundle");

        debug!("Adding peer {:?} to pipeline", peer_id);
        self.pipeline.add_many(&[&queue, &webrtcbin])?;

        let queue_src = queue.get_static_pad("src").ok_or(NullPad("queue_src"))?;
        let webrtc_sink = webrtcbin
            .get_request_pad("sink_%u")
            .ok_or(NullPad("webrtc_sink"))?;
        queue_src.link(&webrtc_sink)?;

        let tee = self
            .pipeline
            .get_by_name("videotee")
            .ok_or(NullElement("videotee"))?;
        let tee_src = tee.get_request_pad("src_%u").ok_or(NullPad("tee_src"))?;
        let queue_sink = queue.get_static_pad("sink").ok_or(NullPad("queue_sink"))?;
        tee_src.link(&queue_sink)?;

        queue.sync_state_with_parent()?;
        webrtcbin.sync_state_with_parent()?;

        // values that will be moved into the closure below
        let channel = self.wsproxy.clone();
        let peer_id = peer_id.to_string();
        let from_jid = self.config.http.jid.clone();

        webrtcbin.connect("on-ice-candidate", false, move |values| {
            let mlineindex = values[1].get_some::<u32>().expect("Invalid argument");
            let candidate = values[2].get::<String>().expect("Invalid argument")?;

            channel
                .lock()
                .unwrap()
                .unbounded_send(protocol::Envelope {
                    from_jid: from_jid.to_string(),
                    to_jid: peer_id.to_string(),
                    message: protocol::Message::NewIceCandidate {
                        candidate,
                        sdp_mline_index: mlineindex,
                    },
                })
                .unwrap();

            None
        })?;

        let pipeline = self.pipeline.downgrade();

        webrtcbin.connect_pad_added(move |_webrtc, pad| {
            // Early return for the source pads we're adding ourselves
            if pad.get_direction() != gst::PadDirection::Src {
                return;
            }

            let decodebin = gst::ElementFactory::make("decodebin", None).unwrap();

            let pipeline0 = pipeline.clone();

            decodebin.connect_pad_added(move |_decodebin, pad| {
                let caps = pad.get_current_caps().unwrap();
                let name = caps.get_structure(0).unwrap().get_name();

                let sink = if name.starts_with("video/") {
                    gst::parse_bin_from_description(
                        "queue ! videoconvert ! videoscale ! autovideosink",
                        true,
                    )
                    .unwrap()
                } else if name.starts_with("audio/") {
                    gst::parse_bin_from_description(
                        "queue ! audioconvert ! audioresample ! autoaudiosink",
                        true,
                    )
                    .unwrap()
                } else {
                    println!("Unknown pad {:?}, ignoring", pad);
                    return;
                };

                pipeline0.upgrade().unwrap().add(&sink).unwrap();
                sink.sync_state_with_parent().unwrap();

                let sinkpad = sink.get_static_pad("sink").unwrap();
                pad.link(&sinkpad).unwrap();
            });

            pipeline.upgrade().unwrap().add(&decodebin).unwrap();
            decodebin.sync_state_with_parent().unwrap();

            let sinkpad = decodebin.get_static_pad("sink").unwrap();
            pad.link(&sinkpad).unwrap();
        });

        Ok(())
    }

    fn handle_ice(
        &mut self,
        peer_id: &str,
        candidate: String,
        sdp_mline_index: u32,
    ) -> Result<(), Error> {
        let webrtcbin = self
            .pipeline
            .get_by_name(peer_id)
            .ok_or(NullElement("webrtcbin"))?;
        webrtcbin.emit("add-ice-candidate", &[&sdp_mline_index, &candidate])?;
        Ok(())
    }

    fn handle_sdp_offer(&mut self, sdp: String, peer_id: &str) -> Result<(), Error> {
        let msg = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()).unwrap();
        let offer =
            gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Offer, msg);
        let webrtcbin = self.pipeline.get_by_name(peer_id).unwrap();
        webrtcbin.emit("set-remote-description", &[&offer, &None::<gst::Promise>])?;

        // values that will be moved into the closure below
        let channel = self.wsproxy.clone();
        let peer_id = peer_id.to_string();
        let from_jid = self.config.http.jid.clone();
        let pipeline = self.pipeline.downgrade();

        let promise = &gst::Promise::new_with_change_func(move |reply| {
            let answer = reply
                .unwrap()
                .get_value("answer")
                .unwrap()
                .get::<gst_webrtc::WebRTCSessionDescription>()
                .unwrap()
                // .ok_or(NullSessionDescription)
                .unwrap();

            let webrtcbin = pipeline
                .upgrade()
                .unwrap()
                .get_by_name(&peer_id)
                // .ok_or(NullElement("webrtcbin"))
                .unwrap();

            // println!("THE ANSWER {:?}", reply?.get_value("answer"));

            webrtcbin
                .emit("set-local-description", &[&answer, &None::<gst::Promise>])
                .unwrap();

            channel
                .lock()
                .unwrap()
                .unbounded_send(protocol::Envelope {
                    from_jid: from_jid,
                    to_jid: peer_id,
                    message: protocol::Message::CallAnswer {
                        sdp: protocol::SDP {
                            type_: "answer".to_string(),
                            sdp: answer.get_sdp().as_text().unwrap(),
                        },
                    },
                })
                .unwrap();
        });

        webrtcbin.emit("create-answer", &[&None::<gst::Structure>, &promise])?;
        Ok(())
    }
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

struct CaptureActor {
    config: Config,
    gstapp: GstWebRTCApp,
    framed: SinkWrite<Message, SplitSink<Framed<BoxedSocket, Codec>, Message>>,
}

impl Actor for CaptureActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        let from_jid = self.config.http.jid.clone();
        let msg = protocol::Envelope {
            from_jid: from_jid,
            to_jid: "".to_string(),
            message: protocol::Message::Capabilities(self.capabilities()),
        };
        let json_text = serde_json::to_string(&msg).unwrap();
        self.framed.write(Message::Text(json_text)).unwrap();
        self.hb(ctx)
    }

    fn stopped(&mut self, _: &mut Context<Self>) {
        println!("Disconnected");
        // Stop application on disconnect
        System::current().stop();
    }
}

impl CaptureActor {
    fn capabilities(&self) -> HashSet<String> {
        let mut caps = HashSet::new();
        caps.insert("produce:video".to_string());
        caps.insert("produce:audio".to_string());
        caps.insert("consume:audio".to_string());
        return caps;
    }

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
        let envelope: protocol::Envelope = serde_json::from_str(utf8)?;
        let peer_id = envelope.from_jid;

        match envelope.message {
            protocol::Message::CallOffer { sdp } => {
                info!("Handle call offered by {}", peer_id);
                self.gstapp.add_peer_to_pipeline(&peer_id)?;
                if sdp.type_ == "offer" {
                    self.gstapp.handle_sdp_offer(sdp.sdp, &peer_id)?;
                } else {
                    println!(r#"Sdp type is not "offer""#);
                }
            }

            protocol::Message::NewIceCandidate {
                sdp_mline_index,
                candidate,
            } => {
                info!("Handle ICE candidate from {}", peer_id);
                self.gstapp
                    .handle_ice(&peer_id, candidate, sdp_mline_index)?;
            }

            msg @ _ => {
                warn!("MESSAGE NOT HANDLED: {:?}", msg);
            }
        };
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
                debug!("Websocket text: {:?}", txt);
                self.handle_text_message(txt)?;
                Ok(())
            }
            _ => bail!("Can't parse websocket message"),
        }
    }
}

/// Handles messages from the GstApp actor to be forwarded via the
/// websocket client.
impl StreamHandler<protocol::Envelope> for CaptureActor {
    fn handle(&mut self, msg: protocol::Envelope, _ctx: &mut Context<Self>) {
        let json_text = serde_json::to_string(&msg).unwrap();
        debug!("Message sent: {}", &json_text);
        self.framed.write(Message::Text(json_text)).unwrap();
    }

    fn finished(&mut self, _ctx: &mut Self::Context) {
        error!("OH NOOO");
    }
}

/// Handle server websocket messages
impl StreamHandler<Result<Frame, WsProtocolError>> for CaptureActor {
    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, _: &mut Context<Self>) {
        if let Err(e) = self.handle_websocket_message(&msg) {
            error!("Can't handle websocket message: {:?}, {}", &msg, e)
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

impl actix::io::WriteHandler<WsProtocolError> for CaptureActor {
    fn error(&mut self, err: WsProtocolError, _ctx: &mut Self::Context) -> Running {
        println!("ERROR {:?}", err);
        Running::Stop
    }
}
