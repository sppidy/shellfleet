'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';
import { SeriesChart, type Series, type Unit } from '@/components/SeriesChart';
import MetricsPanelForm, { type EditingPanel } from '@/components/MetricsPanelForm';
import { panelMatchesSource, makePollGate } from '@/lib/metricsClient';

interface PanelInfo { id: string; title: string; description: string | null; unit: string; source: string | null; builtin: boolean; query?: string }
interface PanelsResponse { enabled: boolean; panels: PanelInfo[]; sources: string[] }
interface QueryResp { series: Series[]; unit: string; expanded_query: string; source: string; upstream_status: string; upstream_error: string | null }

const REFRESH_OPTS: { label: string; ms: number }[] = [
  { label: 'off', ms: 0 }, { label: '10s', ms: 10_000 }, { label: '30s', ms: 30_000 }, { label: '60s', ms: 60_000 },
];

// One auto-querying chart card. Re-queries on agent/source/range change and on
// each `tick`; skips a tick while a request is in flight (poll gate) and aborts
// a superseded request (AbortController).
function PanelCard({ agent, panel, range, tick, onEdit, onDelete }: {
  agent: string; panel: PanelInfo; range: string; tick: number;
  onEdit: (p: PanelInfo) => void; onDelete: (p: PanelInfo) => void;
}) {
  const [data, setData] = useState<QueryResp | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const gate = useRef(makePollGate());

  useEffect(() => {
    if (!gate.current.shouldRun()) return; // a previous poll is still running
    const ctrl = new AbortController();
    gate.current.start();
    setLoading(true);
    apiFetch('/api/ee/metrics/query', {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ panel: panel.id, agent_id: agent, range, source: panel.source }),
      signal: ctrl.signal,
    })
      .then(async (res) => {
        if (!res.ok) throw new Error((await res.text().catch(() => `HTTP ${res.status}`)) || `HTTP ${res.status}`);
        setData(await res.json() as QueryResp); setError(null);
      })
      .catch((e) => { if (e?.name !== 'AbortError') setError(e instanceof Error ? e.message : 'failed'); })
      .finally(() => { gate.current.done(); setLoading(false); });
    return () => ctrl.abort();
  }, [agent, panel.id, panel.source, range, tick]);

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">▤</span> {panel.title.toUpperCase()}
          {panel.source && <span className="meta">{panel.source}</span>}
        </div>
        {!panel.builtin && (
          <div style={{ display: 'flex', gap: 4 }}>
            <button className="btn btn-sm" title="Edit" onClick={() => onEdit(panel)}>✎</button>
            <button className="btn btn-sm" title="Delete" style={{ color: 'var(--err)' }} onClick={() => onDelete(panel)}>✕</button>
          </div>
        )}
      </div>
      <div className="panel-body" style={{ padding: 12 }}>
        {loading && !data ? (
          <div style={{ display: 'flex', justifyContent: 'center', padding: 16 }}><Loader2Icon className="w-4 h-4 animate-spin" /></div>
        ) : error ? (
          <div className="mono" style={{ color: 'var(--err)', fontSize: 11 }}>{error}</div>
        ) : data && data.upstream_status !== 'success' ? (
          <div className="mono" style={{ color: 'var(--warn)', fontSize: 11 }}>source error: {data.upstream_error ?? 'unknown'}</div>
        ) : data ? (
          <>
            <SeriesChart series={data.series} unit={(data.unit as Unit) || (panel.unit as Unit)} />
            <details style={{ marginTop: 8 }}>
              <summary className="muted" style={{ cursor: 'pointer', fontSize: 10.5, fontFamily: 'var(--mono)' }}>query</summary>
              <pre className="code" style={{ marginTop: 4, fontSize: 10.5, whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>{data.expanded_query}</pre>
            </details>
          </>
        ) : null}
      </div>
    </div>
  );
}

export default function MetricsEePage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [panels, setPanels] = useState<PanelsResponse | null>(null);
  const [agents, setAgents] = useState<string[]>([]);
  const [agent, setAgent] = useState('');
  const [range, setRange] = useState('1h');
  const [refreshMs, setRefreshMs] = useState(0);
  const [sourceFilter, setSourceFilter] = useState('');
  const [showForm, setShowForm] = useState(false);
  const [editing, setEditing] = useState<EditingPanel | null>(null);
  const [tick, setTick] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [p, t] = await Promise.all([apiFetch('/api/ee/metrics/panels'), apiFetch('/api/tokens')]);
      if (p.ok) setPanels(await p.json()); else { setError(`HTTP ${p.status}`); setPanels({ enabled: false, panels: [], sources: [] }); }
      if (t.ok) { const toks: { hostname?: string }[] = await t.json(); setAgents(toks.filter((x) => x.hostname).map((x) => `${x.hostname}-id`)); }
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setPanels({ enabled: false, panels: [], sources: [] }); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  // Page-level shared tick: one interval drives every card. Pauses on a hidden
  // tab (and refreshes immediately when it becomes visible again).
  useEffect(() => {
    if (refreshMs <= 0) return;
    let id: ReturnType<typeof setInterval> | null = null;
    const startTimer = () => { if (!id) id = setInterval(() => setTick((n) => n + 1), refreshMs); };
    const stopTimer = () => { if (id) { clearInterval(id); id = null; } };
    const onVis = () => {
      if (document.visibilityState === 'hidden') stopTimer();
      else { setTick((n) => n + 1); startTimer(); }
    };
    if (document.visibilityState === 'visible') startTimer();
    document.addEventListener('visibilitychange', onVis);
    return () => { stopTimer(); document.removeEventListener('visibilitychange', onVis); };
  }, [refreshMs]);

  const openNew = () => { setEditing(null); setShowForm(true); };
  const openEdit = (p: PanelInfo) => {
    setEditing({ id: p.id, title: p.title, query: p.query ?? '', unit: p.unit, source: p.source });
    setShowForm(true);
  };
  const remove = async (p: PanelInfo) => {
    if (!confirm(`Delete custom panel "${p.title}"?`)) return;
    try { await apiFetch(`/api/ee/metrics/panels/${p.id}`, { method: 'DELETE' }); await load(); } catch { /* ignore */ }
  };
  const onSaved = async () => { setShowForm(false); setEditing(null); await load(); };

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/metrics-ee requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  const visible = (panels?.panels ?? []).filter((p) => panelMatchesSource(p, sourceFilter));

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">ee metrics (multi-source)</span>
          </div>
          <div className="topbar-actions">
            <select className="input" value={agent} onChange={(e) => setAgent(e.target.value)} style={{ width: 160 }}>
              <option value="">— agent —</option>
              {agents.map((a) => <option key={a} value={a}>{a.replace(/-id$/, '')}</option>)}
            </select>
            <div className="seg">
              {['1h', '6h', '24h', '7d'].map((r) => <button key={r} className={range === r ? 'on' : ''} onClick={() => setRange(r)}>{r}</button>)}
            </div>
            <select className="input" value={refreshMs} onChange={(e) => setRefreshMs(Number(e.target.value))} style={{ width: 90 }} title="Auto-refresh">
              {REFRESH_OPTS.map((o) => <option key={o.ms} value={o.ms}>{o.ms === 0 ? '↻ off' : `↻ ${o.label}`}</option>)}
            </select>
            {refreshMs > 0 && <span className="mono" style={{ fontSize: 11, color: 'var(--accent)' }}>● live</span>}
            <button className="btn btn-accent" onClick={openNew}>+ panel</button>
            <button className="btn" onClick={load} title="Reload panels">↻</button>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="metrics-multi" label="EE Metrics (multi-source)">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {showForm && (
                <MetricsPanelForm sources={panels?.sources ?? []} agents={agents} editing={editing} onSaved={onSaved} onCancel={() => { setShowForm(false); setEditing(null); }} />
              )}
              {panels && !panels.enabled ? (
                <div className="panel" style={{ borderColor: 'var(--warn-bd)' }}>
                  <div className="panel-head"><div className="panel-title"><span className="ico">○</span> NOT CONFIGURED</div></div>
                  <div className="panel-body"><div className="mono muted" style={{ fontSize: 12 }}>
                    Multi-source metrics are licensed but no sources are defined. Point <span style={{ color: 'var(--fg-2)' }}>EE_METRICS_CONFIG_PATH</span> at a panel config on the EE sidecar and refresh.
                  </div></div>
                </div>
              ) : panels === null ? (
                <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
              ) : (
                <>
                  {(panels.sources?.length ?? 0) > 1 && (
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
                      <span className="mono muted" style={{ fontSize: 11 }}>source:</span>
                      <select className="input" value={sourceFilter} onChange={(e) => setSourceFilter(e.target.value)} style={{ width: 160 }}>
                        <option value="">all</option>
                        {panels.sources.map((s) => <option key={s} value={s}>{s}</option>)}
                      </select>
                    </div>
                  )}
                  {!agent ? (
                    <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 12 }}>Select a target agent to render {visible.length} panel{visible.length === 1 ? '' : 's'}.</div></div></div>
                  ) : visible.length === 0 ? (
                    <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 12 }}>No panels{sourceFilter ? ` for source “${sourceFilter}”` : ''}. Use “+ panel” to add one.</div></div></div>
                  ) : (
                    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(420px, 1fr))', gap: 10 }}>
                      {visible.map((p) => <PanelCard key={p.id} agent={agent} panel={p} range={range} tick={tick} onEdit={openEdit} onDelete={remove} />)}
                    </div>
                  )}
                </>
              )}
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
