'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import {
  ServerIcon,
  CheckCircleIcon,
  AlertCircleIcon,
  Loader2Icon,
  ArrowLeftIcon,
} from 'lucide-react';
import { apiFetch } from '@/lib/api';

type AuthStatus = 'checking' | 'authed' | 'guest';
type SubmitStatus = 'idle' | 'loading' | 'success' | 'error';

export default function DeviceAuthPage() {
  const router = useRouter();
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
      <div className="min-h-screen flex items-center justify-center bg-slate-950 text-slate-300">
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
        <span className="text-sm text-slate-500">Connect an agent</span>
      </header>

      <main className="flex-1 flex items-center justify-center px-4">
        <div className="w-full max-w-md">
          <div className="text-center mb-8">
            <div className="inline-flex w-12 h-12 items-center justify-center rounded-lg bg-blue-500/10 text-blue-400 mb-4">
              <ServerIcon className="w-6 h-6" />
            </div>
            <h1 className="text-2xl font-semibold">Connect a new agent</h1>
            <p className="text-sm text-slate-400 mt-2">
              On the agent host, run{' '}
              <code className="bg-slate-800/80 text-slate-200 px-1.5 py-0.5 rounded text-xs">
                journalctl -u sys-manager-agent -n 20
              </code>{' '}
              and paste the 8-character code below.
            </p>
          </div>

          <form
            onSubmit={handleSubmit}
            className="bg-slate-900 border border-slate-800 rounded-lg p-6 space-y-4"
          >
            <label className="block">
              <span className="block text-xs uppercase tracking-wide text-slate-400 mb-2">
                Device code
              </span>
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
                className="w-full bg-slate-950 border border-slate-700 rounded-md px-3 py-3 text-center text-lg tracking-[0.4em] uppercase font-mono focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
              />
            </label>

            <button
              type="submit"
              disabled={submitStatus === 'loading' || !userCode.trim()}
              className="w-full inline-flex items-center justify-center gap-2 py-2.5 rounded-md bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 disabled:cursor-not-allowed text-white text-sm font-medium transition-colors"
            >
              {submitStatus === 'loading' && <Loader2Icon className="w-4 h-4 animate-spin" />}
              {submitStatus === 'loading' ? 'Approving…' : 'Approve agent'}
            </button>

            {submitStatus === 'success' && (
              <div className="flex items-start gap-2 text-sm text-emerald-400 bg-emerald-500/5 border border-emerald-500/20 rounded-md p-3">
                <CheckCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
                <span>{message}</span>
              </div>
            )}
            {submitStatus === 'error' && (
              <div className="flex items-start gap-2 text-sm text-red-400 bg-red-500/5 border border-red-500/20 rounded-md p-3">
                <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
                <span>{message}</span>
              </div>
            )}
          </form>

          <p className="mt-6 text-xs text-slate-500 text-center">
            Codes expire after 5 minutes. Generate a new one by restarting the agent service.
          </p>
        </div>
      </main>
    </div>
  );
}
