'use client';

import { useCallback, useEffect, useState } from 'react';
import { useUi } from './providers/UiProvider';
import { useCanWrite } from './providers/SessionProvider';
import { apiFetch } from '@/lib/api';
import type { HealthProbe, HealthProbeKind, ProbeLibraryEntry } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

function fmtTs(secs: number | null | undefined) {
  if (!secs) return '—';
  return new Date(secs * 1000).toLocaleString();
}

export default function HealthProbes({ agentId }: { agentId: string }) {
  const ui = useUi();
  const canWrite = useCanWrite();
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
      const res = await apiFetch(`/api/health-probes/${id}`, { method: 'DELETE' });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      ui.toast('success', `Probe "${name}" removed`);
      void refresh();
    } catch (e) {
      ui.toast('error', `Delete failed: ${(e as Error).message}`);
    }
  };

  const greenCount = probes.filter((p) => p.last_state === 'green').length;
  const redCount = probes.filter((p) => p.last_state === 'red').length;

  return (
    <div className="pane">
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

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">♡</span> HEALTH PROBES
            <span className="meta">
              {probes.length} probes · {greenCount} green · {redCount} red
            </span>
          </div>
          <div className="panel-actions">
            <button
              className="btn primary"
              onClick={() => setCreating(true)}
              disabled={!canWrite}
              title={!canWrite ? 'viewer role: read-only' : undefined}
            >
              + probe
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {loading && probes.length === 0 ? (
            <div className="empty">
              <Loader2Icon className="w-4 h-4 animate-spin" />
            </div>
          ) : probes.length === 0 ? (
            <div className="empty">No probes configured for this host yet.</div>
          ) : (
            <table className="tbl">
              <tbody>
                {probes.map((p) => {
                  const cls =
                    p.last_state === 'green'
                      ? 'ok'
                      : p.last_state === 'red'
                        ? 'err-c'
                        : 'muted';
                  return (
                    <tr key={p.id}>
                      <td className={`${cls} center`} style={{ width: 24 }}>
                        ●
                      </td>
                      <td className="mono" style={{ color: 'var(--fg)' }}>
                        {p.name}
                        {!p.enabled && (
                          <span className="chip muted" style={{ marginLeft: 8 }}>
                            disabled
                          </span>
                        )}
                      </td>
                      <td className="mono muted" style={{ width: 60 }}>
                        {p.kind}
                      </td>
                      <td className="mono">{p.target}</td>
                      <td className="mono" style={{ width: 80 }}>
                        {p.last_latency_ms != null ? `${p.last_latency_ms}ms` : '—'}
                      </td>
                      <td className={`mono ${cls}`} style={{ width: 220 }}>
                        {p.last_state ? p.last_detail ?? '—' : 'awaiting first sample…'}
                      </td>
                      <td className="muted" style={{ fontSize: 10.5, width: 180 }}>
                        every {p.interval_secs}s · {fmtTs(p.last_run_at)}
                      </td>
                      <td className="actions" style={{ width: 80 }}>
                        <button
                          className="btn sm icon danger"
                          title={!canWrite ? 'viewer role: read-only' : 'Remove'}
                          disabled={!canWrite}
                          onClick={() => remove(p.id, p.name)}
                        >
                          ×
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
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
        /* ignore */
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
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">+</span> NEW PROBE
        </div>
        <div className="panel-actions">
          <div className="seg">
            <button
              className={mode === 'custom' ? 'on' : ''}
              onClick={() => setMode('custom')}
            >
              custom
            </button>
            <button
              className={mode === 'library' ? 'on' : ''}
              onClick={() => setMode('library')}
            >
              library
            </button>
          </div>
        </div>
      </div>
      <form
        onSubmit={submit}
        className="panel-body"
        style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        {mode === 'library' && (
          <div className="field">
            <label>stock probe</label>
            {library.length === 0 ? (
              <span className="muted" style={{ fontSize: 11 }}>
                Loading library…
              </span>
            ) : (
              <select
                className="select"
                value={libraryPick}
                onChange={(e) => applyLibraryPick(e.target.value)}
              >
                <option value="">— pick a probe —</option>
                {library.map((e) => (
                  <option key={e.script} value={e.script}>
                    {e.title} ({e.script})
                  </option>
                ))}
              </select>
            )}
            {libraryPick && (
              <div className="muted" style={{ fontSize: 11 }}>
                {library.find((e) => e.script === libraryPick)?.description}
              </div>
            )}
          </div>
        )}

        <div className="grid-2">
          <div className="field">
            <label>name</label>
            <input
              className="input"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="api-healthz"
              required
            />
          </div>
          <div className="field">
            <label>kind</label>
            <select
              className="select"
              value={kind}
              onChange={(e) => setKind(e.target.value as HealthProbeKind)}
            >
              <option value="http">http</option>
              <option value="tcp">tcp</option>
              <option value="exec">exec</option>
            </select>
          </div>
        </div>

        <div className="field">
          <label>target</label>
          <input
            className="input"
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
            required
          />
        </div>

        <div className="grid-2">
          <div className="field">
            <label>interval (s)</label>
            <input
              className="input"
              type="number"
              min={1}
              value={intervalSecs}
              onChange={(e) => setIntervalSecs(Number(e.target.value))}
            />
          </div>
          <div className="field">
            <label>timeout (s)</label>
            <input
              className="input"
              type="number"
              min={1}
              value={timeoutSecs}
              onChange={(e) => setTimeoutSecs(Number(e.target.value))}
            />
          </div>
        </div>

        {kind === 'exec' && (
          <div className="field">
            <div className="row between">
              <label>env (KEY=VALUE)</label>
              <button
                type="button"
                className="btn sm"
                onClick={() => setEnvPairs((p) => [...p, { key: '', value: '' }])}
              >
                + add
              </button>
            </div>
            {envPairs.length === 0 ? (
              <span className="muted" style={{ fontSize: 11 }}>
                No env overrides. Add things like THRESHOLD=85.
              </span>
            ) : (
              envPairs.map((p, i) => (
                <div
                  key={i}
                  style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 6 }}
                >
                  <input
                    className="input"
                    type="text"
                    value={p.key}
                    onChange={(e) =>
                      setEnvPairs((arr) =>
                        arr.map((x, j) => (j === i ? { ...x, key: e.target.value } : x)),
                      )
                    }
                    placeholder="KEY"
                  />
                  <input
                    className="input"
                    type="text"
                    value={p.value}
                    onChange={(e) =>
                      setEnvPairs((arr) =>
                        arr.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)),
                      )
                    }
                    placeholder="value"
                  />
                  <button
                    type="button"
                    className="btn sm icon danger"
                    onClick={() => setEnvPairs((arr) => arr.filter((_, j) => j !== i))}
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
          <div className="grid-2">
            <div className="field">
              <label>expect status (optional)</label>
              <input
                className="input"
                type="number"
                value={expectStatus}
                onChange={(e) => setExpectStatus(e.target.value)}
                placeholder="200"
              />
            </div>
            <div className="field">
              <label>body must contain (optional)</label>
              <input
                className="input"
                type="text"
                value={expectBody}
                onChange={(e) => setExpectBody(e.target.value)}
                placeholder="ok"
              />
            </div>
          </div>
        )}

        <div className="row between">
          <button type="button" className="btn" onClick={onClose}>
            cancel
          </button>
          <button type="submit" className="btn primary" disabled={submitting}>
            {submitting ? '…' : '+ create probe'}
          </button>
        </div>
      </form>
    </div>
  );
}
