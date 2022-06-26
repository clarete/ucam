use serde_derive::{Deserialize, Serialize};
use std::collections::HashSet;

/// Struct that wraps both header fields and message together
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub struct Envelope {
    /// The JID from who sent the message.  When a message is
    /// originated from the server rather than another client, the JID
    /// will look like a bare dns address.  Which is still valid as a
    /// JID value.
    pub from_jid: String,

    /// The JID of the user receiving this message.
    pub to_jid: String,

    /// The body of the message to be exchanged
    pub message: Message,
}

/// Message is the struct that carries the different types of
/// information clients exchange with the server and other clients
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Message {
    /// Notify peer that a peer in their roster has come online
    PeerOnline { capabilities: HashSet<String> },

    /// Notify that a peer in their roster has gone offline
    PeerOffline,

    /// Send capabilities upon connection to the server.  The server
    /// will keep the capabilities associated with the peer's JID and
    /// send it over to all peers in its roster
    PeerCaps(HashSet<String>),

    /// Request another peer to send you an offer.  Sounds counter
    /// intuitive, but this message allows the Web UI to request the
    /// backend to start a WebRTC negotiation by sending an SDP
    /// message of the type *offer*.
    PeerRequestCall,

    /// Exchange text messages between peers
    PeerChat(String),

    /// Exchange SDP messages between peers
    SDP {
        #[serde(rename = "type")]
        type_: String,
        sdp: String,
    },

    /// Exchange a ICE candidates between local and remote peers
    ICE {
        candidate: String,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: u32,
    },
}
