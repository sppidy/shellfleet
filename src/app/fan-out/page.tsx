'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useSession } from '@/components/providers/SessionProvider';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useUi } from '@/components/providers/UiProvider';
import { apiFetch } from '@/lib/api';
import type { FanOutKind, FanOutRunDetail, LabelsResponse } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

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
  const [targetMode, setTargetMode] = useState<'ids' | 'label'>('ids');
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [labelChoice, setLabelChoice] = useState<string>('');
  const [labels, setLabels] = useState<string[]>([]);
  const [pkg, setPkg] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [run, setRun] = useState<FanOutRunDetail | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const res = await apiFetch('/api/agent-labels');
        if (!res.ok) return;
        const data: LabelsResponse = await res.json();
        if (cancelled) return;
        setLabels(Object.keys(data.by_label).sort());
      } catch {
        /* ignore */
      }
    };
    void load();
  }, []);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

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
    if (targetMode === 'ids' && ids.length === 0) {
      ui.toast('error', 'Pick at least one host');
      return;
    }
    if (targetMode === 'label' && !labelChoice) {
      ui.toast('error', 'Pick a label');
      return;
    }
    if (kind === 'apt-upgrade') {
      const targetDesc =
        targetMode === 'ids'
          ? `${ids.length} host${ids.length === 1 ? '' : 's'}`
          : `every host tagged "${labelChoice}"`;
      const ok = await ui.confirm({
        title: `Run apt upgrade on ${targetDesc}?`,
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
      const body: Record<string, unknown> = {
        kind,
        package: kind === 'apt-upgrade' && pkg ? pkg : null,
      };
      if (targetMode === 'ids') {
        body.agent_ids = ids;
      } else {
        body.label = labelChoice;
      }
      const res = await apiFetch('/api/fan-out', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
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
      <div className="center-screen">
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  const selectedCount = agents.filter((a) => selected[a]).length;

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
            <span className="here">fan-out</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={refresh}>↻ refresh</button>
          </div>
        </div>

        <div className="scroll">
          <div className="pane">
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">↗</span> FAN-OUT
                </div>
              </div>
              <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
                <div className="grid-2">
                  <div className="field">
                    <label>kind</label>
                    <select
                      className="select"
                      value={kind}
                      onChange={(e) => setKind(e.target.value as FanOutKind)}
                    >
                      {(Object.keys(KIND_LABELS) as FanOutKind[]).map((k) => (
                        <option key={k} value={k}>
                          {KIND_LABELS[k]}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div className="field">
                    <label>targets</label>
                    <div className="seg">
                      <button
                        className={targetMode === 'ids' ? 'on' : ''}
                        onClick={() => setTargetMode('ids')}
                      >
                        by host
                      </button>
                      <button
                        className={targetMode === 'label' ? 'on' : ''}
                        onClick={() => setTargetMode('label')}
                      >
                        by label
                      </button>
                    </div>
                  </div>
                </div>

                {kind === 'apt-upgrade' && (
                  <div className="field">
                    <label>package (optional)</label>
                    <input
                      className="input"
                      type="text"
                      value={pkg}
                      onChange={(e) => setPkg(e.target.value)}
                      placeholder="leave blank for full upgrade"
                    />
                  </div>
                )}

                {targetMode === 'ids' ? (
                  <div className="field">
                    <label>
                      hosts ({selectedCount} selected · {agents.length} online)
                    </label>
                    <div
                      style={{
                        display: 'grid',
                        gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))',
                        gap: 6,
                        border: '1px solid var(--line)',
                        borderRadius: 'var(--r)',
                        padding: 10,
                        background: 'var(--bg)',
                      }}
                    >
                      {agents.map((a) => (
                        <label
                          key={a}
                          className="row"
                          style={{ gap: 6, fontSize: 11.5, color: 'var(--fg-1)' }}
                        >
                          <input
                            type="checkbox"
                            checked={!!selected[a]}
                            onChange={(e) =>
                              setSelected((prev) => ({ ...prev, [a]: e.target.checked }))
                            }
                          />
                          <span className="mono" style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
                            {a.replace(/-id$/, '')}
                          </span>
                        </label>
                      ))}
                    </div>
                    <div className="row" style={{ gap: 8, marginTop: 4 }}>
                      <button className="btn sm" onClick={() => toggleAll(true)}>
                        select all
                      </button>
                      <button className="btn sm" onClick={() => toggleAll(false)}>
                        clear
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="field">
                    <label>label</label>
                    {labels.length === 0 ? (
                      <span className="muted" style={{ fontSize: 11 }}>
                        No labels defined yet — add some on each agent&apos;s overview tab.
                      </span>
                    ) : (
                      <select
                        className="select"
                        value={labelChoice}
                        onChange={(e) => setLabelChoice(e.target.value)}
                      >
                        <option value="">— pick a label —</option>
                        {labels.map((l) => (
                          <option key={l} value={l}>
                            {l}
                          </option>
                        ))}
                      </select>
                    )}
                  </div>
                )}

                <div className="row between">
                  <div className="kbd-hint">
                    runs in parallel · timeout 30s per host
                  </div>
                  <button
                    type="button"
                    onClick={submit}
                    disabled={submitting}
                    className="btn primary"
                  >
                    {submitting ? '…' : `▶ run on ${targetMode === 'ids' ? `${selectedCount} hosts` : `label "${labelChoice || '—'}"`}`}
                  </button>
                </div>
              </div>
            </div>

            {run && (
              <div className="panel">
                <div className="panel-head">
                  <div className="panel-title">
                    <span className="ico">▤</span> RUN #{run.run.id}
                    <span className="meta">
                      {run.run.kind} · started {fmtTs(run.run.started_at)}
                    </span>
                  </div>
                </div>
                <div className="panel-body flush">
                  <table className="tbl">
                    <thead>
                      <tr>
                        <th style={{ width: 110 }}>STATUS</th>
                        <th>HOST</th>
                        <th>DETAIL</th>
                        <th style={{ width: 180 }}>FINISHED</th>
                      </tr>
                    </thead>
                    <tbody>
                      {run.results.map((r) => {
                        const cls =
                          r.status === 'success'
                            ? 'ok'
                            : r.status === 'failed'
                              ? 'err-c'
                              : r.status === 'pending'
                                ? 'warn-c'
                                : 'muted';
                        return (
                          <tr key={r.agent_id}>
                            <td>
                              <span className={`status ${cls}`}>
                                <span className="dot" />
                                {r.status}
                              </span>
                            </td>
                            <td className="mono">{r.agent_id.replace(/-id$/, '')}</td>
                            <td className="mono muted">{r.detail ?? '—'}</td>
                            <td className="mono muted" style={{ fontSize: 11 }}>
                              {fmtTs(r.finished_at)}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              </div>
            )}
          </div>
        </div>
      </main>
    </div>
  );
}
