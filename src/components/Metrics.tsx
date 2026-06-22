'use client';

import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { Loader2Icon } from 'lucide-react';
import { SeriesChart, type Range, type Series, type Unit } from './SeriesChart';

interface PanelInfo {
  id: string;
  title: string;
  description?: string;
  unit: Unit;
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

function PanelCard({
  agentId,
  panel,
  range,
  source,
}: {
  agentId: string;
  panel: PanelInfo;
  range: Range;
  source?: string;
}) {
  const [data, setData] = useState<QueryResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    apiFetch('/api/metrics/query', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ panel: panel.id, agent_id: agentId, range, ...(source ? { source } : {}) }),
    })
      .then(async (res) => {
        if (cancelled) return;
        if (!res.ok) {
          const text = await res.text().catch(() => `HTTP ${res.status}`);
          throw new Error(text || `HTTP ${res.status}`);
        }
        const j = (await res.json()) as QueryResponse;
        if (cancelled) return;
        setData(j);
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : 'failed');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [agentId, panel.id, range, source]);

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">▤</span> {panel.title.toUpperCase()}
          {panel.description && <span className="meta">{panel.description}</span>}
        </div>
      </div>
      <div className="panel-body" style={{ padding: 12 }}>
        {loading ? (
          <div style={{ display: 'flex', justifyContent: 'center', padding: 16 }}>
            <Loader2Icon className="w-4 h-4 animate-spin" />
          </div>
        ) : error ? (
          <div className="mono" style={{ color: 'var(--err)', fontSize: 11 }}>
            {error}
          </div>
        ) : data && data.upstream_status !== 'success' ? (
          <div className="mono" style={{ color: 'var(--warn)', fontSize: 11 }}>
            prometheus error: {data.upstream_error ?? 'unknown'}
          </div>
        ) : data ? (
          <>
            <SeriesChart series={data.series} unit={data.unit} />
            <details style={{ marginTop: 8 }}>
              <summary
                className="muted"
                style={{ cursor: 'pointer', fontSize: 10.5, fontFamily: 'var(--mono)' }}
              >
                query
              </summary>
              <pre
                className="code"
                style={{
                  marginTop: 4,
                  fontSize: 10.5,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                }}
              >
                {data.expanded_query}
              </pre>
            </details>
          </>
        ) : null}
      </div>
    </div>
  );
}

export default function Metrics({ agentId }: { agentId: string }) {
  const [panels, setPanels] = useState<PanelInfo[] | null>(null);
  const [pluginEnabled, setPluginEnabled] = useState<boolean | null>(null);
  const [range, setRange] = useState<Range>('1h');
  const [sources, setSources] = useState<string[]>([]);
  const [activeSource, setActiveSource] = useState<string>('');
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/metrics/panels');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as PanelsResponse;
      setPluginEnabled(data.enabled);
      setPanels(data.panels);
      if (data.sources && data.sources.length > 0) {
        setSources(data.sources);
        if (!activeSource) setActiveSource(data.sources[0]);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
      setPluginEnabled(false);
      setPanels([]);
    }
  }, [activeSource]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (panels === null) {
    return (
      <div className="pane">
        <div className="empty">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      </div>
    );
  }

  if (!pluginEnabled || panels.length === 0) {
    return (
      <div className="pane">
        <div
          className="panel"
          style={{ borderColor: 'var(--warn-bd)' }}
        >
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">▤</span> METRICS
              <span className="meta">plugin disabled</span>
            </div>
          </div>
          <div className="panel-body" style={{ padding: 16, fontSize: 12 }}>
            <div className="muted" style={{ fontFamily: 'var(--mono)', marginBottom: 8 }}>
              {pluginEnabled
                ? 'No panels configured.'
                : 'Metrics plugin not configured on this server.'}
            </div>
            <div style={{ fontFamily: 'var(--mono)', lineHeight: 1.6 }}>
              ShellFleet doesn&apos;t store time-series — point this at
              your existing Prometheus and configure named panel
              templates. See{' '}
              <a
                href="https://github.com/sppidy/shellfleet/blob/main/docs/METRICS.md"
                target="_blank"
                rel="noreferrer"
                style={{ color: 'var(--accent)' }}
              >
                docs/METRICS.md
              </a>{' '}
              for the YAML schema and a worked example using
              process_exporter.
            </div>
            {error && (
              <div className="mono" style={{ color: 'var(--err)', fontSize: 11, marginTop: 8 }}>
                {error}
              </div>
            )}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="pane">
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▤</span> METRICS
            <span className="meta">
              {panels.length} panel{panels.length === 1 ? '' : 's'} · prometheus plugin
            </span>
          </div>
          <div className="panel-actions">
            <div className="seg">
              {RANGES.map((r) => (
                <button
                  key={r}
                  className={range === r ? 'on' : ''}
                  onClick={() => setRange(r)}
                >
                  {r}
                </button>
              ))}
            </div>
            {sources.length > 1 && (
              <select
                className="input"
                value={activeSource}
                onChange={(e) => setActiveSource(e.target.value)}
                style={{ fontSize: 11, padding: '3px 6px', height: 26 }}
              >
                {sources.map((s) => (
                  <option key={s} value={s}>{s}</option>
                ))}
              </select>
            )}
            <button className="btn sm" onClick={refresh} title="Refresh panel list">
              ↻
            </button>
          </div>
        </div>
      </div>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(420px, 1fr))',
          gap: 10,
          marginTop: 10,
        }}
      >
        {panels.map((p) => (
          <PanelCard key={p.id} agentId={agentId} panel={p} range={range} source={activeSource || undefined} />
        ))}
      </div>
    </div>
  );
}
