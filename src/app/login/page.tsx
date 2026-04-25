'use client';

import { useEffect } from 'react';
import { useSession } from '@/components/providers/SessionProvider';
import { ServerIcon, KeyIcon, Loader2Icon } from 'lucide-react';

export default function LoginPage() {
  const { status } = useSession();

  // If we already have a valid session, jump back to the dashboard.
  useEffect(() => {
    if (status === 'authed') {
      window.location.href = '/';
    }
  }, [status]);

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 flex items-center justify-center px-4">
      <div className="w-full max-w-sm">
        <div className="text-center mb-8">
          <div className="inline-flex w-12 h-12 items-center justify-center rounded-lg bg-blue-500/10 text-blue-400 mb-4">
            <ServerIcon className="w-6 h-6" />
          </div>
          <h1 className="text-2xl font-semibold">Sys Manager</h1>
          <p className="text-sm text-slate-400 mt-2">
            Sign in with the GitHub account on the allowlist.
          </p>
        </div>

        <div className="bg-slate-900 border border-slate-800 rounded-lg p-6">
          {status === 'loading' ? (
            <div className="flex items-center justify-center py-2">
              <Loader2Icon className="w-5 h-5 animate-spin text-slate-400" />
            </div>
          ) : (
            <a
              href="/auth/login"
              className="w-full inline-flex items-center justify-center gap-2 py-2.5 rounded-md bg-slate-100 hover:bg-white text-slate-900 text-sm font-medium transition-colors"
            >
              <KeyIcon className="w-4 h-4" />
              Continue with GitHub
            </a>
          )}
        </div>

        <p className="mt-6 text-xs text-slate-500 text-center">
          Sessions last 24 hours.
        </p>
      </div>
    </div>
  );
}
