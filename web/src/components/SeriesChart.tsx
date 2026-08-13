'use client';

import { useMemo } from 'react';

export type Range = '15m' | '1h' | '6h' | '24h' | '7d';
export type Unit = 'percent' | 'bytes' | 'bytes_per_sec' | 'cpu_seconds_per_sec' | 'raw';

export interface Series {
  label: string;
  points: [number, number][];
}

export function formatValue(v: number, unit: Unit): string {
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

export function fmtBytes(n: number): string {
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
export function SeriesChart({
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
