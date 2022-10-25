import { createContext, useReducer, createRef } from 'react';
import { getInitialAuthState } from '../services/auth';
import { createReducer } from './reducers';

/// State of webrtc connections.  There is no need to describe a value
/// for the `Disconnected' state, as this is implied when a given JID
/// isn't present in the mapping to peer states within `wsStateById'.
export const PeerState = {
  Connecting: 1 << 1,
  Connected:  1 << 2,
};

/// Values the application starts with.  Will be overrided by other
/// storage types, like cookies for authentication tokens and local or
/// session storage for application data.
const initialState = {
  /// where are we in the authentication process
  authState: getInitialAuthState(),
  /// struct with both http status and http status text of the
  /// authentication call
  authError: null,
  /// authentication token acquired after an API call to the backend
  authToken: null,
  /// the JID of the local user
  authJID: null,
  /// map from JID's to array of strings with client capabilities
  peers: {},
  /// field that will hold the WebSocket instance
  ws: createRef(),
  /// map of JID's to RTCPeerConnection instances
  wsPeersByID: createRef(),
  /// map of JID's to PeerState entries.  Being `undefined' means
  // being disconnected
  wsStateByID: {},
};

/// Where globally accessible data of the application is kept
export const appContext = createContext(initialState);

/// Provider for wrapping application with
export function ContextProvider({ children }) {
  const { Provider } = appContext;
  const memoizedReducer = React.useCallback(createReducer(), []);
  const [state, dispatch] = useReducer(memoizedReducer, initialState);
  return <Provider value={{ state, dispatch }}>{children}</Provider>;
}
