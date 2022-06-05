import { useEffect, useRef } from 'react';
import { useAppContext } from './useAppContext';
import { actions } from '../context/reducers';

export function useWebSocket() {
  const { state, dispatch } = useAppContext();
  const address = webSocketUrl(state);
  const webSocket = useRef(null);

  useEffect(() => {
    // create the websocket object and send store its ref in the state
    const ws = webSocket.current = new WebSocket(address);
    // don't say we can send media for now
    const capabilities = ['consume:audio', 'consume:video'];
    ws.addEventListener('open', event => send({ capabilities }, state.authJID));
    ws.addEventListener('close', event => dispatch({ type: actions.WSCK_ON_CLOSE }));
    ws.addEventListener('error', error => dispatch({ type: actions.WSCK_ON_ERROR, error }));
    ws.addEventListener('message', makeMessageHandler(dispatch));

    // the hook's clean up function disconnects the websocket.  This
    // is done with the enclosed ws variable instead of the reference
    return () => ws.current.close();
  }, []);

  const send = (message, fromJID, toJID) => {
    if (webSocket.current === null) {
      console.error(`this is messed up: trying to send message without a reference to a websocket instance`);
      return;
    }
    webSocket.current.send(JSON.stringify({
      from_jid: fromJID,
      to_jid: toJID === undefined ? "" : toJID,
      message,
    }));
  }

  return { send }
}

function makeMessageHandler(dispatch) {
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

      if (message.callanswer !== undefined) {
        console.log("WEBRTC: Receive SDP answer", fromJID, message);
        const desc = new RTCSessionDescription(message.callanswer.sdp);
        // peerConn
        //   .setRemoteDescription(desc)
        //   .catch(e => { console.error("Cannot set Remote Description", e); });
        return;
      }

      if (message.calloffer !== undefined) {
        console.log("WEBRTC: Receive SDP answer", fromJID, message);
        // this.createPeerConnection(fromJID);
        // this.state.peerConnections[fromJID]
        //   .setRemoteDescription(new RTCSessionDescription(message.calloffer.sdp));
        // this.sendAnswer(fromJID);
        return;
      }

      if (message.newicecandidate !== undefined) {
        const candidate = new RTCIceCandidate(message.newicecandidate.ice);
        console.log("WEBRTC: Receive ICE candidate", fromJID, message);
        // peerConn
        //   .addIceCandidate(candidate)
        //   .catch(e => { console.error("Cannot add ICE candidate", e); });
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
