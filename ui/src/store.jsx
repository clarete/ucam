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

    default:
      throw new Error(`No action ${action.type}`);
    };
  };
};

// Generate a unique ID on this browser that is used to distinguish
// between different devices of the same user.
const rnd = new Uint32Array(1); window.crypto.getRandomValues(rnd);
const resource = rnd.join('');

/** The public API for this storage layer.  */
const buildAPI = (state, dispatch) => ({
  /** Retrieve user authentication state */
  authState: () => state.auth.state,

  /** Authenticate a user */
  auth: async (data) => {
    // Update UI to show loading spinner
    dispatch({ type: 'auth.state', data: AuthState.Loading });

    // Build the full user's JID.  That'd allow the same user to
    // access the system from different devices.
    const jid = `${data.email}/${resource}`;

    // Actual auth
    const response = await fetch('/b/auth', {
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
    dispatch({
      type: 'auth.state',
      data: statusToState[response.status],
    });
  },
});

const Provider = ({ children }) => {
  const { Provider } = store;
  const memoizedReducer = React.useCallback(createReducer(), []);
  const [state, dispatch] = useReducer(memoizedReducer, initialState);
  const api = buildAPI(state, dispatch);
  return (<Provider value={{ state, dispatch, api }}>{children}</Provider>);
};

export { store, Provider, AuthState };
