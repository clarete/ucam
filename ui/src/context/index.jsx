import { createContext, useReducer } from 'react';
import { AuthState } from '../services/auth';
import { createReducer } from './reducers';

/// Values the application starts with.  Will be overrided by other
/// storage types, like cookies for authentication tokens and local or
/// session storage for application data.
const initialState = {
  webSocket: null,
  authState: AuthState.Anonymous,
  peersByID: {},
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
