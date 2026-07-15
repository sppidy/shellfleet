'use client';

import { Suspense, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { Loader2Icon } from 'lucide-react';
import { apiFetch } from '@/lib/api';
import { useCanWrite } from '@/components/providers/SessionProvider';

type AuthStatus = 'checking' | 'authed' | 'guest';
type SubmitStatus = 'idle' | 'loading' | 'success' | 'error';

export default function DeviceAuthPage() {
  return (
    <Suspense
      fallback={
        <div className="center-screen">
          <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
        </div>
      }
    >
      <DeviceAuthPageContent />
    </Suspense>
  );
}

function DeviceAuthPageContent() {
  const router = useRouter();
  const cliAuth = useSearchParams().get('cli') === '1';
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
        setMessage(
          cliAuth
            ? 'CLI authorized. Return to the terminal to continue.'
            : 'Agent approved. It should connect within a few seconds.',
        );
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
            <span className="here">{cliAuth ? 'authorize-cli' : 'connect-agent'}</span>
          </div>
        </div>

        <div className="scroll">
          <div className="pane" style={{ alignItems: 'center' }}>
            <div className="panel" style={{ width: 'min(560px, 100%)', marginTop: 48 }}>
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">＋</span> {cliAuth ? 'AUTHORIZE CLI' : 'CONNECT AGENT'}
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
                  {cliAuth
                    ? `┌──────────────────────────────────────────┐
│  1. Run shellfleet login on your local      │
│     machine. It displays a short code.       │
│                                              │
│  2. Enter that code below to authorize the   │
│     trusted ShellFleet CLI.                  │
│                                              │
│  The CLI receives a short-lived session for  │
│  the operator WebSocket only. It cannot be   │
│  used as a browser or API session.           │
└──────────────────────────────────────────┘`
                    : `┌──────────────────────────────────────────┐
│  1. Install the agent on the new host:   │
│                                          │
│  curl -fsSL https://shellfleet-repo.     │
│    sppidy.in/shellfleet.gpg              │
│    | sudo tee /etc/apt/keyrings/         │
│    shellfleet.asc > /dev/null            │
│                                          │
│  echo "deb [signed-by=/etc/apt/keyrings/ │
│    shellfleet.asc] https://shellfleet-   │
│    repo.sppidy.in stable main"           │
│    | sudo tee /etc/apt/sources.list.d/   │
│    shellfleet.list                       │
│                                          │
│  sudo apt update                         │
│  sudo apt install -y shellfleet-agent    │
│                                          │
│  2. Start pairing:                       │
│                                          │
│  sudo shellfleet-agent --pair            │
│                                          │
│  3. It prints an 8-char code below.      │
│     Enter it here to approve.            │
└──────────────────────────────────────────┘`}
                </pre>

                <form onSubmit={handleSubmit} style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div className="field">
                    <label>{cliAuth ? 'authorization code' : 'pairing code'}</label>
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
                      {submitStatus === 'loading'
                        ? '…'
                        : cliAuth
                          ? '▶ authorize CLI'
                          : '▶ approve & connect'}
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
