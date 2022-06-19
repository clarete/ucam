use base64;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

#[macro_use]
extern crate log;

use bytes::Bytes;
#[macro_use]
extern crate failure;
use failure::{Error, Fail};
use futures::channel::mpsc;
use futures::stream::{SplitSink, Stream, StreamExt};
use openssl::ssl::{SslConnector, SslMethod};

use crate::failure::ResultExt;

use gst::gst_element_error;
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
const VIDEO_WIDTH: u32 = 1024;
const VIDEO_HEIGHT: u32 = 768;

// upgrade weak reference or return
#[macro_export]
macro_rules! upgrade_weak {
    ($x:ident, $r:expr) => {{
        match $x.upgrade() {
            Some(o) => o,
            None => return $r,
        }
    }};
    ($x:ident) => {
        upgrade_weak!($x, ())
    };
}

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

// Strong reference to our application state
#[derive(Debug, Clone)]
struct App(Arc<AppInner>);

// Weak reference to our application state
#[derive(Debug, Clone)]
struct AppWeak(Weak<AppInner>);

// Actual application state
#[derive(Debug)]
struct AppInner {
    config: Config,
    pipeline: gst::Pipeline,
    video_tee: gst::Element,
    audio_tee: gst::Element,
    video_mixer: gst::Element,
    audio_mixer: gst::Element,
    send_msg_tx: Arc<Mutex<mpsc::UnboundedSender<protocol::Envelope>>>,
    peers: Mutex<BTreeMap<String, Peer>>,
}

// Strong reference to the state of one peer
#[derive(Debug, Clone)]
struct Peer(Arc<PeerInner>);

// Weak reference to the state of one peer
#[derive(Debug, Clone)]
struct PeerWeak(Weak<PeerInner>);

// Actual peer state
#[derive(Debug)]
struct PeerInner {
    peer_id: String,
    bin: gst::Bin,
    webrtcbin: gst::Element,
    send_msg_tx: Arc<Mutex<mpsc::UnboundedSender<protocol::Envelope>>>,
}

// To be able to access the App's fields directly
impl std::ops::Deref for App {
    type Target = AppInner;

    fn deref(&self) -> &AppInner {
        &self.0
    }
}

// To be able to access the Peers's fields directly
impl std::ops::Deref for Peer {
    type Target = PeerInner;

    fn deref(&self) -> &PeerInner {
        &self.0
    }
}

impl AppWeak {
    // Try upgrading a weak reference to a strong one
    fn upgrade(&self) -> Option<App> {
        self.0.upgrade().map(App)
    }
}

impl PeerWeak {
    // Try upgrading a weak reference to a strong one
    fn upgrade(&self) -> Option<Peer> {
        self.0.upgrade().map(Peer)
    }
}

impl App {
    // Downgrade the strong reference to a weak reference
    fn downgrade(&self) -> AppWeak {
        AppWeak(Arc::downgrade(&self.0))
    }

    fn new(
        config: Config,
        // initial_peers: &[&str],
    ) -> Result<
        (
            Self,
            impl Stream<Item = gst::Message>,
            impl Stream<Item = protocol::Envelope>,
        ),
        Error,
    > {
        // Create the GStreamer pipeline
        let pipeline = gst::parse_launch(
            &format!(
                "videotestsrc is-live=true ! vp8enc deadline=1 ! rtpvp8pay pt=96 ! tee name=video-tee ! \
                 queue ! fakesink sync=true \
                 audiotestsrc wave=ticks is-live=true ! opusenc ! rtpopuspay pt=97 ! tee name=audio-tee ! \
                 queue ! fakesink sync=true \
                 audiotestsrc wave=silence is-live=true ! audio-mixer. \
                 audiomixer name=audio-mixer sink_0::mute=true ! audioconvert ! audioresample ! autoaudiosink \
                 videotestsrc pattern=black ! capsfilter caps=video/x-raw,width=1,height=1 ! video-mixer. \
                 compositor name=video-mixer background=black sink_0::alpha=0.0 ! capsfilter caps=video/x-raw,width={width},height={height} ! videoconvert ! autovideosink",
                width=VIDEO_WIDTH,
                height=VIDEO_HEIGHT,
        ))?;

        // Downcast from gst::Element to gst::Pipeline
        let pipeline = pipeline
            .downcast::<gst::Pipeline>()
            .expect("not a pipeline");

        // Get access to the tees and mixers by name
        let video_tee = pipeline
            .get_by_name("video-tee")
            .expect("can't find video-tee");
        let audio_tee = pipeline
            .get_by_name("audio-tee")
            .expect("can't find audio-tee");

        let video_mixer = pipeline
            .get_by_name("video-mixer")
            .expect("can't find video-mixer");
        let audio_mixer = pipeline
            .get_by_name("audio-mixer")
            .expect("can't find audio-mixer");

        // Create a stream for handling the GStreamer message asynchronously
        let bus = pipeline.get_bus().unwrap();
        let send_gst_msg_rx = bus.stream();

        // Channel for outgoing WebSocket messages from other threads
        let (send_ws_msg_tx, send_ws_msg_rx) = mpsc::unbounded::<protocol::Envelope>();

        // Asynchronously set the pipeline to Playing
        pipeline.call_async(|pipeline| {
            pipeline
                .set_state(gst::State::Playing)
                .expect("Couldn't set pipeline to Playing");
        });

        let app = App(Arc::new(AppInner {
            config,
            pipeline,
            video_tee,
            audio_tee,
            video_mixer,
            audio_mixer,
            peers: Mutex::new(BTreeMap::new()),
            send_msg_tx: Arc::new(Mutex::new(send_ws_msg_tx)),
        }));

        // for peer in initial_peers {
        //     app.add_peer(peer, true)?;
        // }

        // Asynchronously set the pipeline to Playing
        app.pipeline.call_async(|pipeline| {
            // If this fails, post an error on the bus so we exit
            if pipeline.set_state(gst::State::Playing).is_err() {
                gst_element_error!(
                    pipeline,
                    gst::LibraryError::Failed,
                    ("Failed to set pipeline to Playing")
                );
            }
        });

        Ok((app, send_gst_msg_rx, send_ws_msg_rx))
    }

    // Handle WebSocket messages, both our own as well as WebSocket protocol messages
    fn handle_websocket_message(&mut self, text_msg: &Bytes) -> Result<(), Error> {
        let msg = std::str::from_utf8(text_msg)?;
        let envelope: protocol::Envelope = serde_json::from_str(msg)?;

        match envelope.message {
            protocol::Message::ClientOnline {
                capabilities: _capabilities,
            } => self.add_peer(&envelope.from_jid, false),
            protocol::Message::ClientOffline => self.remove_peer(&envelope.from_jid),
            protocol::Message::CallOffer { sdp } => {
                info!("Handle call offered by {}", envelope.from_jid);
                match self.get_peer(&envelope.from_jid) {
                    Some(peer) => peer.handle_sdp(&sdp.type_, &sdp.sdp),
                    None => bail!("Can't find peer {}", envelope.from_jid),
                }
            }
            protocol::Message::NewIceCandidate {
                sdp_mline_index,
                candidate,
            } => {
                info!("Handle ICE candidate from {}", envelope.from_jid);
                match self.get_peer(&envelope.from_jid) {
                    Some(peer) => peer.handle_ice(sdp_mline_index, &candidate),
                    None => bail!("Can't find peer {}", envelope.from_jid),
                }
            }
            _ => {
                bail!("WAT ARE YOU DOIN");
            }
        }
    }

    // Receive GStreamer messages coming from the pipeline and forward them to the error handling mechanism
    fn handle_pipeline_message(&self, message: &gst::Message) -> Result<(), Error> {
        use gst::message::MessageView;

        match message.view() {
            MessageView::Error(err) => bail!(
                "Error from element {}: {} ({})",
                err.get_src()
                    .map(|s| String::from(s.get_path_string()))
                    .unwrap_or_else(|| String::from("None")),
                err.get_error(),
                err.get_debug().unwrap_or_else(|| String::from("None")),
            ),
            MessageView::Warning(warning) => {
                println!("Warning: \"{}\"", warning.get_debug().unwrap());
            }
            _ => (),
        }

        Ok(())
    }

    fn get_peer(&self, peer: &str) -> Option<Peer> {
        let peers = self.peers.lock().unwrap();
        if let Some(p) = peers.get(&peer.to_string()) {
            return Some(p.clone());
        }
        None
    }

    // Add this new peer and if requested, send the offer to it
    fn add_peer(&mut self, peer: &str, offer: bool) -> Result<(), Error> {
        println!("Adding peer {}", peer);
        let peer_id = peer.to_string();
        let mut peers = self.peers.lock().unwrap();
        if peers.contains_key(&peer_id) {
            bail!("Peer {} already called", peer_id);
        }

        let peer_bin = gst::parse_bin_from_description(
            "queue name=video-queue ! webrtcbin. \
             queue name=audio-queue ! webrtcbin. \
             webrtcbin name=webrtcbin",
            false,
        )?;

        // Get access to the webrtcbin by name
        let webrtcbin = peer_bin
            .get_by_name("webrtcbin")
            .expect("can't find webrtcbin");

        // Set some properties on webrtcbin
        webrtcbin.set_property_from_str("stun-server", STUN_SERVER);
        webrtcbin.set_property_from_str("turn-server", TURN_SERVER);
        webrtcbin.set_property_from_str("bundle-policy", "max-bundle");

        // Add ghost pads for connecting to the input
        let audio_queue = peer_bin
            .get_by_name("audio-queue")
            .expect("can't find audio-queue");
        let audio_sink_pad = gst::GhostPad::new(
            Some("audio_sink"),
            &audio_queue.get_static_pad("sink").unwrap(),
        )
        .unwrap();
        peer_bin.add_pad(&audio_sink_pad).unwrap();

        let video_queue = peer_bin
            .get_by_name("video-queue")
            .expect("can't find video-queue");
        let video_sink_pad = gst::GhostPad::new(
            Some("video_sink"),
            &video_queue.get_static_pad("sink").unwrap(),
        )
        .unwrap();
        peer_bin.add_pad(&video_sink_pad).unwrap();

        let peer = Peer(Arc::new(PeerInner {
            peer_id: peer_id.clone(),
            bin: peer_bin,
            webrtcbin,
            send_msg_tx: self.send_msg_tx.clone(),
        }));

        // Insert the peer into our map
        peers.insert(peer_id, peer.clone());
        drop(peers);

        // Add to the whole pipeline
        self.pipeline.add(&peer.bin).unwrap();

        // If we should send the offer to the peer, do so from on-negotiation-needed
        if offer {
            // Connect to on-negotiation-needed to handle sending an Offer
            let peer_clone = peer.downgrade();
            peer.webrtcbin
                .connect("on-negotiation-needed", false, move |values| {
                    let _webrtc = values[0].get::<gst::Element>().unwrap();

                    let peer = upgrade_weak!(peer_clone, None);
                    if let Err(err) = peer.on_negotiation_needed() {
                        gst_element_error!(
                            peer.bin,
                            gst::LibraryError::Failed,
                            ("Failed to negotiate: {:?}", err)
                        );
                    }

                    None
                })
                .unwrap();
        }

        // Whenever there is a new ICE candidate, send it to the peer
        let peer_clone = peer.downgrade();
        peer.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                println!("on-ice-candidate");

                let _webrtc = values[0].get::<gst::Element>().expect("Invalid argument");
                let mlineindex = values[1].get_some::<u32>().expect("Invalid argument");
                let candidate = values[2]
                    .get::<String>()
                    .expect("Invalid argument")
                    .unwrap();

                let peer = upgrade_weak!(peer_clone, None);

                if let Err(err) = peer.on_ice_candidate(mlineindex, candidate) {
                    gst_element_error!(
                        peer.bin,
                        gst::LibraryError::Failed,
                        ("Failed to send ICE candidate: {:?}", err)
                    );
                }

                None
            })
            .unwrap();

        // Whenever there is a new stream incoming from the peer, handle it
        let peer_clone = peer.downgrade();
        peer.webrtcbin.connect_pad_added(move |_webrtc, pad| {
            let peer = upgrade_weak!(peer_clone);

            if let Err(err) = peer.on_incoming_stream(pad) {
                gst_element_error!(
                    peer.bin,
                    gst::LibraryError::Failed,
                    ("Failed to handle incoming stream: {:?}", err)
                );
            }
        });

        // Whenever a decoded stream comes available, handle it and connect it to the mixers
        let app_clone = self.downgrade();
        peer.bin.connect_pad_added(move |_bin, pad| {
            let app = upgrade_weak!(app_clone);

            if pad.get_name() == "audio_src" {
                let audiomixer_sink_pad = app.audio_mixer.get_request_pad("sink_%u").unwrap();
                pad.link(&audiomixer_sink_pad).unwrap();

                // Once it is unlinked again later when the peer is being removed,
                // also release the pad on the mixer
                audiomixer_sink_pad.connect_unlinked(move |pad, _peer| {
                    if let Some(audiomixer) = pad.get_parent() {
                        let audiomixer = audiomixer.downcast_ref::<gst::Element>().unwrap();
                        audiomixer.release_request_pad(pad);
                    }
                });
            } else if pad.get_name() == "video_src" {
                let videomixer_sink_pad = app.video_mixer.get_request_pad("sink_%u").unwrap();
                pad.link(&videomixer_sink_pad).unwrap();

                app.relayout_videomixer();

                // Once it is unlinked again later when the peer is being removed,
                // also release the pad on the mixer
                let app_clone = app.downgrade();
                videomixer_sink_pad.connect_unlinked(move |pad, _peer| {
                    let app = upgrade_weak!(app_clone);

                    if let Some(videomixer) = pad.get_parent() {
                        let videomixer = videomixer.downcast_ref::<gst::Element>().unwrap();
                        videomixer.release_request_pad(pad);
                    }

                    app.relayout_videomixer();
                });
            }
        });

        // Add pad probes to both tees for blocking them and
        // then unblock them once we reached the Playing state.
        //
        // Then link them and unblock, in case they got blocked
        // in the meantime.
        //
        // Otherwise it might happen that data is received before
        // the elements are ready and then an error happens.
        let audio_src_pad = self.audio_tee.get_request_pad("src_%u").unwrap();
        let audio_block = audio_src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_pad, _info| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();
        audio_src_pad.link(&audio_sink_pad)?;

        let video_src_pad = self.video_tee.get_request_pad("src_%u").unwrap();
        let video_block = video_src_pad
            .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_pad, _info| {
                gst::PadProbeReturn::Ok
            })
            .unwrap();
        video_src_pad.link(&video_sink_pad)?;

        // Asynchronously set the peer bin to Playing
        peer.bin.call_async(move |bin| {
            // If this fails, post an error on the bus so we exit
            if bin.sync_state_with_parent().is_err() {
                gst_element_error!(
                    bin,
                    gst::LibraryError::Failed,
                    ("Failed to set peer bin to Playing")
                );
            }

            // And now unblock
            audio_src_pad.remove_probe(audio_block);
            video_src_pad.remove_probe(video_block);
        });

        Ok(())
    }

    // Remove this peer
    fn remove_peer(&self, peer: &str) -> Result<(), Error> {
        println!("Removing peer {}", peer);
        // let peer_id = str::parse::<u32>(peer).with_context(|_| format!("Can't parse peer id"))?;
        let peer_id = peer.to_string();
        let mut peers = self.peers.lock().unwrap();
        if let Some(peer) = peers.remove(&peer_id) {
            drop(peers);

            // Now asynchronously remove the peer from the pipeline
            let app_clone = self.downgrade();
            self.pipeline.call_async(move |_pipeline| {
                let app = upgrade_weak!(app_clone);

                // Block the tees shortly for removal
                let audio_tee_sinkpad = app.audio_tee.get_static_pad("sink").unwrap();
                let audio_block = audio_tee_sinkpad
                    .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_pad, _info| {
                        gst::PadProbeReturn::Ok
                    })
                    .unwrap();

                let video_tee_sinkpad = app.video_tee.get_static_pad("sink").unwrap();
                let video_block = video_tee_sinkpad
                    .add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_pad, _info| {
                        gst::PadProbeReturn::Ok
                    })
                    .unwrap();

                // Release the tee pads and unblock
                let audio_sinkpad = peer.bin.get_static_pad("audio_sink").unwrap();
                let video_sinkpad = peer.bin.get_static_pad("video_sink").unwrap();

                if let Some(audio_tee_srcpad) = audio_sinkpad.get_peer() {
                    let _ = audio_tee_srcpad.unlink(&audio_sinkpad);
                    app.audio_tee.release_request_pad(&audio_tee_srcpad);
                }
                audio_tee_sinkpad.remove_probe(audio_block);

                if let Some(video_tee_srcpad) = video_sinkpad.get_peer() {
                    let _ = video_tee_srcpad.unlink(&video_sinkpad);
                    app.video_tee.release_request_pad(&video_tee_srcpad);
                }
                video_tee_sinkpad.remove_probe(video_block);

                // Then remove the peer bin gracefully from the pipeline
                let _ = app.pipeline.remove(&peer.bin);
                let _ = peer.bin.set_state(gst::State::Null);

                println!("Removed peer {}", peer.peer_id);
            });
        }

        Ok(())
    }

    fn relayout_videomixer(&self) {
        let mut pads = self.video_mixer.get_sink_pads();
        if pads.is_empty() {
            return;
        }

        // We ignore the first pad
        pads.remove(0);
        let npads = pads.len();

        let (width, height) = if npads <= 1 {
            (1, 1)
        } else if npads <= 4 {
            (2, 2)
        } else if npads <= 16 {
            (4, 4)
        } else {
            // FIXME: we don't support more than 16 streams for now
            (4, 4)
        };

        let mut x: i32 = 0;
        let mut y: i32 = 0;
        let w = VIDEO_WIDTH as i32 / width;
        let h = VIDEO_HEIGHT as i32 / height;

        for pad in pads {
            pad.set_property("xpos", &x).unwrap();
            pad.set_property("ypos", &y).unwrap();
            pad.set_property("width", &w).unwrap();
            pad.set_property("height", &h).unwrap();

            x += w;
            if x >= VIDEO_WIDTH as i32 {
                x = 0;
                y += h;
            }
        }
    }
}

// Make sure to shut down the pipeline when it goes out of scope
// to release any system resources
impl Drop for AppInner {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Peer {
    // Downgrade the strong reference to a weak reference
    fn downgrade(&self) -> PeerWeak {
        PeerWeak(Arc::downgrade(&self.0))
    }

    // Whenever webrtcbin tells us that (re-)negotiation is needed, simply ask
    // for a new offer SDP from webrtcbin without any customization and then
    // asynchronously send it to the peer via the WebSocket connection
    fn on_negotiation_needed(&self) -> Result<(), Error> {
        println!("starting negotiation with peer {}", self.peer_id);

        let peer_clone = self.downgrade();
        let promise = gst::Promise::new_with_change_func(move |r| {
            let peer = upgrade_weak!(peer_clone);
            let reply = match r {
                Ok(r) => r,
                Err(err) => {
                    gst_element_error!(
                        peer.bin,
                        gst::LibraryError::Failed,
                        ("Failed to send SDP offer[0]: {:?}", err)
                    );
                    return;
                }
            };

            if let Err(err) = peer.on_offer_created(Ok(Some(reply))) {
                gst_element_error!(
                    peer.bin,
                    gst::LibraryError::Failed,
                    ("Failed to send SDP offer[1]: {:?}", err)
                );
            }
        });

        self.webrtcbin
            .emit("create-offer", &[&None::<gst::Structure>, &promise])
            .unwrap();

        Ok(())
    }

    // Once webrtcbin has create the offer SDP for us, handle it by sending it to the peer via the
    // WebSocket connection
    fn on_offer_created(
        &self,
        reply: Result<Option<&gst::StructureRef>, gst::PromiseError>,
    ) -> Result<(), Error> {
        let reply = match reply {
            Ok(Some(reply)) => reply,
            Ok(None) => {
                bail!("Offer creation future got no reponse");
            }
            Err(err) => {
                bail!("Offer creation future got error reponse: {:?}", err);
            }
        };

        let offer = reply
            .get_value("offer")
            .unwrap()
            .get::<gst_webrtc::WebRTCSessionDescription>()
            .expect("Invalid argument")
            .unwrap();
        self.webrtcbin
            .emit("set-local-description", &[&offer, &None::<gst::Promise>])
            .unwrap();

        println!(
            "sending SDP offer to peer: {}",
            offer.get_sdp().as_text().unwrap()
        );

        self.send_msg_tx
            .lock()
            .unwrap()
            .unbounded_send(protocol::Envelope {
                from_jid: "cam001@studio.loc".to_string(),
                to_jid: self.peer_id.clone(),
                message: protocol::Message::CallOffer {
                    sdp: protocol::SDP {
                        type_: "offer".to_string(),
                        sdp: offer.get_sdp().as_text().unwrap(),
                    },
                },
            })
            .with_context(|_| format!("Failed to send SDP offer"))?;

        Ok(())
    }

    // Once webrtcbin has create the answer SDP for us, handle it by sending it to the peer via the
    // WebSocket connection
    fn on_answer_created(
        &self,
        reply: Result<Option<&gst::StructureRef>, gst::PromiseError>,
    ) -> Result<(), Error> {
        let reply = match reply {
            Ok(Some(reply)) => reply,
            Ok(None) => {
                bail!("Answer creation future got no reponse");
            }
            Err(err) => {
                bail!("Answer creation future got error reponse: {:?}", err);
            }
        };

        let answer = reply
            .get_value("answer")
            .unwrap()
            .get::<gst_webrtc::WebRTCSessionDescription>()
            .expect("Invalid argument")
            .unwrap();
        self.webrtcbin
            .emit("set-local-description", &[&answer, &None::<gst::Promise>])
            .unwrap();

        let type_ = "answer".to_string();

        let sdp = answer.get_sdp().as_text().unwrap();

        println!("sending SDP {} to peer {}: {}", type_, self.peer_id, sdp);

        self.send_msg_tx
            .lock()
            .unwrap()
            .unbounded_send(protocol::Envelope {
                from_jid: "cam001@studio.loc".to_string(),
                to_jid: self.peer_id.clone(),
                message: protocol::Message::CallAnswer {
                    sdp: protocol::SDP { type_, sdp },
                },
            })
            .with_context(|_| format!("Failed to send SDP answer"))?;

        Ok(())
    }

    // Handle incoming SDP answers from the peer
    fn handle_sdp(&self, type_: &str, sdp: &str) -> Result<(), Error> {
        if type_ == "answer" {
            print!("Received answer:\n{}\n", sdp);

            let ret = match gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) {
                Ok(r) => r,
                Err(_) => bail!("Failed to parse SDP answer"),
            };
            // .map_err(|_| bail!("Failed to parse SDP answer"))?;

            let answer =
                gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Answer, ret);

            self.webrtcbin
                .emit("set-remote-description", &[&answer, &None::<gst::Promise>])
                .unwrap();

            Ok(())
        } else if type_ == "offer" {
            print!("Received offer:\n{}\n", sdp);

            let ret = match gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) {
                Ok(r) => r,
                Err(_) => bail!("Failed to parse SDP offer"),
            };

            // And then asynchronously start our pipeline and do the next steps. The
            // pipeline needs to be started before we can create an answer
            let peer_clone = self.downgrade();
            self.bin.call_async(move |_pipeline| {
                let peer = upgrade_weak!(peer_clone);

                let offer = gst_webrtc::WebRTCSessionDescription::new(
                    gst_webrtc::WebRTCSDPType::Offer,
                    ret,
                );

                peer.0
                    .webrtcbin
                    .emit("set-remote-description", &[&offer, &None::<gst::Promise>])
                    .unwrap();

                let peer_clone = peer.downgrade();
                let promise = gst::Promise::new_with_change_func(move |r| {
                    let peer = upgrade_weak!(peer_clone);
                    let reply = match r {
                        Ok(r) => r,
                        Err(err) => {
                            gst_element_error!(
                                peer.bin,
                                gst::LibraryError::Failed,
                                ("Failed to send SDP answer[0]: {:?}", err)
                            );
                            return;
                        }
                    };
                    if let Err(err) = peer.on_answer_created(Ok(Some(reply))) {
                        gst_element_error!(
                            peer.bin,
                            gst::LibraryError::Failed,
                            ("Failed to send SDP answer[1]: {:?}", err)
                        );
                    }
                });

                peer.0
                    .webrtcbin
                    .emit("create-answer", &[&None::<gst::Structure>, &promise])
                    .unwrap();
            });

            Ok(())
        } else {
            bail!("Sdp type is not \"answer\" but \"{}\"", type_)
        }
    }

    // Handle incoming ICE candidates from the peer by passing them to webrtcbin
    fn handle_ice(&self, sdp_mline_index: u32, candidate: &str) -> Result<(), Error> {
        self.webrtcbin
            .emit("add-ice-candidate", &[&sdp_mline_index, &candidate])
            .unwrap();

        Ok(())
    }

    // Asynchronously send ICE candidates to the peer via the WebSocket connection as a JSON
    // message
    fn on_ice_candidate(&self, sdp_mline_index: u32, candidate: String) -> Result<(), Error> {
        self.send_msg_tx
            .lock()
            .unwrap()
            .unbounded_send(protocol::Envelope {
                from_jid: "cam001@studio.loc".to_string(),
                to_jid: self.peer_id.clone(),
                message: protocol::Message::NewIceCandidate {
                    candidate,
                    sdp_mline_index,
                },
            })
            .with_context(|_| format!("Failed to send ICE candidate"))?;

        Ok(())
    }

    // Whenever there's a new incoming, encoded stream from the peer create a new decodebin
    // and audio/video sink depending on the stream type
    fn on_incoming_stream(&self, pad: &gst::Pad) -> Result<(), Error> {
        // Early return for the source pads we're adding ourselves
        if pad.get_direction() != gst::PadDirection::Src {
            return Ok(());
        }

        let caps = pad.get_current_caps().unwrap();
        let s = caps.get_structure(0).unwrap();
        let media_type = match s.get::<&str>("media").expect("Invalid type") {
            Some(mt) => mt,
            None => bail!("no media type in caps {:?}", caps),
        };

        let conv = if media_type == "video" {
            gst::parse_bin_from_description(
                &format!(
                    "decodebin name=dbin ! queue ! videoconvert ! videoscale ! capsfilter name=src caps=video/x-raw,width={width},height={height},pixel-aspect-ratio=1/1",
                    width=VIDEO_WIDTH,
                    height=VIDEO_HEIGHT
                ),
                false,
            )?
        } else if media_type == "audio" {
            gst::parse_bin_from_description(
                "decodebin name=dbin ! queue ! audioconvert ! audioresample name=src",
                false,
            )?
        } else {
            println!("Unknown pad {:?}, ignoring", pad);
            return Ok(());
        };

        // Add a ghost pad on our conv bin that proxies the sink pad of the decodebin
        let dbin = conv.get_by_name("dbin").unwrap();
        let sinkpad =
            gst::GhostPad::new(Some("sink"), &dbin.get_static_pad("sink").unwrap()).unwrap();
        conv.add_pad(&sinkpad).unwrap();

        // And another one that proxies the source pad of the last element
        let src = conv.get_by_name("src").unwrap();
        let srcpad = gst::GhostPad::new(Some("src"), &src.get_static_pad("src").unwrap()).unwrap();
        conv.add_pad(&srcpad).unwrap();

        self.bin.add(&conv).unwrap();
        conv.sync_state_with_parent()
            .with_context(|_| format!("can't start sink for stream {:?}", caps))?;

        pad.link(&sinkpad)
            .with_context(|_| format!("can't link sink for stream {:?}", caps))?;

        // And then add a new ghost pad to the peer bin that proxies the source pad we added above
        if media_type == "video" {
            let srcpad = gst::GhostPad::new(Some("video_src"), &srcpad).unwrap();
            srcpad.set_active(true).unwrap();
            self.bin.add_pad(&srcpad).unwrap();
        } else if media_type == "audio" {
            let srcpad = gst::GhostPad::new(Some("audio_src"), &srcpad).unwrap();
            srcpad.set_active(true).unwrap();
            self.bin.add_pad(&srcpad).unwrap();
        }

        Ok(())
    }
}

// At least shut down the bin here if it didn't happen so far
impl Drop for PeerInner {
    fn drop(&mut self) {
        let _ = self.bin.set_state(gst::State::Null);
    }
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
    gst::init()?;

    // initialize config
    let args: Vec<String> = std::env::args().collect();
    let config: Config = load_config(&args)?;

    // initialize libraries we depend on
    env_logger::init();
    check_plugins()?;

    // create the websocket client and connect to the server
    let framed = get_ws_client(&config)
        .await
        .expect("Can't initialize client");

    // Create our application state
    let (app, send_gst_msg_rx, send_ws_msg_rx) = App::new(config.clone())?;

    let (sink, stream) = framed.split();

    CaptureActor::create(|ctx| {
        CaptureActor::add_stream(stream, ctx);
        CaptureActor::add_stream(send_ws_msg_rx, ctx);
        CaptureActor::add_stream(send_gst_msg_rx, ctx);
        CaptureActor {
            config: config,
            gstapp: app,
            framed: SinkWrite::new(sink, ctx),
        }
    });

    let _ = actix_rt::signal::ctrl_c().await?;

    Ok(())
}

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
        "compositor",
        "audiomixer",
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

struct CaptureActor {
    config: Config,
    gstapp: App,
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
}

impl StreamHandler<gst::Message> for CaptureActor {
    fn handle(&mut self, msg: gst::Message, _ctx: &mut Context<Self>) {
        if let Err(e) = self.gstapp.handle_pipeline_message(&msg) {
            error!("Can't handle gst message: {:?}, {}", msg, e);
        }
    }

    fn finished(&mut self, _ctx: &mut Self::Context) {
        error!("OH NOOO");
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
        match msg {
            Ok(Frame::Ping(_)) | Ok(Frame::Pong(_)) => {}
            Ok(Frame::Binary(_)) => {
                debug!("Websocket binary?!");
            }
            Ok(Frame::Close(_)) => {
                debug!("Websocket close");
            }
            Ok(Frame::Text(txt)) => {
                debug!("Websocket text: {:?}", txt);
                if let Err(e) = self.gstapp.handle_websocket_message(&txt) {
                    error!("Can't handle websocket message: {:?}, {}", txt, e);
                }
            }
            Err(e) => {
                error!("Error handling websocket message: {:?}", e);
            }
            m @ _ => {
                error!("Unhandled websocket message: {:?}", m);
            }
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
