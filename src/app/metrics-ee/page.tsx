'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface PanelInfo { id: string; title: string; description: string | null; unit: string; source: string | null }
interface PanelsResponse { enabled: boolean; panels: PanelInfo[]; sources: string[] }

export default function MetricsEePage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [panels, setPanels] = useState<PanelsResponse | null>(null);
  const [agents, setAgents] = useState<string[]>([]);
  const [agent, setAgent] = useState('');
  const [range, setRange] = useState('1h');
  const [result, setResult] = useState<{ panel: string; series: unknown[]; query: string; source: string } | null>(null);
  const [querying, setQuerying] = useState(false);
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

  useEffect(() => { if (status === 'authed') load(); }, [status, load]);

  const runQuery = async (panel: PanelInfo) => {
    if (!agent) { setError('select a target agent first'); return; }
    setQuerying(true); setResult(null); setError(null);
    try {
      const res = await apiFetch('/api/ee/metrics/query', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ panel: panel.id, agent_id: agent, range, source: panel.source }),
      });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      const d = await res.json();
      setResult({ panel: panel.title, series: d.series || [], query: d.expanded_query || '', source: d.source || '' });
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
    finally { setQuerying(false); }
  };

  // Pull the latest [ts, value] from an opaque series object.
  const latest = (s: unknown): string => {
    const o = s as { values?: unknown[][]; target?: string };
    const vals = o?.values;
    if (Array.isArray(vals) && vals.length) {
      const last = vals[vals.length - 1];
      if (Array.isArray(last) && last.length >= 2) return Number(last[1]).toLocaleString(undefined, { maximumFractionDigits: 2 });
    }
    return '—';
  };
  // MUST return a string — Prometheus series carry `metric` as an OBJECT
  // ({__name__, instance, device, ...}); returning it raw and rendering it as
  // a React child throws ("Objects are not valid as a React child") and takes
  // the whole page down. Build a readable label from the distinctive labels.
  const seriesName = (s: unknown): string => {
    const o = (s ?? {}) as { target?: unknown; name?: unknown; metric?: unknown };
    if (typeof o.target === 'string' && o.target) return o.target;
    if (typeof o.name === 'string' && o.name) return o.name;
    const m = o.metric;
    if (m && typeof m === 'object') {
      const bits = Object.entries(m as Record<string, unknown>)
        .filter(([k]) => k !== '__name__' && k !== 'instance' && k !== 'job')
        .map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`)
        .sort();
      if (bits.length) return bits.join(', ');
      const name = (m as Record<string, unknown>).__name__;
      if (typeof name === 'string') return name;
    }
    return 'series';
  };

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/metrics-ee requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

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
            <select className="input" value={agent} onChange={(e) => setAgent(e.target.value)} style={{ width: 170 }}>
              <option value="">— agent —</option>
              {agents.map((a) => <option key={a} value={a}>{a.replace(/-id$/, '')}</option>)}
            </select>
            <select className="input" value={range} onChange={(e) => setRange(e.target.value)} style={{ width: 80 }}>
              <option value="15m">15m</option><option value="1h">1h</option><option value="6h">6h</option><option value="24h">24h</option>
            </select>
            <button className="btn" onClick={load}>↻</button>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="metrics-multi" label="EE Metrics (multi-source)">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {panels && !panels.enabled ? (
                <div className="panel" style={{ borderColor: 'var(--warn-bd)' }}>
                  <div className="panel-head"><div className="panel-title"><span className="ico">○</span> NOT CONFIGURED</div></div>
                  <div className="panel-body"><div className="mono muted" style={{ fontSize: 12 }}>
                    Multi-source metrics are licensed but no panels are defined. Point <span style={{ color: 'var(--fg-2)' }}>EE_METRICS_CONFIG_PATH</span> at a panel config (Prometheus / Datadog / New Relic sources) on the EE sidecar and refresh.
                  </div></div>
                </div>
              ) : (
                <>
                  <div className="panel" style={{ marginBottom: 12 }}>
                    <div className="panel-head"><div className="panel-title"><span className="ico">▤</span> PANELS{panels?.sources?.length ? <span className="meta">sources: {panels.sources.join(', ')}</span> : null}</div></div>
                    <div className="panel-body flush">
                      {panels === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                        : panels.panels.length === 0 ? <div className="empty">No panels defined.</div> : (
                        <table className="tbl"><thead><tr><th>PANEL</th><th>SOURCE</th><th>UNIT</th><th>DESCRIPTION</th><th style={{ width: 70 }}></th></tr></thead>
                          <tbody>{panels.panels.map((p) => (
                            <tr key={p.id}>
                              <td className="mono">{p.title}</td>
                              <td className="mono muted">{p.source || '—'}</td>
                              <td className="mono muted">{p.unit}</td>
                              <td className="mono muted" style={{ fontSize: 11 }}>{p.description || '—'}</td>
                              <td><button className="btn btn-sm btn-accent" disabled={!agent || querying} onClick={() => runQuery(p)}>query</button></td>
                            </tr>
                          ))}</tbody></table>
                      )}
                    </div>
                  </div>
                  {querying && <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /> querying…</div>}
                  {result && (
                    <div className="panel">
                      <div className="panel-head"><div className="panel-title"><span className="ico">▶</span> {result.panel} <span className="meta">{result.source}</span></div></div>
                      <div className="panel-body flush">
                        {result.series.length === 0 ? <div className="empty">No data returned.</div> : (
                          <table className="tbl"><thead><tr><th>SERIES</th><th>LATEST</th></tr></thead>
                            <tbody>{result.series.map((s, i) => (
                              <tr key={i}><td className="mono">{seriesName(s)}</td><td className="mono" style={{ color: 'var(--accent)' }}>{latest(s)}</td></tr>
                            ))}</tbody></table>
                        )}
                        {result.query && <div className="panel-body"><span className="mono muted" style={{ fontSize: 11, wordBreak: 'break-all' }}>query: {result.query}</span></div>}
                      </div>
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
