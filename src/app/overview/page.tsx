'use client';

import { useEffect } from 'react';
import { useRouter } from 'next/navigation';
import FleetOverview from '@/components/FleetOverview';
import { useSession } from '@/components/providers/SessionProvider';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { Loader2Icon } from 'lucide-react';

export default function OverviewPage() {
  const router = useRouter();
  const { isConnected } = useWebSocket();
  const { status } = useSession();

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  if (status !== 'authed') {
    return (
      <div className="app-shell" style={{ alignItems: 'center', justifyContent: 'center' }}>
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
            <span className="here">fleet/overview</span>
          </div>
          <div className="topbar-actions">
            <span className={`pill ${isConnected ? 'live' : 'err'}`}>
              <span className={`dot ${isConnected ? 'pulse' : ''}`} />
              {isConnected ? 'LIVE' : 'OFFLINE'}
            </span>
          </div>
        </div>
        <div className="scroll">
          <FleetOverview />
        </div>
      </main>
    </div>
  );
}
