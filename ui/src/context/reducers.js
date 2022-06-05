import { AuthState } from '../services/auth';
import { newWebSocketState } from '../services/websocket';

export const actions = {
  AUTH_LOADING: "AUTH_LOADING",
  AUTH_SUCCESS: "AUTH_SUCCESS",
  AUTH_FAILURE: "AUTH_FAILURE",

  ROSTER_LIST:    "ROSTER_LIST",
  ROSTER_ONLINE:  "ROSTER_ONLINE",
  ROSTER_OFFLINE: "ROSTER_OFFLINE",

  WSCK_CONNECT:    "WSCK_CONNECT",
  WSCK_ON_OPEN:    "WSCK_ON_OPEN",
  WSCK_ON_CLOSE:   "WSCK_ON_CLOSE",
  WSCK_ON_ERROR:   "WSCK_ON_ERROR",
  WSCK_ON_MESSAGE: "WSCK_ON_MESSAGE",
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

    case actions.WSCK_CONNECT:
      return { ...state, webSocket: data.webSocket };

    case actions.WSCK_ON_OPEN:
      return state

    case actions.WSCK_ON_CLOSE:
      return state;

    case actions.WSCK_ON_ERROR:
      return state;

    case actions.WSCK_ON_MESSAGE:
      return state;

    default:
      throw new Error(`action type not known: ${type}`);
    }
  }
}
