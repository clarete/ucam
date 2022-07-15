import { PeerState } from './index'
import * as actions from './actions';

export const connect = jid => ({
  jid,
  type: actions.WRTC_PEER_STATE,
  state: PeerState.Connecting,
});

export const disconnect = jid => ({
  jid,
  type: actions.WRTC_PEER_STATE,
  state: undefined,
});

export const peersList = peers => ({
  peers,
  type: actions.PEER_LIST,
});

export const wsSend = (message, fromJID) => ({
  fromJID,
  message,
  type: actions.WSCK_SEND,
});

export const wrtcConnection = (jid, pc) => ({
  jid,
  pc,
  type: actions.WRTC_PEER_CONNECTION,
});
