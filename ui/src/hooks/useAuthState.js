import { useContext } from 'react';
import { AuthState, login } from '../services/auth';
import { appContext } from '../context';
import * as actions from '../context/actions';

/// Expose authState and the authentication API to components.
export function useAuthState() {
  const { state: { authState, authError }, dispatch } = useContext(appContext);

  const auth = async (authFormData) => {
    dispatch({ type: actions.AUTH_LOADING });
    const { authState, authError } = await login(authFormData);

    if (authState == AuthState.Authenticated) {
      const { jid: authJID } = authFormData;
      const authToken = authJID;
      dispatch({ type: actions.AUTH_SUCCESS, authState, authJID, authToken });
      return;
    }

    dispatch({ type: actions.AUTH_FAILURE, authState, authError });
  }

  return { authState, authError, auth };
}
