'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useSession } from '@/components/providers/SessionProvider';
import { apiFetch } from '@/lib/api';
import { Loader2Icon } from 'lucide-react';

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

export default function LoginPage() {
  const { status } = useSession();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    if (status === 'authed') window.location.href = '/';
  }, [status]);

  // Run the discoverable-passkey assertion. `mediation: 'conditional'` is the
  // silent auto path (browser surfaces a passkey if the user has one, does
  // nothing otherwise); `'optional'` is the explicit button (modal picker).
  const startPasskey = useCallback(async (mediation: 'optional' | 'conditional') => {
    if (typeof window === 'undefined' || !window.PublicKeyCredential) return;
    const explicit = mediation === 'optional';
    if (explicit) { setBusy(true); setError(null); }
    try {
      const begin = await apiFetch('/api/auth/passkey/login/begin', { method: 'POST' });
      if (!begin.ok) { if (explicit) setError('passkey login is unavailable'); return; }
      const { state_id, options } = await begin.json();
      const pk = options.publicKey;
      pk.challenge = b64urlToBuf(pk.challenge);
      if (Array.isArray(pk.allowCredentials)) {
        pk.allowCredentials = pk.allowCredentials.map((c: { id: string }) => ({ ...c, id: b64urlToBuf(c.id) }));
      }
      abortRef.current?.abort();
      const ac = new AbortController();
      abortRef.current = ac;
      const getOpts: CredentialRequestOptions = { publicKey: pk, signal: ac.signal };
      // mediation isn't in the lib's CredentialRequestOptions type yet.
      (getOpts as CredentialRequestOptions & { mediation?: string }).mediation = mediation;
      const cred = (await navigator.credentials.get(getOpts)) as PublicKeyCredential | null;
      if (!cred) return;
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
      const finish = await apiFetch('/api/auth/passkey/login/finish', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ state_id, credential }),
      });
      if (!finish.ok) {
        if (explicit) setError((await finish.text().catch(() => '')) || 'passkey login failed');
        return;
      }
      window.location.href = '/';
    } catch (e) {
      const m = e instanceof Error ? e.message : '';
      // Ignore cancel/abort (and all conditional-flow errors — it's best-effort).
      if (explicit && !/NotAllowed|abort|cancel/i.test(m)) setError(m || 'passkey login failed');
    } finally {
      if (explicit) setBusy(false);
    }
  }, []);

  // Auto-offer a passkey on load where the browser supports conditional UI.
  useEffect(() => {
    if (status !== 'guest') return;
    let active = true;
    (async () => {
      try {
        const pkc = (window as unknown as { PublicKeyCredential?: { isConditionalMediationAvailable?: () => Promise<boolean> } }).PublicKeyCredential;
        if (pkc?.isConditionalMediationAvailable && (await pkc.isConditionalMediationAvailable())) {
          if (active) startPasskey('conditional');
        }
      } catch {
        /* no conditional UI — the button still works */
      }
    })();
    return () => {
      active = false;
      abortRef.current?.abort();
    };
  }, [status, startPasskey]);

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
      {/* Anchor for conditional-UI passkey autofill. */}
      <input
        type="text"
        autoComplete="webauthn"
        tabIndex={-1}
        aria-hidden="true"
        style={{ position: 'absolute', opacity: 0, height: 0, width: 0, pointerEvents: 'none' }}
      />
      <div style={{ width: 'min(420px, 92vw)' }}>
        <div style={{ textAlign: 'center', marginBottom: 24 }}>
          <div className="brand-name" style={{ fontSize: 24, fontWeight: 700, marginBottom: 8 }}>
            <span className="tilde">~/</span>shellfleet
          </div>
          <div className="muted" style={{ fontSize: 12, fontFamily: 'var(--mono)' }}>
            sign in with a passkey, or the GitHub account on the allowlist
          </div>
        </div>

        <div className="panel">
          <div className="panel-body" style={{ padding: 20 }}>
            {status === 'loading' ? (
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 12 }}>
                <Loader2Icon className="w-5 h-5 animate-spin" style={{ color: 'var(--fg-2)' }} />
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                <button
                  type="button"
                  className="btn primary"
                  onClick={() => startPasskey('optional')}
                  disabled={busy}
                  style={{ width: '100%', height: 36, justifyContent: 'center', fontSize: 13, gap: 8 }}
                >
                  {busy ? 'waiting for passkey…' : '⚷ sign in with a passkey'}
                </button>

                {error && (
                  <div className="mono" style={{ color: 'var(--err)', fontSize: 12 }}>
                    {error}
                  </div>
                )}

                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    color: 'var(--fg-2)',
                    fontSize: 11,
                    fontFamily: 'var(--mono)',
                  }}
                >
                  <div style={{ flex: 1, height: 1, background: 'var(--bd)' }} />
                  or
                  <div style={{ flex: 1, height: 1, background: 'var(--bd)' }} />
                </div>

                <a
                  href="/auth/login"
                  className="btn"
                  style={{ width: '100%', height: 36, justifyContent: 'center', fontSize: 13 }}
                >
                  continue with GitHub
                </a>
              </div>
            )}
          </div>
        </div>

        <div className="kbd-hint" style={{ textAlign: 'center', marginTop: 20, fontSize: 11 }}>
          sessions last 24 hours.
        </div>
      </div>
    </div>
  );
}
