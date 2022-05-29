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

/// Struct with SDP fields
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub struct SDP {
    #[serde(rename = "type")]
    pub type_: String,
    pub sdp: String,
}

/// Message is the struct that carries the different types of
/// information clients exchange with the server and other clients
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Message {
    /// Notify client that a known JID has come online
    ClientOnline { capabilities: HashSet<String> },

    /// Notify that a known JID has gone offline
    ClientOffline,

    /// Send a call offer to the remote peer
    CallOffer { sdp: SDP },

    /// Send the local description to the remote peer when answering
    /// to a call started with `CallOffer`.
    CallAnswer { sdp: SDP },

    /// Tell the remote peer a video session is going down
    HangUp,

    /// Exchange a ICE candidates between local and remote peers
    NewIceCandidate {
        candidate: String,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: u32,
    },

    /// Exchange text messages between local and remote peers
    Chat(String),

    /// Send the list of capabilities the client has available to the server
    Capabilities(HashSet<String>),
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
