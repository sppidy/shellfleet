'use client';

import { useEffect } from 'react';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

export default function LoginPage() {
  const { status } = useSession();

  useEffect(() => {
    if (status === 'authed') {
      window.location.href = '/';
    }
  }, [status]);

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
          <div
            className="muted"
            style={{ fontSize: 12, fontFamily: 'var(--mono)' }}
          >
            sign in with the GitHub account on the allowlist
          </div>
        </div>

        <div className="panel">
          <div className="panel-body" style={{ padding: 20 }}>
            {status === 'loading' ? (
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  padding: 12,
                }}
              >
                <Loader2Icon
                  className="w-5 h-5 animate-spin"
                  style={{ color: 'var(--fg-2)' }}
                />
              </div>
            ) : (
              <a
                href="/auth/login"
                className="btn primary"
                style={{
                  width: '100%',
                  height: 36,
                  justifyContent: 'center',
                  fontSize: 13,
                }}
              >
                ⚿ continue with GitHub
              </a>
            )}
          </div>
        </div>

        <div
          className="kbd-hint"
          style={{ textAlign: 'center', marginTop: 20, fontSize: 11 }}
        >
          sessions last 24 hours.
        </div>
      </div>
    </div>
  );
}
