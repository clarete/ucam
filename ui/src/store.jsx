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
  connectedTo: {},
  peerConnections: {},
  videoElements: {},
  auth: {
    state: AuthState.Anonymous,
    user: null,
  },
};

const store = createContext(initialState);

const createReducer = () => {
  return (state, action) => {
    // Debug what's gonna happen to state
    const { type, ...debug } = action;
    console.log(`STATE: ${type.padEnd(20)}: ${JSON.stringify(debug)}`);

    switch (action.type) {

    // ---- authentication ----

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
      const { api, ws } = action;
      newState.ws = ws;
      api.state = newState;
      return newState;
    }
    case 'srv.userList': {
      const newState = { ...state };
      newState.clientList = action.data;
      return newState;
    }
    case 'srv.clientOnline': {
      const newState = { ...state };
      const { capabilities } = action.data.message.clientonline;
      newState.clientList[action.data.from_jid] = capabilities;
      return newState;
    }
    case 'srv.clientOffline': {
      const newState = { ...state };
      delete newState.clientList[action.data.from_jid];
      return newState;
    }

    // ---- Calls ----

    case 'm.videoEl': {
      const newState = { ...state };
      const { remoteJID, node } = action;
      newState.videoElements[remoteJID] = node;
      return newState
    }

    case 'm.connect': {
      const newState = { ...state };
      const { api, remoteJID } = action.data;
      // newState.peerConnections[remoteJID] = api.createPeerConnection();
      // api.sendOffer(remoteJID);
      // api.addPendingCandidates(remoteJID);
      newState.connectedTo[remoteJID] = 'connecting';
      return newState;
    }

    case 'm.disconnect': {
      const newState = { ...state };
      delete newState.connectedTo[action.data];
      return newState;
    }

    case 'm.peerConn': {
      const newState = { ...state };
      newState.peerConnections[action.jid] = action.peerConn;
      return newState;
    }

    // ---- UI state updates ----

    case 'ui.enableAudioRecording': {
      const newState = { ...state };
      newState.ui.recordAudio = false;
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
  }

  /** Return the the user's JID */
  getJID() {
    const { user } = this.state.auth;
    return user;
  }

  /** Return the bare piece of the user's JID */
  getBareJID() {
    const user = this.getJID();
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

  connectionSettings() {
    return {
      // iceServers: [{ urls: ["stun:stun.l.google.com:19302"] }],
    };
  }

  createPeerConnection(jid) {
    const pc = new RTCPeerConnection(this.connectionSettings());
    pc.onicecandidate = (event) => {
      console.log(`onicecandidate: '${event}'`);
      if (!event.candidate) return;
      this.wsSend({ "newicecandidate": event }, jid);
    };
    pc.onaddstream = (event) => {
      const videoEl = this.state.videoElements[jid];
      console.log(`onaddstream: ${event} ${videoEl}`);
      if (videoEl !== undefined) {
        videoEl.autoplay = true;
        videoEl.srcObject = event.stream;
      }
    };
    console.log(`peer connection created for ${jid}`);
    this.state.peerConnections[jid] = pc;
  }

  sendOffer(remoteJID) {
    console.log(`send offer: ${remoteJID}`);
    this.state.peerConnections[remoteJID]
      .createOffer()
      .then(
        sdp => this.setAndSendLocalDescription(remoteJID, sdp),
        error => { console.error('Send offer failed: ', error); },
      );
  }

  sendAnswer(remoteJID) {
    console.log(`Send answer: ${remoteJID}`);
    this.state.peerConnections[remoteJID]
      .createAnswer()
      .then(
        sdp => this.setAndSendLocalDescription(remoteJID, sdp),
        error => { console.error('Send answer failed: ', error); },
      );
  }

  setAndSendLocalDescription(remoteJID, sdp) {
    this.state.peerConnections[remoteJID].setLocalDescription(sdp);
    console.log(`Local description set for ${remoteJID}`);
    this.wsSend({ calloffer: { sdp } }, remoteJID);
  }

  handleConnectionMessage({ from_jid, message }) {
    const peerConn = this.state.peerConnections[from_jid];

    if (message.callanswer) {
      console.log("WEBRTC: Receive SDP answer", from_jid, message);
      const desc = new RTCSessionDescription(message.callanswer.sdp);
      peerConn
        .setRemoteDescription(desc)
        .catch(e => {
          console.error("Cannot set Remote Description", e);
        });
      return;
    }

    if (message.calloffer) {
      console.log("WEBRTC: Receive SDP answer", from_jid, message);
      this.createPeerConnection(from_jid);
      this.state.peerConnections[from_jid]
        .setRemoteDescription(new RTCSessionDescription(message.calloffer.sdp));
      this.sendAnswer(from_jid);
      return;
    }

    if (message.newicecandidate) {
      const candidate = new RTCIceCandidate(message.newicecandidate.ice);
      console.log("WEBRTC: Receive ICE candidate", from_jid, message);
      peerConn
        .addIceCandidate(candidate)
        .catch(e => { console.error("Cannot add ICE candidate", e); });
      return;
    }
  }

  /** Retrieve the list of currently connected clients & updates the internal state */
  async listClients() {
    const response = await window.fetch('/api/roster');
    const allClients = await response.json();
    delete allClients[this.getBareJID()];
    this.dispatch({ type: 'srv.userList', data: allClients });
    return allClients;
  }

  /** Returns true if we're currently connected to client wih JID */
  isConnectedTo(jid) {
    console.log(`check if ${jid} is connected: ${this.state.connectedTo[jid]}`);
    return this.state.connectedTo[jid] !== undefined;
  }

  /** Return a list of all connected clients.
   *
   * This list includes clients that are still connectING. */
  connectedClients() {
    return Object.keys(this.state.connectedTo);
  }

  saveVideoElementForJID(remoteJID, node) {
    console.log(`watevahhhhhhhhhhhh`);
    this.dispatch({ type: 'm.videoEl', data: { api: this, remoteJID, node } });
  }

  /** Dispatch message to connect to a given client */
  connectTo(remoteJID) {
    this.dispatch({ type: 'm.connect', data: { api: this, remoteJID } });
  }

  /** Dispatch message to disconnect from a given client */
  disconnectFrom(client) {
    this.dispatch({ type: 'm.disconnect', data: client });
  }

  /** Return the URL to connect to the WebSocket */
  webSocketUrl() {
    return 'wss://guinho.home:7070/ws?token=admin@domain.tld';
  }

  /** Sends a messave to the websocket server */
  wsSend(message, toJID="") {
    this.state.ws.send(JSON.stringify({
      from_jid: this.getBareJID(),
      to_jid: toJID,
      message,
    }));
  }

  /** Sends this client's capabilities upon successful connection */
  wsOpen(event) {
    this.wsSend({ capabilities: ['consume:audio', 'consume:video', 'produce:audio'] });
  }

  /** Triggered when the server closes the connection */
  wsClose(event) {
  }

  /** Triggered upon error on the connection */
  wsError(event) {
    console.dir(event);
  }

  /** Event triggered when the server sends this client a message */
  wsMessage(e) {
    if (e.type === "message") {
      const data = JSON.parse(e.data);

      if (data.message.clientonline !== undefined) {
        this.dispatch({ type: 'srv.clientOnline', data });
        return;
      }

      if (data.message === 'clientoffline') {
        this.dispatch({ type: 'srv.clientOffline', data });
        return;
      }

      // Server relaying a message from another client
      this.handleConnectionMessage(data)
    }
  }

  connect() {
    const ws = new window.WebSocket(this.webSocketUrl());
    ws.addEventListener('open', this.wsOpen.bind(this));
    ws.addEventListener('close', this.wsClose.bind(this));
    ws.addEventListener('error', this.wsError.bind(this));
    ws.addEventListener('message', this.wsMessage.bind(this));
    this.dispatch({ type: 'srv.connect', api: this, ws });
  }

  /** Entry point for this client's session */
  async startSession(data) {
    // Issue the authentication request
    await this.auth(data);
    // If we're good, proceed to connecting to the chat server
    if (this.authState() === AuthState.Authenticated)
      this.connect();
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
