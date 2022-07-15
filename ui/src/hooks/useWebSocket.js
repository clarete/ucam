import { useContext, useEffect, useRef } from 'react';
import { appContext } from './../context';
import * as actions from '../context/actions';
import * as messages from '../context/messages';

export function useWebSocket() {
  const { state, dispatch } = useContext(appContext);
  const address = webSocketUrl(state);
  const webSocket = useRef(null);

  useEffect(() => {
    // create the websocket object and send store its ref in the state
    const ws = webSocket.current = new WebSocket(address);
    // don't say we can send media for now
    const peercaps = ['consume:audio', 'consume:video'];
    ws.addEventListener('open', event => {
      dispatch(messages.wsSend({ peercaps }, state.authJID));
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
        dispatch({ type: actions.PEER_ONLINE, ...data });
        return;
      }

      if (message === 'clientoffline') {
        dispatch({ type: actions.PEER_OFFLINE, ...data });
        return;
      }

      // handle messages related to the call flow

      const wsPeer = state.wsPeersByID?.current[fromJID];

      if (message.ice !== undefined) {
        console.log("WEBRTC: Receive ICE candidate", fromJID, message);
        wsPeer
          .addIceCandidate(new RTCIceCandidate(message.ice))
          .catch(e => { console.error("Cannot add ICE candidate", e); });
        return;
      }

      if (message.sdp !== undefined) {
        console.log(`WEBRTC: Receive SDP ${fromJID} ${message.sdp.type}`);
        switch (message.sdp.type) {
        case "offer":
          wsPeer
            .setRemoteDescription(new RTCSessionDescription(message.sdp))
            .then(() => wsPeer.createAnswer())
            .then(sdp => {
              wsPeer.setLocalDescription(sdp);
              return sdp;
            })
            .then(sdp => dispatch(messages.wsSend({ sdp }, fromJID)));
          return;

        case "answer":
          wsPeer
            .setRemoteDescription(new RTCSessionDescription(message.sdp))
            .catch(e => { console.error("Cannot set Remote Description", e); });
          return;

        default:
          console.error(`Not sure what to do dawg: ${message.sdp.type}`);
        }
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
