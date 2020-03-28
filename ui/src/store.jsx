import React, { createContext, useReducer } from 'react';

const AuthState = {
  Anonymous:     1 << 1,
  Loading:       1 << 2,
  TokenSent:     1 << 3,
  Authenticated: 1 << 4,
  Unauthorized:  1 << 5,
};

const initialState = {
  ws: null,
  clientList: {},
  connectedTo: [],
  auth: {
    state: AuthState.Anonymous,
    user: null,
  },
};

const store = createContext(initialState);

const createReducer = () => {
  return (state, action) => {
    switch (action.type) {

    case 'auth.state': {
      const newState = { ...state };
      newState.auth.state = action.data;
      return newState;
    }

    case 'auth.success': {
      const newState = { ...state };
      newState.auth.state = AuthState.Authenticated;
      newState.auth.user = action.jid;
      return newState;
    }

    // ---- connection with server ----

    case 'srv.connect': {
      const newState = { ...state };
      const { ws, api } = action;
      newState.ws = ws;
      api.newState(newState);
      api.bindWsEvents(ws);
      return newState;
    }
    case 'srv.userList': {
      const newState = { ...state };
      newState.clientList = action.data;
      return newState;
    }
    case 'srv.userConnected': {
      const newState = { ...state };
      const { jid, caps } = action.data;
      newState.clientList[jid] = caps;
      return newState;
    }
    case 'srv.userDisconnected': {
      const newState = { ...state };
      delete newState.clientList[action.data.jid];
      return newState;
    }

    // ---- Calls ----

    case 'm.connect': {
      const newState = { ...state };
      newState.connectedTo.push(action.data);
      return newState;
    }

    case 'm.disconnect': {
      const newState = { ...state };
      newState.connectedTo = newState.connectedTo.filter(c => c !== action.data);
      return newState;
    }

    default:
      throw new Error(`No action ${action.type}`);
    };
  };
};

// Generate a unique ID on this browser that is used to distinguish
// between different devices of the same user.
const rnd = new Uint32Array(1); window.crypto.getRandomValues(rnd);
const resource = rnd.join('');

/** The public API for this storage layer */
class API {
  constructor (state, dispatch) {
    this.state = state;
    this.dispatch = dispatch;
  }

  /** Retrieve user authentication state */
  authState() {
    return this.state.auth.state;
  };

  /** Return the bare piece of the user's JID */
  getBareJID() {
    const { user } = this.state.auth;
    if (user) {
      const [bareJid, ] = user.split('/');
      return bareJid;
    }
    return user;
  }

  /** Authenticate a user */
  async auth(data) {
    // Update UI to show loading spinner
    this.dispatch({ type: 'auth.state', data: AuthState.Loading });

    // Build the full user's JID.  That'd allow the same user to
    // access the system from different devices.
    const jid = `${data.email}/${resource}`;

    // Actual auth
    const response = await fetch('/api/auth', {
      method: 'post',
      body: JSON.stringify({ jid }),
      headers: { 'Content-Type': 'application/json' },
    });

    // Update UI to either show the result of authentication
    const statusToState = {
      200: AuthState.Authenticated,
      400: AuthState.Failed,
      401: AuthState.Unauthorized,
    };
    const authState = statusToState[response.status];
    if (authState === AuthState.Authenticated)
      this.dispatch({ type: 'auth.success', jid });
    else
      this.dispatch({ type: 'auth.state', data: authState });
    return authState;
  }

  /** Retrieve the list of currently connected clients & updates the internal state */
  async listClients() {
    const response = await window.fetch('/api/clients');
    const allClients = await response.json();
    delete allClients[this.getBareJID()];
    this.dispatch({ type: 'srv.userList', data: allClients });
    return allClients;
  }

  /** Return true if this client is connected to any other client */
  isConnectedToSomeone() {
    return this.state.connectedTo.length > 0;
  }

  /** Returns true if we're currently connected to client wih JID */
  isConnectedTo(jid) {
    return this.state.connectedTo.includes(jid);
  }

  /** Return a list of all connected clients.
   *
   * This list includes clients that are still connectING. */
  connectedClients() {
    return this.state.connectedTo;
  }

  /** Dispatch message to connect to a given client */
  connectTo(client) {
    this.dispatch({ type: 'm.connect', data: client });
  }

  /** Dispatch message to disconnect from a given client */
  disconnectFrom(client) {
    this.dispatch({ type: 'm.disconnect', data: client });
  }

  /** Return the URL to connect to the WebSocket */
  webSocketUrl() {
    return 'ws://localhost:8080/ws?token=admin@domain.tld';
  }

  /** Sends a messave to the websocket server */
  wsSend(message) {
    this.state.ws.send(JSON.stringify(message));
  }

  /** Sends this client's capabilities upon successful connection */
  wsOpen(event) {
    this.wsSend({ caps: ['r:audio', 'r:video', 's:audio'] });
  }

  /** Triggered when the server closes the connection */
  wsClose(event) {
  }

  /** Triggered upon error on the connection */
  wsError(event) {
  }

  /** Event triggered when the server sends this client a message */
  wsMessage(event) {
    if (event.type === "message") {
      const { action, ...data } = JSON.parse(event.data);
      switch (action) {
      case 'connected':
        this.dispatch({ type: 'srv.userConnected', data });
        break;
      case 'disconnected':
        this.dispatch({ type: 'srv.userDisconnected', data });
        break;
      }
    }
  }

  /** Connect to the WebSocket server & bind event callbacks */
  connect() {
    this.state.ws = new window.WebSocket(this.webSocketUrl());
    this.state.ws.addEventListener('open', this.wsOpen.bind(this));
    this.state.ws.addEventListener('error', this.wsError.bind(this));
    this.state.ws.addEventListener('message', this.wsMessage.bind(this));
    this.state.ws.addEventListener('close', this.wsClose.bind(this));
  }

  /** Entry point for this client's session */
  async startSession(data) {
    // Issue the authentication request
    await this.auth(data);
    // If we're good, proceed to connecting to the chat server
    if (this.authState() === AuthState.Authenticated)
      await this.connect();
  }
}

const Provider = ({ children }) => {
  const { Provider } = store;
  const memoizedReducer = React.useCallback(createReducer(), []);
  const [state, dispatch] = useReducer(memoizedReducer, initialState);
  const api = new API(state, dispatch);
  return (<Provider value={{ state, dispatch, api }}>{children}</Provider>);
};

export { store, Provider, AuthState };
