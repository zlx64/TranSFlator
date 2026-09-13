// In-memory + localStorage access-token store (D7).
// The token is only ever sent as `Authorization: Bearer <token>` on /api/*
// and as a `?token=` query param on WebSocket upgrades.

const KEY = "transflator.authToken";

export function getToken(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function setToken(token: string | null): void {
  try {
    if (token) localStorage.setItem(KEY, token);
    else localStorage.removeItem(KEY);
  } catch {
    // ignore storage failures (private mode, etc.)
  }
}
