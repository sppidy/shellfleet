'use client';

import { useEffect } from 'react';
import { useRouter } from 'next/navigation';
import FleetOverview from '@/components/FleetOverview';
import { useSession } from '@/components/providers/SessionProvider';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { ArrowLeftIcon, Loader2Icon } from 'lucide-react';

export default function OverviewPage() {
  const router = useRouter();
  const { isConnected } = useWebSocket();
  const { status } = useSession();

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  if (status !== 'authed') {
    return (
      <div className="min-h-screen flex items-center justify-center text-slate-500">
        <Loader2Icon className="w-6 h-6 animate-spin" />
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 flex flex-col">
      <header className="border-b border-slate-800 px-6 py-4 flex items-center justify-between">
        <button
          type="button"
          onClick={() => router.push('/')}
          className="inline-flex items-center text-sm text-slate-400 hover:text-slate-100"
        >
          <ArrowLeftIcon className="w-4 h-4 mr-1.5" />
          Back to dashboard
        </button>
        <span
          className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full border text-xs ${
            isConnected
              ? 'border-emerald-500/30 text-emerald-400 bg-emerald-500/5'
              : 'border-red-500/30 text-red-400 bg-red-500/5'
          }`}
        >
          <span
            className={`w-1.5 h-1.5 rounded-full ${
              isConnected ? 'bg-emerald-400' : 'bg-red-400'
            }`}
          />
          {isConnected ? 'Live' : 'Offline'}
        </span>
      </header>
      <main className="flex-1">
        <FleetOverview />
      </main>
    </div>
  );
}
