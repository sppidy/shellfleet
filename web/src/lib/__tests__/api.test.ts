import { describe, it, expect, beforeEach } from 'vitest';
import { apiFetch } from '../api';

let lastInit: RequestInit | undefined;

beforeEach(() => {
  // Clear any cookies left over from a prior test.
  for (const c of document.cookie.split(';')) {
    const name = c.split('=')[0]?.trim();
    if (name) {
      document.cookie = `${name}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/; Secure`;
    }
  }
  lastInit = undefined;
  globalThis.fetch = ((_input: RequestInfo | URL, init?: RequestInit) => {
    lastInit = init;
    return Promise.resolve(new Response('ok'));
  }) as typeof fetch;
});

function csrfHeader(): string | null {
  return new Headers(lastInit?.headers).get('X-CSRF');
}

describe('apiFetch CSRF / cookie handling', () => {
  it('prefers the production __Host-csrf cookie on mutating methods', async () => {
    document.cookie = '__Host-csrf=production-token; Secure';
    await apiFetch('/api/auth/mfa/verify', { method: 'POST' });
    expect(csrfHeader()).toBe('production-token');
  });

  it('attaches X-CSRF on mutating methods, reading a value that contains "="', async () => {
    document.cookie = 'csrf=abc=123';
    await apiFetch('/api/x', { method: 'POST' });
    // readCookie must rejoin on "=" so a token containing "=" survives.
    expect(csrfHeader()).toBe('abc=123');
  });

  it('does not attach X-CSRF on GET requests', async () => {
    document.cookie = 'csrf=tok';
    await apiFetch('/api/x');
    expect(csrfHeader()).toBeNull();
  });

  it('omits X-CSRF when no csrf cookie is present (even for a mutating method)', async () => {
    await apiFetch('/api/x', { method: 'DELETE' });
    expect(csrfHeader()).toBeNull();
  });

  it('always sends credentials: include', async () => {
    await apiFetch('/api/x');
    expect(lastInit?.credentials).toBe('include');
  });
});
