'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

/**
 * Post-OAuth second-factor challenge. The user lands here with a pending-MFA
 * cookie (mfa=false). Either a valid TOTP/recovery code OR a registered passkey
 * upgrades the cookie to a fully-verified session — a passkey is itself a strong
 * factor, so using one bypasses the TOTP code entirely.
 */

// base64url <-> ArrayBuffer (webauthn-rs uses URL-safe base64 without padding).
function b64urlToBuf(s: string): ArrayBuffer {
  const pad = s.length % 4 === 0 ? '' : '='.repeat(4 - (s.length % 4));
  const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + pad);
  const u = new Uint8Array(b.length);
  for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i);
  return u.buffer;
}
function bufToB64url(buf: ArrayBuffer): string {
  const u = new Uint8Array(buf);
  let s = '';
  for (let i = 0; i < u.length; i++) s += String.fromCharCode(u[i]);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export default function MfaChallengePage() {
  const router = useRouter();
  const { user, status, refresh, logout } = useSession();
  const [code, setCode] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [passkeyAvailable, setPasskeyAvailable] = useState(false);
  const [passkeyBusy, setPasskeyBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
    if (status === 'authed') router.replace('/');
  }, [status, router]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Advertise the passkey option only if this user actually has one registered
  // and WebAuthn is supported by the browser.
  useEffect(() => {
    if (status !== 'pending_mfa' || typeof window === 'undefined' || !window.PublicKeyCredential) return;
    let cancelled = false;
    (async () => {
      try {
        const res = await apiFetch('/api/auth/passkey/available');
        if (!res.ok) return;
        const data = await res.json();
        if (!cancelled) setPasskeyAvailable(!!data.available);
      } catch {
        /* leave the option hidden */
      }
    })();
    return () => { cancelled = true; };
  }, [status]);

  const submit = useCallback(async (e: React.FormEvent) => {
    e.preventDefault();
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const res = await apiFetch('/api/auth/mfa/verify', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ code: code.trim() }),
      });
      if (!res.ok) {
        const text = await res.text().catch(() => 'invalid code');
        throw new Error(text || 'invalid code');
      }
      refresh();
      router.replace('/');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'failed');
      setCode('');
      setTimeout(() => inputRef.current?.focus(), 0);
    } finally {
      setSubmitting(false);
    }
  }, [submitting, code, refresh, router]);

  const authPasskey = useCallback(async () => {
    if (passkeyBusy) return;
    setPasskeyBusy(true);
    setError(null);
    try {
      const begin = await apiFetch('/api/auth/passkey/begin', { method: 'POST' });
      if (!begin.ok) throw new Error((await begin.text().catch(() => '')) || `begin: HTTP ${begin.status}`);
      const { state_id, options } = await begin.json();
      const pk = options.publicKey;
      pk.challenge = b64urlToBuf(pk.challenge);
      if (Array.isArray(pk.allowCredentials)) {
        pk.allowCredentials = pk.allowCredentials.map((c: { id: string }) => ({ ...c, id: b64urlToBuf(c.id) }));
      }
      const cred = (await navigator.credentials.get({ publicKey: pk })) as PublicKeyCredential | null;
      if (!cred) throw new Error('passkey prompt cancelled');
      const asr = cred.response as AuthenticatorAssertionResponse;
      const credential = {
        id: cred.id,
        rawId: bufToB64url(cred.rawId),
        type: cred.type,
        response: {
          authenticatorData: bufToB64url(asr.authenticatorData),
          clientDataJSON: bufToB64url(asr.clientDataJSON),
          signature: bufToB64url(asr.signature),
          userHandle: asr.userHandle ? bufToB64url(asr.userHandle) : null,
        },
        extensions: cred.getClientExtensionResults(),
      };
      const finish = await apiFetch('/api/auth/passkey/finish', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ state_id, credential }),
      });
      if (!finish.ok) throw new Error((await finish.text().catch(() => '')) || 'passkey verification failed');
      refresh();
      router.replace('/');
    } catch (err) {
      // NotAllowedError = user cancelled / timed out; keep it gentle.
      const m = err instanceof Error ? err.message : 'passkey failed';
      setError(/NotAllowed|cancel/i.test(m) ? 'passkey prompt cancelled' : m);
    } finally {
      setPasskeyBusy(false);
    }
  }, [passkeyBusy, refresh, router]);

  if (status === 'loading') {
    return (
      <div className="center-screen">
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  return (
    <div
      style={{
        minHeight: '100vh',
        width: '100%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 16,
        background: 'var(--bg)',
      }}
    >
      <div style={{ width: 'min(420px, 92vw)' }}>
        <div style={{ textAlign: 'center', marginBottom: 24 }}>
          <div
            className="brand-name"
            style={{ fontSize: 24, fontWeight: 700, marginBottom: 8 }}
          >
            <span className="tilde">~/</span>shellfleet
          </div>
          <div className="muted" style={{ fontSize: 12, fontFamily: 'var(--mono)' }}>
            two-factor challenge {user ? `· ${user}` : ''}
          </div>
        </div>

        <div className="panel">
          <div className="panel-body" style={{ padding: 20 }}>
            {passkeyAvailable && (
              <>
                <button
                  type="button"
                  className="btn primary"
                  onClick={authPasskey}
                  disabled={passkeyBusy || submitting}
                  style={{ width: '100%', height: 36, justifyContent: 'center', fontSize: 13, gap: 8 }}
                >
                  {passkeyBusy ? 'waiting for passkey…' : '⚷ sign in with a passkey'}
                </button>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    margin: '14px 0',
                    color: 'var(--fg-2)',
                    fontSize: 11,
                    fontFamily: 'var(--mono)',
                  }}
                >
                  <div style={{ flex: 1, height: 1, background: 'var(--bd)' }} />
                  or enter a code
                  <div style={{ flex: 1, height: 1, background: 'var(--bd)' }} />
                </div>
              </>
            )}

            <form onSubmit={submit} style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              <label
                htmlFor="totp-code"
                className="muted"
                style={{ fontSize: 12, fontFamily: 'var(--mono)' }}
              >
                enter the 6-digit code from your authenticator app, or a recovery code:
              </label>
              <input
                ref={inputRef}
                id="totp-code"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder="123 456"
                autoComplete="one-time-code"
                inputMode="numeric"
                spellCheck={false}
                disabled={submitting}
                style={{
                  fontFamily: 'var(--mono)',
                  fontSize: 18,
                  letterSpacing: 2,
                  textAlign: 'center',
                  padding: '10px 12px',
                  background: 'var(--bg-1)',
                  border: '1px solid var(--bd)',
                  color: 'var(--fg)',
                  borderRadius: 4,
                }}
              />
              {error && (
                <div className="mono" style={{ color: 'var(--err)', fontSize: 12 }}>
                  {error}
                </div>
              )}
              <button
                type="submit"
                className="btn primary"
                disabled={submitting || !code.trim()}
                style={{ width: '100%', height: 36, justifyContent: 'center', fontSize: 13 }}
              >
                {submitting ? 'verifying…' : 'verify'}
              </button>
            </form>
          </div>
        </div>

        <div className="kbd-hint" style={{ textAlign: 'center', marginTop: 20, fontSize: 11 }}>
          <button
            type="button"
            className="btn"
            onClick={logout}
            style={{ height: 'auto', padding: '0 4px' }}
          >
            cancel — sign out
          </button>
        </div>
      </div>
    </div>
  );
}
