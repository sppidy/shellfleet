'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { SystemStatsPayload } from '@/lib/types';
import { CpuIcon, MemoryStickIcon, HardDriveIcon, ClockIcon, AlertCircleIcon } from 'lucide-react';

const STATS_INTERVAL_MS = 5_000;
const STATS_TIMEOUT_MS = 10_000;

function formatBytes(kib: number): string {
  // /proc/meminfo and `df -k` are KiB. Convert to a human-readable size.
  const bytes = kib * 1024;
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

function formatUptime(secs: number): string {
  if (secs <= 0) return '—';
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3_600);
  const m = Math.floor((secs % 3_600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export default function SystemStats({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [stats, setStats] = useState<SystemStatsPayload | null>(null);
  // null = waiting for first response; true = old agent that doesn't reply.
  const [unsupported, setUnsupported] = useState(false);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setStats(null);
    setUnsupported(false);

    const unsubscribe = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'SystemStatsResponse') {
        if (timeoutRef.current) {
          clearTimeout(timeoutRef.current);
          timeoutRef.current = null;
        }
        setUnsupported(false);
        setStats(msg.payload);
      }
    });

    const request = () => {
      sendToAgent(agentId, { type: 'SystemStatsRequest' });
    };

    request();
    // Mark as unsupported if we never get a first response. Older agents
    // (protocol_version < 2) silently drop the message.
    timeoutRef.current = setTimeout(() => setUnsupported(true), STATS_TIMEOUT_MS);
    const interval = setInterval(request, STATS_INTERVAL_MS);

    return () => {
      unsubscribe();
      clearInterval(interval);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [agentId, sendToAgent, onAgentMessage]);

  if (unsupported && !stats) {
    return (
      <div className="flex items-center gap-2 text-xs text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-md px-3 py-2">
        <AlertCircleIcon className="w-3.5 h-3.5 shrink-0" />
        <span>
          This agent doesn&apos;t expose system stats. Upgrade it via{' '}
          <code className="bg-amber-500/20 px-1 py-0.5 rounded">apt install --only-upgrade sys-manager-agent</code>.
        </span>
      </div>
    );
  }

  if (!stats) {
    return (
      <div className="grid grid-cols-2 md:grid-cols-4 gap-2">
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            className="h-20 bg-slate-900/50 rounded-md animate-pulse"
          />
        ))}
      </div>
    );
  }

  const memUsed = stats.mem_total_kb - stats.mem_available_kb;
  const memPct = stats.mem_total_kb > 0 ? (memUsed / stats.mem_total_kb) * 100 : 0;
  const diskPct =
    stats.root_disk_total_kb > 0
      ? (stats.root_disk_used_kb / stats.root_disk_total_kb) * 100
      : 0;
  const loadPct = stats.cpu_count > 0 ? (stats.load_1 / stats.cpu_count) * 100 : 0;
  const swapUsed = stats.swap_total_kb - stats.swap_free_kb;

  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 md:grid-cols-4 gap-2">
        <StatCard
          icon={<CpuIcon className="w-4 h-4" />}
          label="Load (1m)"
          value={stats.load_1.toFixed(2)}
          sub={`${stats.cpu_count} CPU · ${stats.load_5.toFixed(2)} / ${stats.load_15.toFixed(2)}`}
          pct={loadPct}
        />
        <StatCard
          icon={<MemoryStickIcon className="w-4 h-4" />}
          label="Memory"
          value={`${memPct.toFixed(0)}%`}
          sub={`${formatBytes(memUsed)} / ${formatBytes(stats.mem_total_kb)}`}
          pct={memPct}
        />
        <StatCard
          icon={<HardDriveIcon className="w-4 h-4" />}
          label="Disk /"
          value={`${diskPct.toFixed(0)}%`}
          sub={`${formatBytes(stats.root_disk_used_kb)} / ${formatBytes(stats.root_disk_total_kb)}`}
          pct={diskPct}
        />
        <StatCard
          icon={<ClockIcon className="w-4 h-4" />}
          label="Uptime"
          value={formatUptime(stats.uptime_secs)}
          sub={`Linux ${stats.kernel}`}
        />
      </div>
      {stats.swap_total_kb > 0 && (
        <div className="text-[11px] text-slate-500">
          Swap: {formatBytes(swapUsed)} / {formatBytes(stats.swap_total_kb)}
        </div>
      )}
    </div>
  );
}

function StatCard({
  icon,
  label,
  value,
  sub,
  pct,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  sub: string;
  pct?: number;
}) {
  const tone =
    pct === undefined
      ? 'bg-slate-700'
      : pct >= 90
        ? 'bg-red-500'
        : pct >= 75
          ? 'bg-amber-500'
          : 'bg-emerald-500';
  return (
    <div className="bg-slate-900 border border-slate-800 rounded-md p-3 flex flex-col">
      <div className="flex items-center justify-between text-[11px] uppercase tracking-wide text-slate-500">
        <span className="flex items-center gap-1.5">
          <span className="text-slate-400">{icon}</span>
          {label}
        </span>
      </div>
      <div className="text-xl font-semibold text-slate-100 mt-1">{value}</div>
      <div className="text-[11px] text-slate-500 truncate" title={sub}>
        {sub}
      </div>
      {pct !== undefined && (
        <div className="mt-2 h-1 rounded-full bg-slate-800 overflow-hidden">
          <div
            className={`h-full transition-all ${tone}`}
            style={{ width: `${Math.min(100, Math.max(0, pct))}%` }}
          />
        </div>
      )}
    </div>
  );
}
