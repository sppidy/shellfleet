'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';
import { SeriesChart, type Range, type Series, type Unit } from '@/components/SeriesChart';

interface PanelInfo { id: string; title: string; description: string | null; unit: string; source: string | null }
interface PanelsResponse { enabled: boolean; panels: PanelInfo[]; sources: string[] }
interface QueryResponse {
  panel_id: string;
  title: string;
  unit: string;
  series: Series[];
  expanded_query: string;
  source: string;
  upstream_status: string;
  upstream_error: string | null;
}

const RANGES: Range[] = ['1h', '6h', '24h', '7d'];

// One panel = one auto-querying chart card. Re-fetches whenever the target
// agent or range changes (deps), so a single shared selector drives them all.
function PanelCard({ agent, panel, range }: { agent: string; panel: PanelInfo; range: Range }) {
  const [data, setData] = useState<QueryResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    apiFetch('/api/ee/metrics/query', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ panel: panel.id, agent_id: agent, range, source: panel.source }),
    })
      .then(async (res) => {
        if (cancelled) return;
        if (!res.ok) { const t = await res.text().catch(() => `HTTP ${res.status}`); throw new Error(t || `HTTP ${res.status}`); }
        const j = (await res.json()) as QueryResponse;
        if (!cancelled) setData(j);
      })
      .catch((e) => { if (!cancelled) setError(e instanceof Error ? e.message : 'failed'); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [agent, panel.id, panel.source, range]);

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">▤</span> {panel.title.toUpperCase()}
          {panel.source && <span className="meta">{panel.source}</span>}
        </div>
      </div>
      <div className="panel-body" style={{ padding: 12 }}>
        {loading ? (
          <div style={{ display: 'flex', justifyContent: 'center', padding: 16 }}><Loader2Icon className="w-4 h-4 animate-spin" /></div>
        ) : error ? (
          <div className="mono" style={{ color: 'var(--err)', fontSize: 11 }}>{error}</div>
        ) : data && data.upstream_status !== 'success' ? (
          <div className="mono" style={{ color: 'var(--warn)', fontSize: 11 }}>source error: {data.upstream_error ?? 'unknown'}</div>
        ) : data ? (
          <>
            <SeriesChart series={data.series} unit={data.unit as Unit} />
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
  const [range, setRange] = useState<Range>('1h');
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
            <div className="seg">
              {RANGES.map((r) => <button key={r} className={range === r ? 'on' : ''} onClick={() => setRange(r)}>{r}</button>)}
            </div>
            <button className="btn" onClick={load} title="Refresh panel list">↻</button>
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
              ) : panels === null ? (
                <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
              ) : panels.panels.length === 0 ? (
                <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 12 }}>No panels defined.{panels.sources?.length ? ` Sources: ${panels.sources.join(', ')}.` : ''}</div></div></div>
              ) : !agent ? (
                <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 12 }}>Select a target agent above to render {panels.panels.length} panel{panels.panels.length === 1 ? '' : 's'}{panels.sources?.length ? ` · sources: ${panels.sources.join(', ')}` : ''}.</div></div></div>
              ) : (
                <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(420px, 1fr))', gap: 10 }}>
                  {panels.panels.map((p) => <PanelCard key={p.id} agent={agent} panel={p} range={range} />)}
                </div>
              )}
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
