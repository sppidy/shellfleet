'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import type { DockerContainerStats } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

const POLL_MS = 10_000;
const HISTORY_LEN = 12;

function fmtBytes(n: number): string {
  if (!n) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

type History = { cpu: number[]; mem: number[] };

export default function ContainerStats({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [snapshots, setSnapshots] = useState<DockerContainerStats[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [paused, setPaused] = useState(false);
  const [lastFetchAt, setLastFetchAt] = useState<number | null>(null);
  const historyRef = useRef<Record<string, History>>({});
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const visibleRef = useRef<boolean>(true);

  const refresh = useCallback(() => {
    sendToAgent(agentId, { type: 'DockerStatsRequest' });
  }, [agentId, sendToAgent]);

  useEffect(() => {
    setSnapshots(null);
    setError(null);
    historyRef.current = {};
  }, [agentId]);

  useEffect(() => {
    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type !== 'DockerStatsResponse') return;
      setLastFetchAt(Date.now());
      if (!msg.payload.available) {
        setError(msg.payload.error ?? 'docker not available');
        setSnapshots([]);
        return;
      }
      setError(msg.payload.error);
      setSnapshots(msg.payload.snapshots);
      for (const s of msg.payload.snapshots) {
        const memPct =
          s.mem_limit_bytes > 0 ? (s.mem_bytes / s.mem_limit_bytes) * 100 : 0;
        const h = historyRef.current[s.id] ?? { cpu: [], mem: [] };
        h.cpu = [...h.cpu.slice(-(HISTORY_LEN - 1)), s.cpu_percent];
        h.mem = [...h.mem.slice(-(HISTORY_LEN - 1)), memPct];
        historyRef.current[s.id] = h;
      }
    });

    const startPolling = () => {
      if (intervalRef.current) return;
      refresh();
      intervalRef.current = setInterval(() => {
        if (!paused && visibleRef.current) refresh();
      }, POLL_MS);
    };
    const stopPolling = () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };

    const onVisibility = () => {
      visibleRef.current = document.visibilityState === 'visible';
      if (visibleRef.current && !paused) startPolling();
      else stopPolling();
    };
    document.addEventListener('visibilitychange', onVisibility);
    visibleRef.current = document.visibilityState === 'visible';
    if (visibleRef.current) startPolling();

    return () => {
      unsub();
      document.removeEventListener('visibilitychange', onVisibility);
      stopPolling();
    };
  }, [agentId, onAgentMessage, refresh, paused]);

  return (
    <div className="pane">
      <div
        className="panel"
        style={{ background: 'var(--bg-2)', borderColor: 'var(--accent-bd)' }}
      >
        <div className="panel-body" style={{ fontSize: 11.5, color: 'var(--fg-1)' }}>
          <div style={{ marginBottom: 4, color: 'var(--accent)' }}>
            ▾ Cost banner
          </div>
          Stats are pulled <strong>on demand</strong>. Each tick runs{' '}
          <code style={{ background: 'rgba(0,0,0,0.2)', padding: '0 4px' }}>
            docker stats --no-stream
          </code>{' '}
          on the agent — one short docker daemon call per visible host. The agent
          has no background polling loop. Polling pauses when the tab is hidden;
          sparklines are kept in memory only.
        </div>
      </div>

      {error && (
        <div
          style={{
            padding: 10,
            background: 'var(--err-bg)',
            border: '1px solid var(--err-bd)',
            borderRadius: 'var(--r)',
            color: 'var(--err)',
            fontFamily: 'var(--mono)',
            fontSize: 11.5,
          }}
        >
          {error}
        </div>
      )}

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▥</span> CONTAINER STATS
            <span className="meta">
              {snapshots === null ? 'loading…' : `${snapshots.length} running · 10s tick`}
              {lastFetchAt &&
                ` · last fetch ${Math.max(0, Math.floor((Date.now() - lastFetchAt) / 1000))}s ago`}
              {paused && ' · paused'}
            </span>
          </div>
          <div className="panel-actions">
            <button className="btn" onClick={() => setPaused((p) => !p)}>
              {paused ? '▶ resume' : '❚❚ pause'}
            </button>
            <button className="btn" onClick={refresh}>
              ↻ refresh now
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {snapshots === null ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : snapshots.length === 0 ? (
            <div className="empty">No running containers.</div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>NAME</th>
                  <th className="right">CPU%</th>
                  <th>MEM</th>
                  <th>SPARK</th>
                  <th className="right">NET RX/TX</th>
                  <th className="right">BLK R/W</th>
                  <th className="right">PIDS</th>
                </tr>
              </thead>
              <tbody>
                {snapshots.map((s) => {
                  const memPct =
                    s.mem_limit_bytes > 0 ? (s.mem_bytes / s.mem_limit_bytes) * 100 : 0;
                  const hist = historyRef.current[s.id];
                  return (
                    <tr key={s.id}>
                      <td className="mono" style={{ color: 'var(--fg)' }} title={s.name}>
                        {s.name}
                      </td>
                      <td className="right mono">{s.cpu_percent.toFixed(1)}</td>
                      <td className="mono" style={{ fontSize: 11 }}>
                        {fmtBytes(s.mem_bytes)} / {fmtBytes(s.mem_limit_bytes)}{' '}
                        <span className="muted">({memPct.toFixed(1)}%)</span>
                      </td>
                      <td>
                        {hist && (
                          <div style={{ display: 'flex', flexDirection: 'column', gap: 1 }}>
                            <Sparkline values={hist.cpu} max={100} color="var(--info)" />
                            <Sparkline values={hist.mem} max={100} color="var(--accent)" />
                          </div>
                        )}
                      </td>
                      <td className="right mono">
                        {fmtBytes(s.net_rx_bytes)} / {fmtBytes(s.net_tx_bytes)}
                      </td>
                      <td className="right mono">
                        {fmtBytes(s.blk_read_bytes)} / {fmtBytes(s.blk_write_bytes)}
                      </td>
                      <td className="right mono">{s.pids}</td>
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

function Sparkline({
  values,
  max,
  color,
}: {
  values: number[];
  max: number;
  color: string;
}) {
  if (values.length < 2) {
    return <div style={{ width: 96, height: 12 }} />;
  }
  const W = 96;
  const H = 12;
  const xStep = W / (values.length - 1);
  const points = values
    .map((v, i) => {
      const clamped = Math.min(max, Math.max(0, v));
      const y = H - (clamped / max) * H;
      return `${(i * xStep).toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');
  return (
    <svg width={W} height={H} style={{ display: 'block' }}>
      <polyline
        points={points}
        fill="none"
        stroke={color}
        strokeWidth={1.2}
        strokeLinejoin="round"
      />
    </svg>
  );
}
