'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { Loader2Icon } from 'lucide-react';

type Range = '1h' | '6h' | '24h' | '7d';
type Unit = 'percent' | 'bytes' | 'bytes_per_sec' | 'cpu_seconds_per_sec' | 'raw';

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

interface Series {
  label: string;
  points: [number, number][];
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

function formatValue(v: number, unit: Unit): string {
  if (!Number.isFinite(v)) return '—';
  switch (unit) {
    case 'percent':
      return `${v.toFixed(1)}%`;
    case 'bytes':
      return fmtBytes(v);
    case 'bytes_per_sec':
      return `${fmtBytes(v)}/s`;
    case 'cpu_seconds_per_sec':
      // Effectively cores in use.
      return `${v.toFixed(2)} cpu`;
    default:
      // Compact-ish.
      if (Math.abs(v) >= 1000) return v.toFixed(0);
      return v.toFixed(2);
  }
}

function fmtBytes(n: number): string {
  if (!n) return '0 B';
  const sign = n < 0 ? '-' : '';
  let v = Math.abs(n);
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${sign}${v >= 10 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

function relTime(ts: number): string {
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (delta < 60) return `${delta}s`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m`;
  if (delta < 86_400) return `${Math.floor(delta / 3_600)}h`;
  return `${Math.floor(delta / 86_400)}d`;
}

/**
 * Sparkline-style multi-series line chart. Pure SVG, no chart lib —
 * matches the rest of the dashboard's "no fluff" aesthetic. Renders
 * up to ~8 series cleanly; more than that and the legend gets noisy
 * (which is what `topk()` in the panel query is for).
 */
function SeriesChart({
  series,
  unit,
  height = 120,
}: {
  series: Series[];
  unit: Unit;
  height?: number;
}) {
  const data = useMemo(() => {
    let xMin = Infinity;
    let xMax = -Infinity;
    let yMin = Infinity;
    let yMax = -Infinity;
    for (const s of series) {
      for (const [x, y] of s.points) {
        if (!Number.isFinite(y)) continue;
        if (x < xMin) xMin = x;
        if (x > xMax) xMax = x;
        if (y < yMin) yMin = y;
        if (y > yMax) yMax = y;
      }
    }
    if (!Number.isFinite(xMin) || !Number.isFinite(yMin)) {
      return null;
    }
    if (yMin === yMax) {
      yMin -= 1;
      yMax += 1;
    }
    if (xMin === xMax) xMax = xMin + 1;
    return { xMin, xMax, yMin, yMax };
  }, [series]);

  const colors = [
    'var(--accent)',
    '#82a8d4',
    '#e6b450',
    '#c885c4',
    '#6ec1c1',
    '#e57373',
    '#a8d5a0',
    '#d9a3d6',
  ];

  if (!data) {
    return (
      <div
        className="muted mono"
        style={{
          fontSize: 11,
          padding: '20px 0',
          textAlign: 'center',
        }}
      >
        no data in range
      </div>
    );
  }

  const W = 800;
  const H = height;
  const PAD_L = 4;
  const PAD_R = 4;
  const PAD_T = 6;
  const PAD_B = 6;
  const innerW = W - PAD_L - PAD_R;
  const innerH = H - PAD_T - PAD_B;

  const sx = (x: number) =>
    PAD_L + ((x - data.xMin) / (data.xMax - data.xMin)) * innerW;
  const sy = (y: number) =>
    PAD_T + (1 - (y - data.yMin) / (data.yMax - data.yMin)) * innerH;

  return (
    <div>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        style={{
          width: '100%',
          height,
          background: 'var(--bg-1)',
          borderRadius: 4,
          border: '1px solid var(--line)',
        }}
      >
        {/* baseline grid (top + bottom) */}
        <line
          x1={PAD_L}
          x2={W - PAD_R}
          y1={PAD_T}
          y2={PAD_T}
          stroke="var(--line)"
          strokeDasharray="2 4"
        />
        <line
          x1={PAD_L}
          x2={W - PAD_R}
          y1={H - PAD_B}
          y2={H - PAD_B}
          stroke="var(--line)"
          strokeDasharray="2 4"
        />
        {series.map((s, i) => {
          const path = s.points
            .filter(([, y]) => Number.isFinite(y))
            .map(([x, y], idx) => `${idx === 0 ? 'M' : 'L'} ${sx(x).toFixed(1)} ${sy(y).toFixed(1)}`)
            .join(' ');
          return (
            <path
              key={`${s.label}-${i}`}
              d={path}
              fill="none"
              stroke={colors[i % colors.length]}
              strokeWidth={1.25}
              vectorEffect="non-scaling-stroke"
            />
          );
        })}
      </svg>

      <div
        className="mono muted"
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontSize: 10,
          marginTop: 4,
          padding: '0 2px',
        }}
      >
        <span>{formatValue(data.yMin, unit)}</span>
        <span>{relTime(data.xMax)} ago … {relTime(data.xMin)} ago</span>
        <span>{formatValue(data.yMax, unit)}</span>
      </div>

      {/* Legend (only if multiple series). */}
      {series.length > 1 && (
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: '4px 12px',
            marginTop: 6,
            fontSize: 10.5,
            fontFamily: 'var(--mono)',
            color: 'var(--fg-2)',
          }}
        >
          {series.map((s, i) => (
            <span
              key={`${s.label}-${i}`}
              style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}
            >
              <span
                style={{
                  display: 'inline-block',
                  width: 8,
                  height: 8,
                  borderRadius: 2,
                  background: colors[i % colors.length],
                }}
              />
              {s.label || '(unnamed)'}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

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
