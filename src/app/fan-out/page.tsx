'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useSession } from '@/components/providers/SessionProvider';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useUi } from '@/components/providers/UiProvider';
import { apiFetch } from '@/lib/api';
import type { FanOutKind, FanOutRunDetail } from '@/lib/types';
import {
  ArrowLeftIcon,
  RefreshCwIcon,
  Loader2Icon,
  CheckCircleIcon,
  AlertCircleIcon,
  CircleDashedIcon,
  WifiOffIcon,
  RocketIcon,
} from 'lucide-react';

const KIND_LABELS: Record<FanOutKind, string> = {
  'apt-status': 'apt status (list upgradable)',
  'apt-upgrade': 'apt upgrade',
  'docker-list': 'docker list',
};

function fmtTs(secs: number | null | undefined) {
  if (!secs) return '—';
  return new Date(secs * 1000).toLocaleString();
}

export default function FanOutPage() {
  const router = useRouter();
  const ui = useUi();
  const { status } = useSession();
  const { agents } = useWebSocket();
  const [kind, setKind] = useState<FanOutKind>('docker-list');
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [pkg, setPkg] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [run, setRun] = useState<FanOutRunDetail | null>(null);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  // Default: select all online agents.
  useEffect(() => {
    setSelected((prev) => {
      if (Object.keys(prev).length > 0) return prev;
      const init: Record<string, boolean> = {};
      for (const a of agents) init[a] = true;
      return init;
    });
  }, [agents]);

  const refresh = useCallback(async () => {
    if (!run) return;
    try {
      const res = await apiFetch(`/api/fan-out/${run.run.id}`);
      if (!res.ok) return;
      const data: FanOutRunDetail = await res.json();
      setRun(data);
    } catch {
      /* swallow */
    }
  }, [run]);

  useEffect(() => {
    if (!run) return;
    const t = setInterval(refresh, 2_000);
    return () => clearInterval(t);
  }, [run, refresh]);

  const toggleAll = (val: boolean) => {
    const next: Record<string, boolean> = {};
    for (const a of agents) next[a] = val;
    setSelected(next);
  };

  const submit = async () => {
    const ids = agents.filter((a) => selected[a]);
    if (ids.length === 0) {
      ui.toast('error', 'Pick at least one host');
      return;
    }
    if (kind === 'apt-upgrade') {
      const ok = await ui.confirm({
        title: `Run apt upgrade on ${ids.length} host${ids.length === 1 ? '' : 's'}?`,
        description: pkg
          ? `Package: ${pkg}`
          : 'This runs apt-get -y upgrade across every selected host.',
        destructive: true,
        confirmLabel: 'Run',
      });
      if (!ok) return;
    }
    setSubmitting(true);
    try {
      const res = await apiFetch('/api/fan-out', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          kind,
          agent_ids: ids,
          package: kind === 'apt-upgrade' && pkg ? pkg : null,
        }),
      });
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      const data: FanOutRunDetail = await res.json();
      setRun(data);
      ui.toast('success', `Fan-out run #${data.run.id} dispatched`);
    } catch (e) {
      ui.toast('error', `Submit failed: ${(e as Error).message}`);
    } finally {
      setSubmitting(false);
    }
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
        <div className="max-w-5xl mx-auto px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => router.push('/')}
              className="text-slate-400 hover:text-slate-100"
              aria-label="Back"
            >
              <ArrowLeftIcon className="w-5 h-5" />
            </button>
            <RocketIcon className="w-5 h-5 text-slate-400" />
            <h1 className="text-lg font-semibold">Fan-out</h1>
          </div>
          <button
            type="button"
            onClick={refresh}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 border border-slate-700 rounded-md text-slate-300 hover:bg-slate-800"
          >
            <RefreshCwIcon className="w-3.5 h-3.5" />
            Refresh
          </button>
        </div>
      </header>

      <main className="max-w-5xl mx-auto px-6 py-6 space-y-6">
        <section className="space-y-3">
          <h2 className="text-sm uppercase tracking-wide text-slate-500">
            Dispatch a command
          </h2>
          <div className="rounded-md border border-slate-800 bg-slate-900/40 p-4 space-y-3">
            <div className="grid grid-cols-2 gap-3">
              <label className="text-xs text-slate-400 flex flex-col gap-1">
                Kind
                <select
                  value={kind}
                  onChange={(e) => setKind(e.target.value as FanOutKind)}
                  className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
                >
                  {(Object.keys(KIND_LABELS) as FanOutKind[]).map((k) => (
                    <option key={k} value={k}>
                      {KIND_LABELS[k]}
                    </option>
                  ))}
                </select>
              </label>
              {kind === 'apt-upgrade' && (
                <label className="text-xs text-slate-400 flex flex-col gap-1">
                  Package (optional)
                  <input
                    type="text"
                    value={pkg}
                    onChange={(e) => setPkg(e.target.value)}
                    placeholder="leave blank for full upgrade"
                    className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm font-mono text-slate-100"
                  />
                </label>
              )}
            </div>

            <div>
              <div className="flex items-center justify-between mb-1">
                <span className="text-xs text-slate-400">Targets ({agents.length} online)</span>
                <div className="flex gap-2 text-[11px]">
                  <button
                    type="button"
                    onClick={() => toggleAll(true)}
                    className="text-slate-400 hover:text-slate-100"
                  >
                    select all
                  </button>
                  <button
                    type="button"
                    onClick={() => toggleAll(false)}
                    className="text-slate-400 hover:text-slate-100"
                  >
                    clear
                  </button>
                </div>
              </div>
              <div className="grid grid-cols-2 md:grid-cols-3 gap-1">
                {agents.map((a) => (
                  <label
                    key={a}
                    className="flex items-center gap-2 px-2 py-1 rounded text-sm bg-slate-950 border border-slate-800 hover:border-slate-700"
                  >
                    <input
                      type="checkbox"
                      checked={!!selected[a]}
                      onChange={(e) =>
                        setSelected((prev) => ({ ...prev, [a]: e.target.checked }))
                      }
                      className="accent-blue-600"
                    />
                    <span className="truncate">{a.replace(/-id$/, '')}</span>
                  </label>
                ))}
              </div>
            </div>

            <div className="flex justify-end">
              <button
                type="button"
                onClick={submit}
                disabled={submitting}
                className="text-sm flex items-center gap-1.5 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 text-white rounded-md"
              >
                {submitting ? (
                  <Loader2Icon className="w-4 h-4 animate-spin" />
                ) : (
                  <RocketIcon className="w-4 h-4" />
                )}
                Dispatch
              </button>
            </div>
          </div>
        </section>

        {run && (
          <section className="space-y-3">
            <h2 className="text-sm uppercase tracking-wide text-slate-500 flex items-center gap-2">
              Run #{run.run.id} — {run.run.kind}
              <span className="text-slate-600 normal-case tracking-normal text-xs">
                started {fmtTs(run.run.started_at)}
              </span>
            </h2>
            <div className="rounded-md border border-slate-800 overflow-hidden">
              <table className="w-full text-sm">
                <thead className="bg-slate-900/60 text-[11px] uppercase tracking-wide text-slate-500">
                  <tr>
                    <th className="text-left px-3 py-2 font-medium">Host</th>
                    <th className="text-left px-3 py-2 font-medium">Status</th>
                    <th className="text-left px-3 py-2 font-medium">Detail</th>
                    <th className="text-left px-3 py-2 font-medium">Finished</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-800">
                  {run.results.map((r) => (
                    <tr key={r.agent_id} className="bg-slate-900/30">
                      <td className="px-3 py-2 font-mono text-slate-200">
                        {r.agent_id.replace(/-id$/, '')}
                      </td>
                      <td className="px-3 py-2">
                        <StatusBadge status={r.status} />
                      </td>
                      <td className="px-3 py-2 text-slate-400 truncate max-w-md" title={r.detail ?? ''}>
                        {r.detail ?? '—'}
                      </td>
                      <td className="px-3 py-2 text-slate-500 text-xs">
                        {fmtTs(r.finished_at)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}
      </main>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  switch (status) {
    case 'success':
      return (
        <span className="inline-flex items-center gap-1 text-xs text-emerald-300">
          <CheckCircleIcon className="w-3.5 h-3.5" /> success
        </span>
      );
    case 'failed':
      return (
        <span className="inline-flex items-center gap-1 text-xs text-red-300">
          <AlertCircleIcon className="w-3.5 h-3.5" /> failed
        </span>
      );
    case 'pending':
      return (
        <span className="inline-flex items-center gap-1 text-xs text-amber-300">
          <Loader2Icon className="w-3.5 h-3.5 animate-spin" /> pending
        </span>
      );
    case 'offline':
      return (
        <span className="inline-flex items-center gap-1 text-xs text-slate-400">
          <WifiOffIcon className="w-3.5 h-3.5" /> offline
        </span>
      );
    default:
      return (
        <span className="inline-flex items-center gap-1 text-xs text-slate-400">
          <CircleDashedIcon className="w-3.5 h-3.5" /> {status}
        </span>
      );
  }
}
