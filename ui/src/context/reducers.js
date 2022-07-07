import { AuthState } from '../services/auth';

export const actions = {
  AUTH_LOADING: "AUTH_LOADING",
  AUTH_SUCCESS: "AUTH_SUCCESS",
  AUTH_FAILURE: "AUTH_FAILURE",

  ROSTER_LIST:    "ROSTER_LIST",
  ROSTER_ONLINE:  "ROSTER_ONLINE",
  ROSTER_OFFLINE: "ROSTER_OFFLINE",

  WSCK_CONNECT:    "WSCK_CONNECT",
  WSCK_SEND:       "WSCK_SEND",
  WSCK_ON_OPEN:    "WSCK_ON_OPEN",
  WSCK_ON_CLOSE:   "WSCK_ON_CLOSE",
  WSCK_ON_ERROR:   "WSCK_ON_ERROR",
  WSCK_ON_MESSAGE: "WSCK_ON_MESSAGE",

  WRTC_PEER_STATE:      "WRTC_PEER_STATE",
  WRTC_PEER_CONNECTION: "WRTC_PEER_CONNECTION",
};

export function createReducer() {
  return (state, action) => {
    const { type, ...data } = action;

    console.log(`EVENT OF TYPE ${type}`);
    console.dir(data);

    switch (type) {
    case actions.AUTH_LOADING:
      return { ...state, authState: AuthState.Loading };
    case actions.AUTH_SUCCESS:
      return { ...state, ...data };
    case actions.AUTH_FAILURE:
      return { ...state, ...data  };

    case actions.ROSTER_LIST:
      return { ...state, ...data };
    case actions.ROSTER_ONLINE: {
      const newState = { ...state };
      const { capabilities } = data.message.clientonline;
      newState.roster[data.from_jid] = capabilities;
      return newState;
    }
    case actions.ROSTER_OFFLINE: {
      const newState = { ...state };
      if (newState.roster[data.from_jid] !== undefined)
        delete newState.roster[data.from_jid];
      return newState;
    }

    case actions.WSCK_CONNECT: {
      const newState = { ...state };
      newState.ws.current = data.ws;
      return newState;
    }
    case actions.WSCK_SEND:
      wsSend(state, data.message, data.fromJID);
      return state;
    case actions.WSCK_ON_OPEN:
      wsSend(state, data.capabilities);
      return state
    case actions.WSCK_ON_CLOSE:
      return state;
    case actions.WSCK_ON_ERROR:
      return state;
    case actions.WSCK_ON_MESSAGE:
      return state;

    case actions.WRTC_PEER_STATE: {
      const newState = { ...state };
      newState.wsStateByID[data.jid] = data.state;
      return newState;
    }
    case actions.WRTC_PEER_CONNECTION: {
      const newState = { ...state };
      if (newState.wsPeersByID.current === null)
        newState.wsPeersByID.current = {};
      newState.wsPeersByID.current[data.jid] = data.pc;
      return newState;
    }

    default:
      throw new Error(`action type not known: ${type}`);
    }
  }
}

export function wsSend(state, message, toJID) {
  if (state.ws.current === null) {
    console.error(`this is messed up: trying to send message without a reference to a websocket instance`);
    return;
  }

  console.log(`Send message to=${toJID} msg=${JSON.stringify(message)}`);

  state.ws.current.send(JSON.stringify({
    from_jid: state.authJID,
    to_jid: toJID === undefined ? "" : toJID,
    message,
  }));
}
