import React, { createContext, useReducer } from 'react';

const AuthStatus = {
  Anonymous:     1 << 1,
  Loading:       1 << 2,
  Authenticated: 1 << 3,
};

const initialState = {
  auth: {
    state: AuthStatus.Anonymous,
    user: null,
  },
};

const store = createContext(initialState);

const createReducer = () => {
  return (state, action) => {
    switch (action.type) {
    case 'auth':
      const newState = { ...state };
      newState.auth.state = AuthStatus.Loading;
      return newState;
    default:
      throw new Error(`No action ${action.type}`);
    };
  };
};

const Provider = ({ children }) => {
  const { Provider } = store;
  const memoizedReducer = React.useCallback(createReducer(), []);
  const [state, dispatch] = useReducer(memoizedReducer, initialState);
  return (<Provider value={{ state, dispatch }}>{children}</Provider>);
};

export { store, Provider, AuthStatus };
