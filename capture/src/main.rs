use base64;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

#[macro_use]
extern crate log;

use bytes::Bytes;
use futures::channel::mpsc;
use futures::stream::{SplitSink, Stream, StreamExt};
use openssl::ssl::{SslConnector, SslMethod};

use gst::gst_element_error;
use gst::{self, prelude::*};
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

mod err;

use err::{Error, ErrorType};

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
    local_id: String,
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
        let bus = pipeline
            .get_bus()
            .expect("Pipeline without bus. Shouldn't happen!");
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
            protocol::Message::PeerOnline {
                capabilities: _capabilities,
            } => self.add_peer(&envelope.from_jid, false),
            protocol::Message::PeerOffline => self.remove_peer(&envelope.from_jid),
            protocol::Message::PeerRequestCall => {
                let peers = self.peers.lock().unwrap();

                if let Some(peer) = peers.get(&envelope.from_jid) {
                    info!("call request from={}", envelope.from_jid);

                    if let Err(err) = peer.on_negotiation_needed() {
                        gst_element_error!(
                            peer.bin,
                            gst::LibraryError::Failed,
                            ("Failed to negotiate: {:?}", err)
                        );
                    }
                }

                Ok(())
            }
            protocol::Message::SDP { type_, sdp } => {
                info!("Handle call offer by {}", envelope.from_jid);
                let jid = envelope.from_jid.clone();
                let peer = self
                    .get_peer(&envelope.from_jid)
                    .ok_or_else(move || Error::new_proto(format!("Can't find peer: {}", jid)))?;
                peer.handle_sdp(&type_, &sdp)
            }
            protocol::Message::ICE {
                sdp_mline_index,
                candidate,
            } => {
                info!("Handle ICE candidate from {}", envelope.from_jid);
                let jid = envelope.from_jid.clone();
                let peer = self
                    .get_peer(&envelope.from_jid)
                    .ok_or_else(move || Error::new_proto(format!("Can't find peer: {}", jid)))?;
                peer.handle_ice(sdp_mline_index, &candidate)
            }
            msg @ _ => Err(Error::new_proto(format!("Unknown message: {:?}", msg))),
        }
    }

    // Receive GStreamer messages coming from the pipeline and forward them to the error handling mechanism
    fn handle_pipeline_message(&self, message: &gst::Message) -> Result<(), Error> {
        use gst::message::MessageView;

        match message.view() {
            MessageView::Error(err) => {
                return Err(Error::new_gst(format!(
                    "Error from element {}: {} ({})",
                    err.get_src()
                        .map(|s| String::from(s.get_path_string()))
                        .unwrap_or_else(|| String::from("None")),
                    err.get_error(),
                    err.get_debug().unwrap_or_else(|| String::from("None")),
                )));
            }
            MessageView::Warning(warning) => {
                warn!("{}", warning.get_debug().unwrap());
            }
            MessageView::StateChanged(state_changed) => {
                let current = state_changed.get_current();
                let bin_ref = self.pipeline.upcast_ref::<gst::Bin>();
                bin_ref.debug_to_dot_file(gst::DebugGraphDetails::all(), state_name(current));
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
        debug!("Adding peer {}", peer);
        let peer_id = peer.to_string();
        let mut peers = self.peers.lock().unwrap();
        if peers.contains_key(&peer_id) {
            warn!("Peer {} already called", peer_id);
            return Ok(());
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
            local_id: "cam001@studio.loc".to_string(),
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
            debug!("webrtcbin.connect_pad_added");

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
            debug!("connect_pad_added: {}", pad.get_name());

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
        info!("Removing peer {}", peer);
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

                info!("Removed peer {}", peer.peer_id);
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

fn state_name(s: gst::State) -> &'static str {
    match s {
        gst::State::Playing => "PLAYING",
        gst::State::Null => "NULL",
        gst::State::VoidPending => "VOID_PENDING",
        gst::State::Ready => "READY",
        gst::State::Paused => "PAUSED",
        _ => "UNKNOWN",
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

    // Enqueue a message to be sent via websocket
    fn send(&self, message: protocol::Message) -> Result<(), Error> {
        Ok(self
            .send_msg_tx
            .lock()
            .unwrap()
            .unbounded_send(protocol::Envelope {
                from_jid: self.local_id.clone(),
                to_jid: self.peer_id.clone(),
                message,
            })?)
    }

    // Whenever webrtcbin tells us that (re-)negotiation is needed, simply ask
    // for a new offer SDP from webrtcbin without any customization and then
    // asynchronously send it to the peer via the WebSocket connection
    fn on_negotiation_needed(&self) -> Result<(), Error> {
        debug!("on_negotiation_needed: self={}", self.peer_id);

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
                return Err(Error::new_gst(
                    "Offer creation future got no response".to_string(),
                ));
            }
            Err(err) => {
                let msg = format!("Offer creation future got error reponse: {:?}", err);
                return Err(Error::new_gst(msg));
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

        self.send(protocol::Message::SDP {
            type_: "offer".to_string(),
            sdp: offer.get_sdp().as_text().unwrap(),
        })
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
                return Err(Error::new_gst(
                    "Answer creation future got no response".to_string(),
                ));
            }
            Err(err) => {
                let msg = format!("Answer creation future got error reponse: {:?}", err);
                return Err(Error::new_gst(msg));
            }
        };

        let answer = get_answer_from_reply(reply)?;

        self.webrtcbin
            .emit("set-local-description", &[&answer, &None::<gst::Promise>])
            .unwrap();

        let type_ = "answer".to_string();

        let sdp = answer.get_sdp().as_text().unwrap();

        println!("sending SDP {} to peer {}: {}", type_, self.peer_id, sdp);

        self.send(protocol::Message::SDP { type_, sdp })
    }

    // Handle incoming SDP answers from the peer
    fn handle_sdp(&self, type_: &str, sdp: &str) -> Result<(), Error> {
        if type_ == "answer" {
            debug!("Received answer: {}", sdp);
            let ret = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes())
                .map_err(|_| Error::new_proto("Error parsing answer".to_string()))?;

            let answer =
                gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Answer, ret);

            self.webrtcbin
                .emit("set-remote-description", &[&answer, &None::<gst::Promise>])?;

            Ok(())
        } else if type_ == "offer" {
            debug!("Received offer: {}", sdp);
            let ret = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes())
                .map_err(|_| Error::new_proto("Error parsing offer".to_string()))?;

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
            Err(Error::new_proto(format!(
                "Unknown SDP message type: {:?}",
                type_
            )))
        }
    }

    // Handle incoming ICE candidates from the peer by passing them to webrtcbin
    fn handle_ice(&self, sdp_mline_index: u32, candidate: &str) -> Result<(), Error> {
        self.webrtcbin
            .emit("add-ice-candidate", &[&sdp_mline_index, &candidate])?
            .ok_or_else(|| Error::new_gst("can't emit add-ice-candidate".to_string()))?;
        Ok(())
    }

    // Asynchronously send ICE candidates to the peer via the WebSocket connection as a JSON
    // message
    fn on_ice_candidate(&self, sdp_mline_index: u32, candidate: String) -> Result<(), Error> {
        debug!("on_ice_candidate: {}", candidate);
        self.send(protocol::Message::ICE {
            candidate,
            sdp_mline_index,
        })
    }

    // Whenever there's a new incoming, encoded stream from the peer create a new decodebin
    // and audio/video sink depending on the stream type
    fn on_incoming_stream(&self, pad: &gst::Pad) -> Result<(), Error> {
        debug!("on_incoming_stream: {}", pad.get_name());

        // Early return for the source pads we're adding ourselves
        if pad.get_direction() != gst::PadDirection::Src {
            return Ok(());
        }

        let caps = pad.get_current_caps().unwrap();
        let s = caps.get_structure(0).unwrap();
        let media_type = s
            .get::<&str>("media")
            .expect("Invalid type")
            .ok_or_else(|| Error::new_proto(format!("no media type in caps {:?}", caps)))?;

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
            warn!("Unknown pad {:?}, ignoring", pad);
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
        conv.sync_state_with_parent()?;
        pad.link(&sinkpad)?;

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

fn get_answer_from_reply(
    reply: &gst::StructureRef,
) -> Result<gst_webrtc::WebRTCSessionDescription, Error> {
    reply
        .get_value("answer")?
        .get::<gst_webrtc::WebRTCSessionDescription>()
        .expect("Invalid Argument")
        .ok_or_else(|| Error::new_proto("Can't read answer from reply".to_string()))
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
    let (_, framed) = Client::build()
        .connector(connector)
        .finish()
        .ws(&config.http.server)
        .bearer_auth(&token)
        .connect()
        .await
        .map_err(|e| Error::new_io(format!("Fudeu criando o cliente: {}", e)))?;
    Ok(framed)
}

fn load_config(args: &Vec<String>) -> Result<Config, Error> {
    if args.len() != 2 {
        // Can't move on without the configuration file
        Err(Error::new(
            ErrorType::Input,
            format!("Usage: {} CONFIG-FILE", args[0]),
        ))
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

    // Create our application state and lay the pipes for the internal
    // communication between gstreamer and the websocket connection
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
        Err(Error::new_gst(format!("Missing plugins: {:?}", missing)))
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
            message: protocol::Message::PeerCaps(self.capabilities()),
        };
        let json_text = serde_json::to_string(&msg).unwrap();
        self.framed.write(Message::Text(json_text)).unwrap();
        self.hb(ctx)
    }

    fn stopped(&mut self, _: &mut Context<Self>) {
        debug!("Disconnected");
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
