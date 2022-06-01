import { useAppContext } from './useAppContext'
import { actions } from '../context/reducers';
import { AuthState, login } from '../services/auth';

/// Expose authState and the authentication API to components.
export function useAuthState() {
  const { state: { authState }, dispatch } = useAppContext();

  const auth = async (authFormData) => {
    dispatch({ type: actions.AUTH_LOADING });
    if (await login(authFormData) == AuthState.Authenticated) {
      dispatch({ type: actions.AUTH_SUCCESS });
      return;
    }
    dispatch({ type: actions.AUTH_FAILURE });
  }

  return { authState, auth };
}
