'use client';

import { useCallback, useEffect, useState } from 'react';
import { useUi } from './providers/UiProvider';
import { apiFetch } from '@/lib/api';
import type { HealthProbe, HealthProbeKind } from '@/lib/types';
import {
  ActivitySquareIcon,
  PlusIcon,
  Trash2Icon,
  Loader2Icon,
  CheckCircleIcon,
  AlertCircleIcon,
  CircleDashedIcon,
} from 'lucide-react';

function fmtTs(secs: number | null | undefined) {
  if (!secs) return '—';
  return new Date(secs * 1000).toLocaleString();
}

export default function HealthProbes({ agentId }: { agentId: string }) {
  const ui = useUi();
  const [loading, setLoading] = useState(true);
  const [probes, setProbes] = useState<HealthProbe[]>([]);
  const [creating, setCreating] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const res = await apiFetch('/api/health-probes');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const rows: HealthProbe[] = await res.json();
      setProbes(rows.filter((r) => r.agent_id === agentId));
    } catch (e) {
      ui.toast('error', `Load failed: ${(e as Error).message}`);
    } finally {
      setLoading(false);
    }
  }, [agentId, ui]);

  useEffect(() => {
    void refresh();
    const t = setInterval(refresh, 5_000);
    return () => clearInterval(t);
  }, [refresh]);

  const remove = async (id: number, name: string) => {
    const ok = await ui.confirm({
      title: `Delete probe "${name}"?`,
      destructive: true,
      confirmLabel: 'Delete',
    });
    if (!ok) return;
    try {
      const res = await apiFetch(`/api/health-probes/${id}`, {
        method: 'DELETE',
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      ui.toast('success', `Probe "${name}" removed`);
      void refresh();
    } catch (e) {
      ui.toast('error', `Delete failed: ${(e as Error).message}`);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <ActivitySquareIcon className="w-5 h-5 text-slate-400" />
          <h2 className="text-base font-semibold">Health probes</h2>
          <span className="text-xs text-slate-500">· {probes.length}</span>
        </div>
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-md"
        >
          <PlusIcon className="w-3.5 h-3.5" />
          Add probe
        </button>
      </div>

      {creating && (
        <ProbeForm
          agentId={agentId}
          onClose={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            void refresh();
          }}
        />
      )}

      {loading && probes.length === 0 ? (
        <div className="flex items-center justify-center py-8 text-slate-500">
          <Loader2Icon className="w-4 h-4 animate-spin" />
        </div>
      ) : probes.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          No probes configured for this host yet. Add one to start monitoring.
        </div>
      ) : (
        <ul className="divide-y divide-slate-800 border border-slate-800 rounded-md overflow-hidden">
          {probes.map((p) => (
            <li
              key={p.id}
              className="px-3 py-2 bg-slate-900 flex items-center gap-3"
            >
              <StateIcon state={p.last_state} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-slate-100 text-sm truncate">
                    {p.name}
                  </span>
                  <span className="text-[10px] uppercase tracking-wide px-1 py-0.5 rounded bg-slate-800 text-slate-300">
                    {p.kind}
                  </span>
                  {!p.enabled && (
                    <span className="text-[10px] uppercase tracking-wide px-1 py-0.5 rounded bg-slate-800 text-slate-500">
                      disabled
                    </span>
                  )}
                </div>
                <div className="text-xs text-slate-500 truncate" title={p.target}>
                  <code className="text-slate-400">{p.target}</code>
                  <span className="ml-2 text-slate-500">
                    every {p.interval_secs}s · timeout {p.timeout_secs}s
                  </span>
                </div>
                <div className="text-[11px] text-slate-500 truncate">
                  {p.last_state ? (
                    <>
                      {p.last_detail ?? '—'}
                      {p.last_latency_ms != null && (
                        <span className="ml-2 text-slate-600">
                          {p.last_latency_ms}ms
                        </span>
                      )}
                      <span className="ml-2 text-slate-600">
                        @ {fmtTs(p.last_run_at)}
                      </span>
                    </>
                  ) : (
                    'awaiting first sample…'
                  )}
                </div>
              </div>
              <button
                type="button"
                onClick={() => remove(p.id, p.name)}
                title="Delete probe"
                className="p-1.5 rounded text-slate-400 hover:text-red-400 hover:bg-slate-800"
              >
                <Trash2Icon className="w-4 h-4" />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function StateIcon({ state }: { state: string | null }) {
  if (state === 'green') {
    return <CheckCircleIcon className="w-4 h-4 text-emerald-400 shrink-0" />;
  }
  if (state === 'red') {
    return <AlertCircleIcon className="w-4 h-4 text-red-400 shrink-0" />;
  }
  return <CircleDashedIcon className="w-4 h-4 text-slate-500 shrink-0" />;
}

function ProbeForm({
  agentId,
  onClose,
  onCreated,
}: {
  agentId: string;
  onClose: () => void;
  onCreated: () => void;
}) {
  const ui = useUi();
  const [name, setName] = useState('');
  const [kind, setKind] = useState<HealthProbeKind>('http');
  const [target, setTarget] = useState('');
  const [intervalSecs, setIntervalSecs] = useState(30);
  const [timeoutSecs, setTimeoutSecs] = useState(5);
  const [expectStatus, setExpectStatus] = useState<string>('');
  const [expectBody, setExpectBody] = useState('');
  const [submitting, setSubmitting] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name || !target) return;
    setSubmitting(true);
    try {
      const body = {
        agent_id: agentId,
        name,
        kind,
        target,
        interval_secs: intervalSecs,
        timeout_secs: timeoutSecs,
        expect_status:
          kind === 'http' && expectStatus ? Number(expectStatus) : null,
        expect_body: kind === 'http' && expectBody ? expectBody : null,
        enabled: true,
      };
      const res = await apiFetch('/api/health-probes', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      ui.toast('success', `Probe "${name}" created`);
      onCreated();
    } catch (err) {
      ui.toast('error', `Create failed: ${(err as Error).message}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form
      onSubmit={submit}
      className="rounded-md border border-slate-800 bg-slate-900/40 p-3 space-y-3"
    >
      <div className="grid grid-cols-2 gap-3">
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Name
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="api-healthz"
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100 focus:outline-none focus:ring-1 focus:ring-blue-500"
            required
          />
        </label>
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Kind
          <select
            value={kind}
            onChange={(e) => setKind(e.target.value as HealthProbeKind)}
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
          >
            <option value="http">http</option>
            <option value="tcp">tcp</option>
          </select>
        </label>
      </div>
      <label className="text-xs text-slate-400 flex flex-col gap-1">
        Target
        <input
          type="text"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          placeholder={kind === 'http' ? 'https://example.com/healthz' : 'host:port'}
          className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100 focus:outline-none focus:ring-1 focus:ring-blue-500"
          required
        />
      </label>
      <div className="grid grid-cols-2 gap-3">
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Interval (s)
          <input
            type="number"
            min={1}
            value={intervalSecs}
            onChange={(e) => setIntervalSecs(Number(e.target.value))}
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
          />
        </label>
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Timeout (s)
          <input
            type="number"
            min={1}
            value={timeoutSecs}
            onChange={(e) => setTimeoutSecs(Number(e.target.value))}
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
          />
        </label>
      </div>
      {kind === 'http' && (
        <div className="grid grid-cols-2 gap-3">
          <label className="text-xs text-slate-400 flex flex-col gap-1">
            Expect status (optional)
            <input
              type="number"
              value={expectStatus}
              onChange={(e) => setExpectStatus(e.target.value)}
              placeholder="200"
              className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
            />
          </label>
          <label className="text-xs text-slate-400 flex flex-col gap-1">
            Body must contain (optional)
            <input
              type="text"
              value={expectBody}
              onChange={(e) => setExpectBody(e.target.value)}
              placeholder="ok"
              className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
            />
          </label>
        </div>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <button
          type="button"
          onClick={onClose}
          className="text-xs px-2.5 py-1.5 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={submitting}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 text-white rounded-md"
        >
          {submitting && <Loader2Icon className="w-3.5 h-3.5 animate-spin" />}
          Create probe
        </button>
      </div>
    </form>
  );
}
