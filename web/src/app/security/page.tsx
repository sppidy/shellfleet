'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { QRCodeSVG } from 'qrcode.react';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

interface StartResponse {
  secret: string;
  otpauth_uri: string;
  recovery_codes: string[];
}

type Stage = 'idle' | 'enrolling' | 'enrolled' | 'disabling';

/**
 * /security — TOTP enrollment + disable.
 *
 * Enrollment flow:
 *  1. POST /api/auth/mfa/start — fetch a candidate secret + recovery codes
 *  2. User scans the otpauth URI (rendered as QR + as a copyable code)
 *  3. User types the current 6-digit code
 *  4. POST /api/auth/mfa/confirm — server verifies the code and persists
 *     the secret + hashed recovery codes
 */
export default function SecurityPage() {
  const router = useRouter();
  const { user, role, mfaEnabled, status, refresh } = useSession();
  const [stage, setStage] = useState<Stage>('idle');
  const [start, setStart] = useState<StartResponse | null>(null);
  const [code, setCode] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
    if (status === 'pending_mfa') router.replace('/mfa');
  }, [status, router]);

  const beginEnroll = useCallback(async () => {
    setError(null);
    setBusy(true);
    try {
      const res = await apiFetch('/api/auth/mfa/start', { method: 'POST' });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as StartResponse;
      setStart(data);
      setStage('enrolling');
      setCode('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    } finally {
      setBusy(false);
    }
  }, []);

  const confirmEnroll = useCallback(async () => {
    if (!start) return;
    setError(null);
    setBusy(true);
    try {
      const res = await apiFetch('/api/auth/mfa/confirm', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          secret: start.secret,
          code: code.trim(),
          recovery_codes: start.recovery_codes,
        }),
      });
      if (!res.ok) {
        const t = await res.text().catch(() => 'invalid code');
        throw new Error(t || 'invalid code');
      }
      setStage('enrolled');
      setStart(null);
      setCode('');
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    } finally {
      setBusy(false);
    }
  }, [start, code, refresh]);

  const disable = useCallback(async () => {
    setError(null);
    setBusy(true);
    try {
      const res = await apiFetch('/api/auth/mfa/disable', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ code: code.trim() }),
      });
      if (!res.ok) {
        const t = await res.text().catch(() => 'invalid code');
        throw new Error(t || 'invalid code');
      }
      setStage('idle');
      setCode('');
      setStart(null);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    } finally {
      setBusy(false);
    }
  }, [code, refresh]);

  if (status !== 'authed') {
    return (
      <div className="center-screen">
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button
              type="button"
              className="nav-item"
              onClick={() => router.push('/')}
              style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}
            >
              ←&nbsp;back
            </button>
            <span className="sep">/</span>
            <span className="here">security</span>
          </div>
        </div>

        <div className="scroll">
          <div className="pane">
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">⚿</span> ACCOUNT
                  <span className="meta">
                    {user} · {role}
                  </span>
                </div>
              </div>
              <div className="panel-body" style={{ padding: 16 }}>
                <div className="mono muted" style={{ fontSize: 12, lineHeight: 1.6 }}>
                  GitHub OAuth controls *who* can sign in (the
                  ALLOWED_GITHUB_USERS env var). The two-factor
                  challenge below adds a per-account second factor on
                  top of the OAuth flow.
                </div>
              </div>
            </div>

            <div className="panel" style={{ marginTop: 12 }}>
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">⌘</span> TWO-FACTOR (TOTP)
                  <span
                    className="meta"
                    style={{
                      color: mfaEnabled ? 'var(--ok)' : 'var(--warn)',
                    }}
                  >
                    {mfaEnabled ? '● enabled' : '○ disabled'}
                  </span>
                </div>
              </div>
              <div className="panel-body" style={{ padding: 16 }}>
                {error && (
                  <div
                    className="mono"
                    style={{
                      color: 'var(--err)',
                      fontSize: 12,
                      marginBottom: 12,
                    }}
                  >
                    {error}
                  </div>
                )}

                {/* idle (not enabled) → offer to enroll */}
                {!mfaEnabled && stage === 'idle' && (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                    <div
                      className="mono muted"
                      style={{ fontSize: 12, lineHeight: 1.6 }}
                    >
                      add a second factor to your account using any
                      RFC-6238 compatible authenticator: Aegis, Bitwarden,
                      1Password, Google Authenticator, etc.
                    </div>
                    <button
                      type="button"
                      className="btn primary"
                      disabled={busy}
                      onClick={beginEnroll}
                      style={{ alignSelf: 'flex-start' }}
                    >
                      {busy ? 'starting…' : 'set up 2FA'}
                    </button>
                  </div>
                )}

                {/* enrolling → show QR + confirm code */}
                {stage === 'enrolling' && start && (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                    <div
                      style={{
                        display: 'flex',
                        gap: 16,
                        flexWrap: 'wrap',
                        alignItems: 'flex-start',
                      }}
                    >
                      <div
                        style={{
                          background: 'var(--bg-1)',
                          padding: 12,
                          borderRadius: 6,
                          border: '1px solid var(--bd)',
                        }}
                      >
                        <QRCodeSVG
                          value={start.otpauth_uri}
                          size={180}
                          bgColor="#0e0e0e"
                          fgColor="#bdf564"
                          level="M"
                          marginSize={2}
                        />
                      </div>
                      <div style={{ minWidth: 240, flex: 1 }}>
                        <div
                          className="muted mono"
                          style={{ fontSize: 11, marginBottom: 4 }}
                        >
                          can't scan? enter the secret manually:
                        </div>
                        <code
                          className="mono"
                          style={{
                            display: 'block',
                            wordBreak: 'break-all',
                            fontSize: 12,
                            background: 'var(--bg-1)',
                            padding: '6px 8px',
                            borderRadius: 4,
                            border: '1px solid var(--bd)',
                          }}
                        >
                          {start.secret.match(/.{1,4}/g)?.join(' ')}
                        </code>
                      </div>
                    </div>

                    <div>
                      <div
                        className="muted mono"
                        style={{ fontSize: 11, marginBottom: 4 }}
                      >
                        save these recovery codes — each one works once
                        if you lose access to your authenticator:
                      </div>
                      <div
                        style={{
                          display: 'grid',
                          gridTemplateColumns: 'repeat(2, 1fr)',
                          gap: 4,
                          fontFamily: 'var(--mono)',
                          fontSize: 12,
                          background: 'var(--bg-1)',
                          padding: 8,
                          borderRadius: 4,
                          border: '1px solid var(--bd)',
                        }}
                      >
                        {start.recovery_codes.map((c) => (
                          <code key={c}>{c}</code>
                        ))}
                      </div>
                    </div>

                    <div
                      style={{
                        display: 'flex',
                        gap: 8,
                        alignItems: 'center',
                        flexWrap: 'wrap',
                      }}
                    >
                      <input
                        value={code}
                        onChange={(e) => setCode(e.target.value)}
                        placeholder="123 456"
                        inputMode="numeric"
                        autoComplete="one-time-code"
                        spellCheck={false}
                        style={{
                          fontFamily: 'var(--mono)',
                          fontSize: 16,
                          letterSpacing: 2,
                          textAlign: 'center',
                          padding: '8px 12px',
                          background: 'var(--bg-1)',
                          border: '1px solid var(--bd)',
                          color: 'var(--fg)',
                          borderRadius: 4,
                          width: 140,
                        }}
                      />
                      <button
                        type="button"
                        className="btn primary"
                        disabled={busy || !code.trim()}
                        onClick={confirmEnroll}
                      >
                        {busy ? 'confirming…' : 'confirm enrollment'}
                      </button>
                      <button
                        type="button"
                        className="btn"
                        disabled={busy}
                        onClick={() => {
                          setStart(null);
                          setStage('idle');
                          setCode('');
                          setError(null);
                        }}
                      >
                        cancel
                      </button>
                    </div>
                  </div>
                )}

                {/* enrolled (just-now confirmed) */}
                {stage === 'enrolled' && (
                  <div className="mono" style={{ color: 'var(--ok)', fontSize: 13 }}>
                    ✓ two-factor enabled. you'll be challenged on next sign-in.
                  </div>
                )}

                {/* enabled (steady state) → offer to disable */}
                {mfaEnabled && stage !== 'enrolling' && stage !== 'enrolled' && (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                    <div className="mono muted" style={{ fontSize: 12, lineHeight: 1.6 }}>
                      to remove your second factor, enter the current 6-digit
                      code from your authenticator. recovery codes are not
                      accepted here on purpose — losing your authenticator
                      should require account recovery, not a one-click
                      bypass.
                    </div>
                    <div
                      style={{
                        display: 'flex',
                        gap: 8,
                        alignItems: 'center',
                        flexWrap: 'wrap',
                      }}
                    >
                      <input
                        value={code}
                        onChange={(e) => setCode(e.target.value)}
                        placeholder="123 456"
                        inputMode="numeric"
                        autoComplete="one-time-code"
                        spellCheck={false}
                        style={{
                          fontFamily: 'var(--mono)',
                          fontSize: 16,
                          letterSpacing: 2,
                          textAlign: 'center',
                          padding: '8px 12px',
                          background: 'var(--bg-1)',
                          border: '1px solid var(--bd)',
                          color: 'var(--fg)',
                          borderRadius: 4,
                          width: 140,
                        }}
                      />
                      <button
                        type="button"
                        className="btn"
                        style={{ borderColor: 'var(--err-bd)', color: 'var(--err)' }}
                        disabled={busy || !code.trim()}
                        onClick={disable}
                      >
                        {busy ? 'disabling…' : 'disable 2FA'}
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
