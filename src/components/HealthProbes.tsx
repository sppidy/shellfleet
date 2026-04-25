'use client';

import { useCallback, useEffect, useState } from 'react';
import { useUi } from './providers/UiProvider';
import { apiFetch } from '@/lib/api';
import type { HealthProbe, HealthProbeKind, ProbeLibraryEntry } from '@/lib/types';
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
  const [mode, setMode] = useState<'custom' | 'library'>('custom');
  const [library, setLibrary] = useState<ProbeLibraryEntry[]>([]);
  const [libraryPick, setLibraryPick] = useState<string>('');
  const [name, setName] = useState('');
  const [kind, setKind] = useState<HealthProbeKind>('http');
  const [target, setTarget] = useState('');
  const [intervalSecs, setIntervalSecs] = useState(30);
  const [timeoutSecs, setTimeoutSecs] = useState(5);
  const [expectStatus, setExpectStatus] = useState<string>('');
  const [expectBody, setExpectBody] = useState('');
  const [envPairs, setEnvPairs] = useState<{ key: string; value: string }[]>([]);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const res = await apiFetch('/api/probe-library');
        if (!res.ok) return;
        const data: ProbeLibraryEntry[] = await res.json();
        if (!cancelled) setLibrary(data);
      } catch {
        /* ignore — library tab just shows empty */
      }
    };
    void load();
  }, []);

  const applyLibraryPick = (script: string) => {
    setLibraryPick(script);
    const entry = library.find((e) => e.script === script);
    if (!entry) return;
    setKind('exec');
    setTarget(entry.script);
    setIntervalSecs(entry.interval_secs);
    setTimeoutSecs(entry.timeout_secs);
    if (!name) setName(entry.script.replace(/\.sh$/, ''));
    setEnvPairs(entry.default_env.map((e) => ({ key: e.key, value: e.value })));
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name || !target) return;
    setSubmitting(true);
    try {
      const env = envPairs
        .map((p) => ({ k: p.key.trim(), v: p.value }))
        .filter((p) => p.k.length > 0)
        .map((p) => `${p.k}=${p.v}`);
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
        env,
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
      <div className="flex items-center gap-3 text-xs text-slate-400 border-b border-slate-800 pb-2">
        <label className="flex items-center gap-1.5">
          <input
            type="radio"
            name="probeMode"
            checked={mode === 'custom'}
            onChange={() => setMode('custom')}
            className="accent-blue-600"
          />
          Custom
        </label>
        <label className="flex items-center gap-1.5">
          <input
            type="radio"
            name="probeMode"
            checked={mode === 'library'}
            onChange={() => setMode('library')}
            className="accent-blue-600"
          />
          From library
        </label>
      </div>

      {mode === 'library' && (
        <div className="space-y-3">
          <label className="text-xs text-slate-400 flex flex-col gap-1">
            Stock probe
            {library.length === 0 ? (
              <span className="text-slate-500 italic">Loading library…</span>
            ) : (
              <select
                value={libraryPick}
                onChange={(e) => applyLibraryPick(e.target.value)}
                className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
              >
                <option value="">— pick a probe —</option>
                {library.map((e) => (
                  <option key={e.script} value={e.script}>
                    {e.title} ({e.script})
                  </option>
                ))}
              </select>
            )}
          </label>
          {libraryPick && (
            <p className="text-[11px] text-slate-500">
              {library.find((e) => e.script === libraryPick)?.description}
            </p>
          )}
        </div>
      )}

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
            <option value="exec">exec</option>
          </select>
        </label>
      </div>
      <label className="text-xs text-slate-400 flex flex-col gap-1">
        Target
        <input
          type="text"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          placeholder={
            kind === 'http'
              ? 'https://example.com/healthz'
              : kind === 'tcp'
                ? 'host:port'
                : 'script-name.sh (in /etc/sys-manager/probes.d/)'
          }
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
      {kind === 'exec' && (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs text-slate-400">Env (KEY=VALUE)</span>
            <button
              type="button"
              onClick={() => setEnvPairs((p) => [...p, { key: '', value: '' }])}
              className="text-[11px] text-slate-400 hover:text-slate-100"
            >
              + add
            </button>
          </div>
          {envPairs.length === 0 ? (
            <p className="text-[11px] text-slate-500 italic">
              No env overrides. Click "add" to set things like THRESHOLD=85.
            </p>
          ) : (
            envPairs.map((p, i) => (
              <div key={i} className="grid grid-cols-[1fr_1fr_auto] gap-2">
                <input
                  type="text"
                  value={p.key}
                  onChange={(e) =>
                    setEnvPairs((arr) => arr.map((x, j) => (j === i ? { ...x, key: e.target.value } : x)))
                  }
                  placeholder="KEY"
                  className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1 font-mono text-xs text-slate-100"
                />
                <input
                  type="text"
                  value={p.value}
                  onChange={(e) =>
                    setEnvPairs((arr) => arr.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))
                  }
                  placeholder="value"
                  className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1 font-mono text-xs text-slate-100"
                />
                <button
                  type="button"
                  onClick={() => setEnvPairs((arr) => arr.filter((_, j) => j !== i))}
                  className="text-slate-500 hover:text-red-300 px-1"
                  title="Remove"
                >
                  ×
                </button>
              </div>
            ))
          )}
        </div>
      )}
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
