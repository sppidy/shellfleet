'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import type { DockerContainerStats } from '@/lib/types';
import {
  ActivityIcon,
  RefreshCwIcon,
  PauseIcon,
  PlayIcon,
  Loader2Icon,
  AlertCircleIcon,
  InfoIcon,
} from 'lucide-react';

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

  // Subscribe to responses + drive polling. Polling pauses when the
  // browser tab is hidden so an idle dashboard doesn't keep
  // hammering docker stats.
  useEffect(() => {
    setSnapshots(null);
    setError(null);
    setPaused(false);
    historyRef.current = {};

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
      // Append to history rings.
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
      if (visibleRef.current && !paused) {
        startPolling();
      } else {
        stopPolling();
      }
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
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <ActivityIcon className="w-5 h-5 text-slate-400" />
          <h2 className="text-base font-semibold">Container stats</h2>
          <span className="text-xs text-slate-500">
            {snapshots === null ? 'loading…' : `· ${snapshots.length}`}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setPaused((p) => !p)}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-md border border-slate-700"
            title={paused ? 'Resume polling' : 'Pause polling'}
          >
            {paused ? (
              <PlayIcon className="w-3.5 h-3.5" />
            ) : (
              <PauseIcon className="w-3.5 h-3.5" />
            )}
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button
            type="button"
            onClick={refresh}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-md"
          >
            <RefreshCwIcon className="w-3.5 h-3.5" />
            Refresh now
          </button>
        </div>
      </div>

      <div className="rounded-md border border-slate-800 bg-slate-900/40 p-3 text-xs text-slate-400 flex items-start gap-2">
        <InfoIcon className="w-4 h-4 mt-0.5 text-slate-500 shrink-0" />
        <div className="space-y-1">
          <p>
            Stats are pulled <strong>on demand</strong>. Each tick runs{' '}
            <code>docker stats --no-stream</code> on the agent — one short
            docker daemon call per visible host. The agent has no background
            polling loop; this view is the only thing that triggers it.
          </p>
          <p>
            Cadence: every 10 s while this tab is in focus. The polling
            pauses automatically when the browser tab is hidden, and the
            Pause button stops it manually. Sparklines are kept in memory
            only — there&apos;s no time-series store on the agent or server.
          </p>
        </div>
      </div>

      {error && (
        <div className="flex items-start gap-2 text-sm text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {snapshots === null ? (
        <div className="flex items-center justify-center py-12 text-slate-500">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      ) : snapshots.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          No running containers.
        </div>
      ) : (
        <>
          <div className="rounded-md border border-slate-800 overflow-hidden">
            <table className="w-full text-sm">
              <thead className="bg-slate-900/60 text-[11px] uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="text-left px-3 py-2 font-medium">Name</th>
                  <th className="text-right px-3 py-2 font-medium">CPU %</th>
                  <th className="text-right px-3 py-2 font-medium">Mem</th>
                  <th className="px-3 py-2"></th>
                  <th className="text-right px-3 py-2 font-medium">Net I/O (rx/tx)</th>
                  <th className="text-right px-3 py-2 font-medium">Blk I/O (r/w)</th>
                  <th className="text-right px-3 py-2 font-medium">PIDs</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-800">
                {snapshots.map((s) => {
                  const memPct =
                    s.mem_limit_bytes > 0 ? (s.mem_bytes / s.mem_limit_bytes) * 100 : 0;
                  const hist = historyRef.current[s.id];
                  return (
                    <tr key={s.id} className="bg-slate-900/30">
                      <td className="px-3 py-2 text-slate-200 truncate max-w-[20ch]" title={s.name}>
                        {s.name}
                      </td>
                      <td className="px-3 py-2 text-right text-slate-300">
                        {s.cpu_percent.toFixed(1)}
                      </td>
                      <td className="px-3 py-2 text-right text-slate-300">
                        <div>{fmtBytes(s.mem_bytes)} / {fmtBytes(s.mem_limit_bytes)}</div>
                        <div className="text-[10px] text-slate-500">{memPct.toFixed(1)}%</div>
                      </td>
                      <td className="px-3 py-2">
                        {hist && (
                          <div className="flex flex-col gap-0.5">
                            <Sparkline values={hist.cpu} max={100} color="#3b82f6" />
                            <Sparkline values={hist.mem} max={100} color="#10b981" />
                          </div>
                        )}
                      </td>
                      <td className="px-3 py-2 text-right text-xs text-slate-400">
                        {fmtBytes(s.net_rx_bytes)} / {fmtBytes(s.net_tx_bytes)}
                      </td>
                      <td className="px-3 py-2 text-right text-xs text-slate-400">
                        {fmtBytes(s.blk_read_bytes)} / {fmtBytes(s.blk_write_bytes)}
                      </td>
                      <td className="px-3 py-2 text-right text-slate-400">{s.pids}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          {lastFetchAt && (
            <p className="text-[11px] text-slate-500">
              Last fetch {Math.max(0, Math.floor((Date.now() - lastFetchAt) / 1000))}s ago.
              {paused && ' Polling paused.'}
            </p>
          )}
        </>
      )}
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
    return <div className="w-24 h-3" />;
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
    <svg width={W} height={H} className="block">
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
