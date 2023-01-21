import { getCookieValue, setCookieValue } from './cookies';

const COOKIE_KEY = "user-token";

export const AuthState = {
  Anonymous:     1 << 1,
  Loading:       1 << 2,
  Authenticated: 1 << 3,
  Failed:        1 << 4,
};

AuthState.fromID = (id) => ({
  [1 << 1]: AuthState.Anonymous,
  [1 << 2]: AuthState.Loading,
  [1 << 3]: AuthState.Authenticated,
  [1 << 4]: AuthState.Failed,
}[id]);

/// Perform authentication and translate return to an AuthState value
export async function login({ jid, password }) {
  const response = await fetch('/api/auth', {
    method: 'post',
    body: JSON.stringify({ jid, password }),
    headers: { 'Content-Type': 'application/json' },
  });
  switch (response.status) {
  case 200:
    console.dir(response);
    //setCookieValue(COOKIE_KEY)
    return { authState: AuthState.Authenticated };
  default:
    const authError = {
      httpStatus: response.status,
      httpString: response.statusText,
    };
    return { authState: AuthState.Failed, authError };
  }
}

/// Get you the auth cookie or undefined
export function getAuthCookie() {
  return getCookieValue(COOKIE_KEY);
}

/// Retrieve initial authentication state
export function getInitialAuthState() {
  return getAuthCookie() === undefined ? AuthState.Anonymous : AuthState.Authenticated;
}
