'use client';

import { useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

/**
 * Post-OAuth TOTP challenge. The user lands here with a pending-MFA
 * cookie (mfa=false). Submitting a valid 6-digit code (or a recovery
 * code) flips the cookie to a fully-verified session.
 */
export default function MfaChallengePage() {
  const router = useRouter();
  const { user, status, refresh, logout } = useSession();
  const [code, setCode] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
    if (status === 'authed') router.replace('/');
  }, [status, router]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  async function submit(e: React.FormEvent) {
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
  }

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
            <span className="tilde">~/</span>sys-manager
          </div>
          <div className="muted" style={{ fontSize: 12, fontFamily: 'var(--mono)' }}>
            two-factor challenge {user ? `· ${user}` : ''}
          </div>
        </div>

        <div className="panel">
          <div className="panel-body" style={{ padding: 20 }}>
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
