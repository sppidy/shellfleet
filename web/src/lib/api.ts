/**
 * Wrapper around `fetch` that automatically attaches the CSRF token
 * for mutating methods. The token lives in a `csrf` cookie set by the
 * server's middleware on first contact; we read it back and echo it
 * via the `X-CSRF` header (double-submit pattern).
 *
 * Always sends `credentials: include` so the auth cookie + csrf cookie
 * actually get attached.
 */
const MUTATING = new Set(['POST', 'PUT', 'DELETE', 'PATCH']);

function readCookie(name: string): string | null {
  if (typeof document === 'undefined') return null;
  for (const part of document.cookie.split(';')) {
    const [k, ...rest] = part.trim().split('=');
    if (k === name) return rest.join('=');
  }
  return null;
}

export async function apiFetch(
  input: RequestInfo | URL,
  init: RequestInit = {},
): Promise<Response> {
  const method = (init.method ?? 'GET').toUpperCase();
  const headers = new Headers(init.headers);
  if (MUTATING.has(method)) {
    // Production uses the host-only cookie name required by the Secure
    // deployment; local HTTP development retains the historical name.
    const token = readCookie('__Host-csrf') ?? readCookie('csrf');
    if (token) headers.set('X-CSRF', token);
  }
  return fetch(input, {
    ...init,
    credentials: init.credentials ?? 'include',
    headers,
  });
}
