'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Loader2Icon } from 'lucide-react';
import { apiFetch } from '@/lib/api';
import { useCanWrite } from '@/components/providers/SessionProvider';

type AuthStatus = 'checking' | 'authed' | 'guest';
type SubmitStatus = 'idle' | 'loading' | 'success' | 'error';

export default function DeviceAuthPage() {
  const router = useRouter();
  const canWrite = useCanWrite();
  const [authStatus, setAuthStatus] = useState<AuthStatus>('checking');
  const [userCode, setUserCode] = useState('');
  const [submitStatus, setSubmitStatus] = useState<SubmitStatus>('idle');
  const [message, setMessage] = useState('');

  useEffect(() => {
    let cancelled = false;
    fetch('/api/me', { credentials: 'same-origin' })
      .then((res) => {
        if (cancelled) return;
        if (res.status === 401) {
          window.location.href = `/auth/login`;
          return;
        }
        setAuthStatus(res.ok ? 'authed' : 'guest');
      })
      .catch(() => {
        if (!cancelled) setAuthStatus('guest');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = userCode.trim().toUpperCase();
    if (!trimmed) return;

    setSubmitStatus('loading');
    setMessage('');

    try {
      const res = await apiFetch('/api/device/approve', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ user_code: trimmed }),
      });

      if (res.ok) {
        setSubmitStatus('success');
        setMessage('Agent approved. It should connect within a few seconds.');
        setUserCode('');
      } else if (res.status === 401) {
        window.location.href = '/auth/login';
      } else {
        const text = await res.text();
        setSubmitStatus('error');
        setMessage(text || 'Invalid or expired code.');
      }
    } catch {
      setSubmitStatus('error');
      setMessage('Could not reach the server.');
    }
  };

  if (authStatus === 'checking') {
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
            <span className="here">connect-agent</span>
          </div>
        </div>

        <div className="scroll">
          <div className="pane" style={{ alignItems: 'center' }}>
            <div className="panel" style={{ width: 'min(560px, 100%)', marginTop: 48 }}>
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">＋</span> CONNECT AGENT
                </div>
              </div>
              <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <pre
                  style={{
                    fontFamily: 'var(--mono)',
                    fontSize: 11,
                    color: 'var(--fg-3)',
                    margin: 0,
                    lineHeight: 1.4,
                  }}
                >
                  {`┌──────────────────────────────────────┐
│  on the new host run:                │
│                                      │
│    $ journalctl -u sys-manager-agent │
│      -n 20                           │
│                                      │
│  it will print an 8-char code.       │
└──────────────────────────────────────┘`}
                </pre>

                <form onSubmit={handleSubmit} style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div className="field">
                    <label>pairing code</label>
                    <input
                      type="text"
                      required
                      autoFocus
                      value={userCode}
                      onChange={(e) => setUserCode(e.target.value)}
                      placeholder="ABCD-1234"
                      spellCheck={false}
                      autoCorrect="off"
                      autoCapitalize="characters"
                      className="input"
                      style={{
                        height: 46,
                        fontSize: 18,
                        textAlign: 'center',
                        letterSpacing: '0.3em',
                        textTransform: 'uppercase',
                      }}
                    />
                  </div>

                  <div className="row between">
                    <div className="kbd-hint">codes expire after ~5 min</div>
                    <button
                      type="submit"
                      disabled={submitStatus === 'loading' || !userCode.trim() || !canWrite}
                      title={!canWrite ? 'viewer role: read-only' : undefined}
                      className="btn primary"
                    >
                      {submitStatus === 'loading' ? '…' : '▶ approve & connect'}
                    </button>
                  </div>
                </form>

                {submitStatus === 'success' && (
                  <div
                    style={{
                      padding: 10,
                      background: 'var(--accent-bg)',
                      border: '1px solid var(--accent-bd)',
                      borderRadius: 'var(--r)',
                      color: 'var(--accent)',
                      fontFamily: 'var(--mono)',
                      fontSize: 12,
                    }}
                  >
                    {message}
                  </div>
                )}
                {submitStatus === 'error' && (
                  <div
                    style={{
                      padding: 10,
                      background: 'var(--err-bg)',
                      border: '1px solid var(--err-bd)',
                      borderRadius: 'var(--r)',
                      color: 'var(--err)',
                      fontFamily: 'var(--mono)',
                      fontSize: 12,
                    }}
                  >
                    {message}
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
