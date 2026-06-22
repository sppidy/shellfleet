'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { Loader2Icon } from 'lucide-react';
import { SeriesChart, type Range, type Series, type Unit } from './SeriesChart';
import { useSession } from './providers/SessionProvider';
import { panelMatchesSource, makePollGate } from '@/lib/metricsClient';

interface PanelInfo {
  id: string;
  title: string;
  description?: string;
  unit: Unit;
  source: string | null;
}

interface PanelsResponse {
  enabled: boolean;
  panels: PanelInfo[];
  sources?: string[];
}

interface QueryResponse {
  panel_id: string;
  title: string;
  unit: Unit;
  series: Series[];
  expanded_query: string;
  upstream_status: string;
  upstream_error: string | null;
}

const RANGES: Range[] = ['1h', '6h', '24h', '7d'];
const REFRESH_OPTS: { label: string; ms: number }[] = [
  { label: 'off', ms: 0 }, { label: '10s', ms: 10_000 }, { label: '30s', ms: 30_000 }, { label: '60s', ms: 60_000 },
];

function PanelCard({
  agentId, panel, range, source, tick,
}: {
  agentId: string;
  panel: PanelInfo;
  range: Range;
  source?: string;
  tick: number;
}) {
  const [data, setData] = useState<QueryResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const gate = useRef(makePollGate());

  useEffect(() => {
    if (!gate.current.shouldRun()) return;
    const ctrl = new AbortController();
    gate.current.start();
    setLoading(true);
    apiFetch('/api/metrics/query', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ panel: panel.id, agent_id: agentId, range, ...(source ? { source } : {}) }),
      signal: ctrl.signal,
    })
      .then(async (res) => {
        if (!res.ok) throw new Error((await res.text().catch(() => `HTTP ${res.status}`)) || `HTTP ${res.status}`);
        setData((await res.json()) as QueryResponse);
        setError(null);
      })
      .catch((e) => { if (e?.name !== 'AbortError') setError(e instanceof Error ? e.message : 'failed'); })
      .finally(() => { gate.current.done(); setLoading(false); });
    return () => ctrl.abort();
  }, [agentId, panel.id, range, source, tick]);

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">▤</span> {panel.title.toUpperCase()}
          {panel.source && <span className="meta">{panel.source}</span>}
        </div>
      </div>
      <div className="panel-body" style={{ padding: 12 }}>
        {loading && !data ? (
          <div style={{ display: 'flex', justifyContent: 'center', padding: 16 }}>
            <Loader2Icon className="w-4 h-4 animate-spin" />
          </div>
        ) : error ? (
          <div className="mono" style={{ color: 'var(--err)', fontSize: 11 }}>{error}</div>
        ) : data && data.upstream_status !== 'success' ? (
          <div className="mono" style={{ color: 'var(--warn)', fontSize: 11 }}>
            source error: {data.upstream_error ?? 'unknown'}
          </div>
        ) : data ? (
          <>
            <SeriesChart series={data.series} unit={data.unit} />
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

export default function Metrics({ agentId }: { agentId: string }) {
  const { role } = useSession();
  const [panels, setPanels] = useState<PanelInfo[] | null>(null);
  const [pluginEnabled, setPluginEnabled] = useState<boolean | null>(null);
  const [range, setRange] = useState<Range>('1h');
  const [sources, setSources] = useState<string[]>([]);
  const [sourceFilter, setSourceFilter] = useState<string>('');
  const [refreshMs, setRefreshMs] = useState(0);
  const [tick, setTick] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/metrics/panels');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as PanelsResponse;
      setPluginEnabled(data.enabled);
      setPanels(data.panels);
      if (data.sources && data.sources.length > 0) setSources(data.sources);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
      setPluginEnabled(false);
      setPanels([]);
    }
  }, []);

  useEffect(() => { if (role === 'admin') void refresh(); }, [refresh, role]);

  // Page-level shared tick; pauses on a hidden tab.
  useEffect(() => {
    if (refreshMs <= 0 || role !== 'admin') return;
    let id: ReturnType<typeof setInterval> | null = null;
    const start = () => { if (!id) id = setInterval(() => setTick((n) => n + 1), refreshMs); };
    const stop = () => { if (id) { clearInterval(id); id = null; } };
    const onVis = () => { if (document.visibilityState === 'hidden') stop(); else { setTick((n) => n + 1); start(); } };
    if (document.visibilityState === 'visible') start();
    document.addEventListener('visibilitychange', onVis);
    return () => { stop(); document.removeEventListener('visibilitychange', onVis); };
  }, [refreshMs, role]);

  // Metrics querying is admin-only (POST /api/metrics/query is admin-gated by
  // CE RBAC). Gate the tab so viewers don't see charts that 403.
  if (role !== 'admin') {
    return (
      <div className="pane">
        <div className="panel" style={{ borderColor: 'var(--warn-bd)' }}>
          <div className="panel-head"><div className="panel-title"><span className="ico">▤</span> METRICS<span className="meta">admin only</span></div></div>
          <div className="panel-body" style={{ padding: 16 }}>
            <div className="mono muted" style={{ fontSize: 12 }}>Metrics querying requires the admin role.</div>
          </div>
        </div>
      </div>
    );
  }

  if (panels === null) {
    return <div className="pane"><div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div></div>;
  }

  if (!pluginEnabled || panels.length === 0) {
    return (
      <div className="pane">
        <div className="panel" style={{ borderColor: 'var(--warn-bd)' }}>
          <div className="panel-head">
            <div className="panel-title"><span className="ico">▤</span> METRICS<span className="meta">plugin disabled</span></div>
          </div>
          <div className="panel-body" style={{ padding: 16, fontSize: 12 }}>
            <div className="muted" style={{ fontFamily: 'var(--mono)', marginBottom: 8 }}>
              {pluginEnabled ? 'No panels configured.' : 'Metrics plugin not configured on this server.'}
            </div>
            <div style={{ fontFamily: 'var(--mono)', lineHeight: 1.6 }}>
              ShellFleet doesn&apos;t store time-series — point this at your existing Prometheus (or another source) and
              configure named panel templates. See{' '}
              <a href="https://github.com/sppidy/shellfleet/blob/main/docs/METRICS.md" target="_blank" rel="noreferrer" style={{ color: 'var(--accent)' }}>docs/METRICS.md</a>.
            </div>
            {error && <div className="mono" style={{ color: 'var(--err)', fontSize: 11, marginTop: 8 }}>{error}</div>}
          </div>
        </div>
      </div>
    );
  }

  const visible = panels.filter((p) => panelMatchesSource(p, sourceFilter));

  return (
    <div className="pane">
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▤</span> METRICS
            <span className="meta">{panels.length} panel{panels.length === 1 ? '' : 's'}</span>
          </div>
          <div className="panel-actions">
            <div className="seg">
              {RANGES.map((r) => <button key={r} className={range === r ? 'on' : ''} onClick={() => setRange(r)}>{r}</button>)}
            </div>
            <select className="input" value={refreshMs} onChange={(e) => setRefreshMs(Number(e.target.value))} style={{ fontSize: 11, padding: '3px 6px', height: 26 }} title="Auto-refresh">
              {REFRESH_OPTS.map((o) => <option key={o.ms} value={o.ms}>{o.ms === 0 ? '↻ off' : `↻ ${o.label}`}</option>)}
            </select>
            {refreshMs > 0 && <span className="mono" style={{ fontSize: 11, color: 'var(--accent)' }}>● live</span>}
            {sources.length > 1 && (
              <select className="input" value={sourceFilter} onChange={(e) => setSourceFilter(e.target.value)} style={{ fontSize: 11, padding: '3px 6px', height: 26 }} title="Filter by source">
                <option value="">all sources</option>
                {sources.map((s) => <option key={s} value={s}>{s}</option>)}
              </select>
            )}
            <button className="btn sm" onClick={refresh} title="Refresh panel list">↻</button>
          </div>
        </div>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(420px, 1fr))', gap: 10, marginTop: 10 }}>
        {visible.map((p) => (
          <PanelCard key={p.id} agentId={agentId} panel={p} range={range} source={sourceFilter || undefined} tick={tick} />
        ))}
      </div>
    </div>
  );
}
