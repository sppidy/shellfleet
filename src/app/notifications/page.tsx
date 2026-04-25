'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useSession } from '@/components/providers/SessionProvider';
import { useUi } from '@/components/providers/UiProvider';
import { apiFetch } from '@/lib/api';
import type { Notification } from '@/lib/types';
import {
  ArrowLeftIcon,
  RefreshCwIcon,
  Loader2Icon,
  CheckCircleIcon,
  AlertCircleIcon,
  AlertTriangleIcon,
  InfoIcon,
  Trash2Icon,
  CheckCheckIcon,
  BellIcon,
} from 'lucide-react';

const RELATIVE = (ts: number) => {
  if (!ts) return '—';
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86_400) return `${Math.floor(delta / 3_600)}h ago`;
  return `${Math.floor(delta / 86_400)}d ago`;
};

export default function NotificationsPage() {
  const router = useRouter();
  const ui = useUi();
  const { status } = useSession();
  const [rows, setRows] = useState<Notification[] | null>(null);
  const [unreadOnly, setUnreadOnly] = useState(false);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  const refresh = useCallback(async () => {
    try {
      const url = unreadOnly
        ? '/api/notifications?unread=true&limit=200'
        : '/api/notifications?limit=200';
      const res = await apiFetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data: Notification[] = await res.json();
      setRows(data);
    } catch (e) {
      ui.toast('error', `Load failed: ${(e as Error).message}`);
    }
  }, [unreadOnly, ui]);

  useEffect(() => {
    void refresh();
    const t = setInterval(refresh, 5_000);
    return () => clearInterval(t);
  }, [refresh]);

  const markRead = async (id: number) => {
    const res = await apiFetch(`/api/notifications/${id}/read`, { method: 'POST' });
    if (res.ok) void refresh();
  };

  const markAll = async () => {
    const res = await apiFetch('/api/notifications/mark-all-read', { method: 'POST' });
    if (res.ok) {
      const j = await res.json();
      ui.toast('success', `Marked ${j.updated} as read`);
      void refresh();
    }
  };

  const remove = async (id: number) => {
    const res = await apiFetch(`/api/notifications/${id}`, { method: 'DELETE' });
    if (res.ok) void refresh();
  };

  if (status === 'loading' || status === 'guest') {
    return (
      <div className="flex h-screen items-center justify-center text-slate-500 bg-slate-950">
        <Loader2Icon className="w-6 h-6 animate-spin" />
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100">
      <header className="border-b border-slate-800 bg-slate-900">
        <div className="max-w-4xl mx-auto px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => router.push('/')}
              className="text-slate-400 hover:text-slate-100"
              aria-label="Back"
            >
              <ArrowLeftIcon className="w-5 h-5" />
            </button>
            <BellIcon className="w-5 h-5 text-slate-400" />
            <h1 className="text-lg font-semibold">Notifications</h1>
          </div>
          <div className="flex items-center gap-2">
            <label className="text-xs text-slate-400 flex items-center gap-1.5 select-none">
              <input
                type="checkbox"
                checked={unreadOnly}
                onChange={(e) => setUnreadOnly(e.target.checked)}
                className="accent-blue-600"
              />
              Unread only
            </label>
            <button
              type="button"
              onClick={markAll}
              className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 border border-slate-700 rounded-md text-slate-300 hover:bg-slate-800"
            >
              <CheckCheckIcon className="w-3.5 h-3.5" />
              Mark all read
            </button>
            <button
              type="button"
              onClick={refresh}
              className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 border border-slate-700 rounded-md text-slate-300 hover:bg-slate-800"
            >
              <RefreshCwIcon className="w-3.5 h-3.5" />
              Refresh
            </button>
          </div>
        </div>
      </header>

      <main className="max-w-4xl mx-auto px-6 py-6">
        {rows === null ? (
          <div className="flex items-center justify-center py-12 text-slate-500">
            <Loader2Icon className="w-5 h-5 animate-spin" />
          </div>
        ) : rows.length === 0 ? (
          <div className="border border-dashed border-slate-800 rounded-md py-16 text-center text-slate-500">
            {unreadOnly ? 'No unread notifications.' : 'Inbox is empty.'}
          </div>
        ) : (
          <ul className="space-y-2">
            {rows.map((n) => (
              <li
                key={n.id}
                className={`rounded-md border px-3 py-3 flex items-start gap-3 transition-colors ${
                  n.read_at == null
                    ? 'bg-slate-900 border-slate-700'
                    : 'bg-slate-900/40 border-slate-800'
                }`}
              >
                <LevelIcon level={n.level} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-sm font-medium text-slate-100 truncate">
                      {n.title}
                    </span>
                    <span className="text-[10px] uppercase tracking-wide px-1 py-0.5 rounded bg-slate-800 text-slate-400">
                      {n.kind}
                    </span>
                    {n.read_at == null && (
                      <span className="text-[10px] uppercase tracking-wide px-1 py-0.5 rounded bg-blue-500/20 text-blue-300">
                        new
                      </span>
                    )}
                  </div>
                  {n.body && (
                    <pre className="mt-1.5 text-xs text-slate-400 whitespace-pre-wrap break-words bg-slate-950/40 rounded px-2 py-1.5 max-h-48 overflow-auto">
                      {n.body}
                    </pre>
                  )}
                  <div className="mt-1.5 text-[11px] text-slate-500">
                    {RELATIVE(n.created_at)}
                    {n.agent_id && (
                      <>
                        {' · '}
                        <code className="text-slate-400">
                          {n.agent_id.replace(/-id$/, '')}
                        </code>
                      </>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  {n.read_at == null && (
                    <button
                      type="button"
                      onClick={() => markRead(n.id)}
                      title="Mark read"
                      className="p-1.5 rounded text-slate-400 hover:text-slate-100 hover:bg-slate-800"
                    >
                      <CheckCircleIcon className="w-4 h-4" />
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => remove(n.id)}
                    title="Delete"
                    className="p-1.5 rounded text-slate-400 hover:text-red-400 hover:bg-slate-800"
                  >
                    <Trash2Icon className="w-4 h-4" />
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </main>
    </div>
  );
}

function LevelIcon({ level }: { level: string }) {
  if (level === 'error') {
    return <AlertCircleIcon className="w-4 h-4 mt-0.5 text-red-400 shrink-0" />;
  }
  if (level === 'warn') {
    return <AlertTriangleIcon className="w-4 h-4 mt-0.5 text-amber-400 shrink-0" />;
  }
  return <InfoIcon className="w-4 h-4 mt-0.5 text-slate-400 shrink-0" />;
}
