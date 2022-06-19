import { useEffect, useRef } from 'react';
import { useAppContext } from './useAppContext';
import { actions } from '../context/reducers';

export function useWebSocket() {
  const { state, dispatch, webSocketSend } = useAppContext();
  const address = webSocketUrl(state);
  const webSocket = useRef(null);

  useEffect(() => {
    // create the websocket object and send store its ref in the state
    const ws = webSocket.current = new WebSocket(address);
    // don't say we can send media for now
    const capabilities = ['consume:audio', 'consume:video'];
    ws.addEventListener('open', event => {
      webSocketSend(state.authJID, { capabilities });
    });
    ws.addEventListener('close', event => dispatch({ type: actions.WSCK_ON_CLOSE }));
    ws.addEventListener('error', error => dispatch({ type: actions.WSCK_ON_ERROR, error }));
    ws.addEventListener('message', makeMessageHandler(state, dispatch));

    // store this instance in the context to allow us to send messages
    // from other components too
    dispatch({ type: actions.WSCK_CONNECT, ws });

    // the hook's clean up function disconnects the websocket
    return () => ws.close();
  }, []);
}

function makeMessageHandler(state, dispatch) {
  return (event) => {
    if (event.type === "message") {
      const data = JSON.parse(event.data);
      const { from_jid: fromJID, message } = data;

      if (message.clientonline !== undefined) {
        dispatch({ type: actions.ROSTER_ONLINE, ...data });
        return;
      }

      if (message === 'clientoffline') {
        dispatch({ type: actions.ROSTER_OFFLINE, ...data });
        return;
      }

      // handle messages related to the call flow

      const wsPeer = state.wsPeersByID?.current[fromJID];

      if (message.newicecandidate !== undefined) {
        console.log("WEBRTC: Receive ICE candidate", fromJID, message);
        wsPeer
          .addIceCandidate(new RTCIceCandidate(message.newicecandidate))
          .catch(e => { console.error("Cannot add ICE candidate", e); });
        return;
      }

      if (message.callanswer !== undefined) {
        console.log("WEBRTC: Receive SDP answer", fromJID, message);
        wsPeer
          .setRemoteDescription(new RTCSessionDescription(message.callanswer.sdp))
          .catch(e => { console.error("Cannot set Remote Description", e); });
        return;
      }

      if (message.calloffer !== undefined) {
        console.log("WEBRTC: Receive SDP offer", fromJID, message);
        wsPeer
          .setRemoteDescription(new RTCSessionDescription(message.calloffer.sdp))
          .then(() => wsPeer.createAnswer())
          .then(answer => wsPeer.setLocalDescription(answer))
          .then(answer => dispatch({ type: actions.WSCK_SEND, fromJID, message }));
        return;
      }
    }
  }
}

function webSocketUrl(state) {
  // TODO: might need to use something like `state.authToken' here,
  // that's why we're taking the state as a parameter.  Might not be
  // needed once the auth token is stored in the browser's cookie jar.
  return 'wss://guinho.home:7070/ws?token=admin@domain.tld';
}
