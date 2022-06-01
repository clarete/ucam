const TOKEN_SESSION_KEY = "user-token"

export const AuthState = {
  Anonymous:     1 << 1,
  Loading:       1 << 2,
  TokenSent:     1 << 3,
  Authenticated: 1 << 4,
  Unauthorized:  1 << 5,
};

/// Perform authentication and translate return to an AuthState value
export async function login({ jid, password }) {
  const response = await fetch('/api/auth', {
    method: 'post',
    body: JSON.stringify({ jid }),
    headers: { 'Content-Type': 'application/json' },
  });
  switch (response.status) {
  case 200:
    return AuthState.Authenticated;
  case 401:
    return AuthState.Unauthorized;
  default:
    return AuthState.Failed;
  }
}
