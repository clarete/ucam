const TOKEN_SESSION_KEY = "user-token"

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
    body: JSON.stringify({ jid }),
    headers: { 'Content-Type': 'application/json' },
  });
  switch (response.status) {
  case 200:
    return { authState: AuthState.Authenticated };
  default:
    const authError = {
      httpStatus: response.status,
      httpString: response.statusText,
    };
    return { authState: AuthState.Failed, authError };
  }
}
