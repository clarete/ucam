import { useContext } from 'react';
import { appContext, PeerState } from '../context';
import { actions, wsSend } from '../context/reducers';

/// Pulls most useful fields of the application context.  Notice: this
/// is mostly intended to be used by *other hooks* instead of letting
/// the application code itself use it.
export function useAppContext() {
  const { state, dispatch } = useContext(appContext);

  /// Returns true if a client isn't disconnected
  const isClientConnected = jid => {
    return state.wsStateByID[jid] !== undefined;
  };

  /// Returns all clients that aren't disconnected
  const getConnectedClients = () => {
    return Object.keys(state.wsStateByID).filter(jid => isClientConnected(jid));
  };

  /// Sets the state of the peer under JID to connecting
  const connectToClient = jid => {
    dispatch({ type: actions.WRTC_PEER_STATE, jid, state: PeerState.Connecting });
  };

  /// Clear out the state of the peer connection for JID
  const disconnectFromClient = jid => {
    dispatch({ type: actions.WRTC_PEER_STATE, jid, state: undefined });
  };

  const webSocketSend = (message, toJID) => wsSend(state, message, toJID);

  return {
    // state utilities
    state,
    dispatch,
    // websocket state utilities
    getConnectedClients,
    isClientConnected,
    webSocketSend,
    // webrtc state utilities
    connectToClient,
    disconnectFromClient,
  };
}
