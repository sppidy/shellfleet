'use client';

import { useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useSession } from '@/components/providers/SessionProvider';
import {
  ArrowLeftIcon,
  ServerIcon,
  CpuIcon,
  MemoryStickIcon,
  HardDriveIcon,
  AlertTriangleIcon,
  BoxIcon,
  RefreshCwIcon,
  Loader2Icon,
} from 'lucide-react';
import {
  DockerListPayload,
  ServiceInfo,
  SystemStatsPayload,
} from '@/lib/types';

type AgentSnapshot = {
  agentId: string;
  hostname: string;
  stats?: SystemStatsPayload;
  services?: ServiceInfo[];
  docker?: DockerListPayload;
};

const POLL_MS = 5_000;

function formatBytes(kib: number): string {
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
  if (!secs) return '—';
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3_600);
  const m = Math.floor((secs % 3_600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export default function OverviewPage() {
  const router = useRouter();
  const { agents, sendToAgent, onAgentMessage, isConnected } = useWebSocket();
  const { status } = useSession();
  const [snapshots, setSnapshots] = useState<Record<string, AgentSnapshot>>({});

  const agentsKey = agents.join(',');

  // Subscribe + poll. Each agent gets its own subscription so messages are
  // routed correctly without the previous "lastAgentMessage" race.
  useEffect(() => {
    if (status !== 'authed') return;
    const unsubs: Array<() => void> = [];

    const seedSnap = (agentId: string): AgentSnapshot => ({
      agentId,
      hostname: agentId.replace(/-id$/, ''),
    });

    setSnapshots((prev) => {
      const next: Record<string, AgentSnapshot> = {};
      for (const a of agents) next[a] = prev[a] ?? seedSnap(a);
      return next;
    });

    for (const agentId of agents) {
      const unsub = onAgentMessage(agentId, (msg) => {
        if (msg.type === 'SystemStatsResponse') {
          setSnapshots((prev) => ({
            ...prev,
            [agentId]: { ...(prev[agentId] ?? seedSnap(agentId)), stats: msg.payload },
          }));
        } else if (msg.type === 'ListServicesResponse') {
          setSnapshots((prev) => ({
            ...prev,
            [agentId]: {
              ...(prev[agentId] ?? seedSnap(agentId)),
              services: msg.payload.services,
            },
          }));
        } else if (msg.type === 'DockerListResponse') {
          setSnapshots((prev) => ({
            ...prev,
            [agentId]: {
              ...(prev[agentId] ?? seedSnap(agentId)),
              docker: msg.payload,
            },
          }));
        }
      });
      unsubs.push(unsub);
    }

    const poll = () => {
      for (const agentId of agents) {
        sendToAgent(agentId, { type: 'SystemStatsRequest' });
        sendToAgent(agentId, { type: 'ListServicesRequest' });
        sendToAgent(agentId, { type: 'DockerListRequest' });
      }
    };
    poll();
    const interval = setInterval(poll, POLL_MS);

    return () => {
      clearInterval(interval);
      for (const u of unsubs) u();
    };
    // agentsKey ensures we re-subscribe whenever the agent set actually changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentsKey, status]);

  const totals = useMemo(() => {
    let cpu = 0;
    let memTotal = 0;
    let memUsed = 0;
    let diskTotal = 0;
    let diskUsed = 0;
    let svcTotal = 0;
    let svcFailed = 0;
    let containers = 0;
    let containersRunning = 0;
    for (const s of Object.values(snapshots)) {
      if (s.stats) {
        cpu += s.stats.cpu_count;
        memTotal += s.stats.mem_total_kb;
        memUsed += s.stats.mem_total_kb - s.stats.mem_available_kb;
        diskTotal += s.stats.root_disk_total_kb;
        diskUsed += s.stats.root_disk_used_kb;
      }
      if (s.services) {
        svcTotal += s.services.length;
        svcFailed += s.services.filter((x) => x.active_state === 'failed').length;
      }
      if (s.docker?.available) {
        containers += s.docker.containers.length;
        containersRunning += s.docker.containers.filter((c) => c.state === 'running').length;
      }
    }
    return {
      cpu,
      memTotal,
      memUsed,
      diskTotal,
      diskUsed,
      svcTotal,
      svcFailed,
      containers,
      containersRunning,
    };
  }, [snapshots]);

  if (status === 'loading') {
    return (
      <div className="min-h-screen flex items-center justify-center text-slate-500">
        <Loader2Icon className="w-6 h-6 animate-spin" />
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 flex flex-col">
      <header className="border-b border-slate-800 px-6 py-4 flex items-center justify-between">
        <button
          type="button"
          onClick={() => router.push('/')}
          className="inline-flex items-center text-sm text-slate-400 hover:text-slate-100"
        >
          <ArrowLeftIcon className="w-4 h-4 mr-1.5" />
          Back to dashboard
        </button>
        <div className="flex items-center gap-3 text-xs text-slate-500">
          <span>{agents.length} agents</span>
          <span
            className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full border ${
              isConnected
                ? 'border-emerald-500/30 text-emerald-400 bg-emerald-500/5'
                : 'border-red-500/30 text-red-400 bg-red-500/5'
            }`}
          >
            <span
              className={`w-1.5 h-1.5 rounded-full ${
                isConnected ? 'bg-emerald-400' : 'bg-red-400'
              }`}
            />
            {isConnected ? 'Live' : 'Offline'}
          </span>
        </div>
      </header>

      <main className="flex-1 px-6 py-6 max-w-6xl mx-auto w-full">
        <h1 className="text-2xl font-semibold mb-1">Fleet overview</h1>
        <p className="text-sm text-slate-500 mb-6">
          Aggregated stats across {agents.length} {agents.length === 1 ? 'agent' : 'agents'}, polled every 5 s.
        </p>

        <div className="grid grid-cols-2 md:grid-cols-4 gap-2 mb-8">
          <Big icon={<CpuIcon className="w-4 h-4" />} label="CPUs" value={totals.cpu.toString()} />
          <Big
            icon={<MemoryStickIcon className="w-4 h-4" />}
            label="Memory used"
            value={`${formatBytes(totals.memUsed)} / ${formatBytes(totals.memTotal)}`}
            pct={totals.memTotal > 0 ? (totals.memUsed / totals.memTotal) * 100 : 0}
          />
          <Big
            icon={<HardDriveIcon className="w-4 h-4" />}
            label="Disk /"
            value={`${formatBytes(totals.diskUsed)} / ${formatBytes(totals.diskTotal)}`}
            pct={totals.diskTotal > 0 ? (totals.diskUsed / totals.diskTotal) * 100 : 0}
          />
          <Big
            icon={<BoxIcon className="w-4 h-4" />}
            label="Containers running"
            value={`${totals.containersRunning} / ${totals.containers}`}
          />
        </div>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-2 mb-6">
          <SmallStat label="Services tracked" value={totals.svcTotal.toString()} />
          <SmallStat
            label="Failed services"
            value={totals.svcFailed.toString()}
            tone={totals.svcFailed > 0 ? 'red' : 'neutral'}
          />
          <SmallStat label="Agents online" value={agents.length.toString()} />
        </div>

        <h2 className="text-sm uppercase tracking-wide text-slate-500 mb-2">Hosts</h2>
        {agents.length === 0 ? (
          <div className="border border-dashed border-slate-800 rounded-md p-8 text-center text-slate-500 text-sm">
            No agents connected.
          </div>
        ) : (
          <ul className="space-y-2">
            {agents.map((agentId) => {
              const snap = snapshots[agentId];
              return (
                <li
                  key={agentId}
                  className="bg-slate-900 border border-slate-800 rounded-md px-4 py-3"
                >
                  <HostRow snapshot={snap ?? { agentId, hostname: agentId.replace(/-id$/, '') }} />
                </li>
              );
            })}
          </ul>
        )}

        <div className="mt-6 flex justify-end">
          <button
            type="button"
            onClick={() => {
              for (const agentId of agents) {
                sendToAgent(agentId, { type: 'SystemStatsRequest' });
                sendToAgent(agentId, { type: 'ListServicesRequest' });
                sendToAgent(agentId, { type: 'DockerListRequest' });
              }
            }}
            className="inline-flex items-center gap-1.5 text-xs font-medium py-1.5 px-3 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
          >
            <RefreshCwIcon className="w-3.5 h-3.5" />
            Refresh now
          </button>
        </div>
      </main>
    </div>
  );
}

function Big({
  icon,
  label,
  value,
  pct,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  pct?: number;
}) {
  const tone =
    pct === undefined ? 'bg-slate-700' : pct >= 90 ? 'bg-red-500' : pct >= 75 ? 'bg-amber-500' : 'bg-emerald-500';
  return (
    <div className="bg-slate-900 border border-slate-800 rounded-md p-4">
      <div className="flex items-center gap-1.5 text-[11px] uppercase tracking-wide text-slate-500">
        <span className="text-slate-400">{icon}</span>
        {label}
      </div>
      <div className="text-xl font-semibold mt-1 truncate" title={value}>
        {value}
      </div>
      {pct !== undefined && (
        <div className="mt-2 h-1 rounded-full bg-slate-800 overflow-hidden">
          <div className={`h-full ${tone}`} style={{ width: `${Math.min(100, Math.max(0, pct))}%` }} />
        </div>
      )}
    </div>
  );
}

function SmallStat({
  label,
  value,
  tone = 'neutral',
}: {
  label: string;
  value: string;
  tone?: 'neutral' | 'red';
}) {
  const valueColor = tone === 'red' ? 'text-red-300' : 'text-slate-100';
  return (
    <div className="bg-slate-900 border border-slate-800 rounded-md px-3 py-2">
      <div className="text-[11px] uppercase tracking-wide text-slate-500">{label}</div>
      <div className={`text-lg font-semibold ${valueColor}`}>{value}</div>
    </div>
  );
}

function HostRow({ snapshot }: { snapshot: AgentSnapshot }) {
  const stats = snapshot.stats;
  const services = snapshot.services;
  const docker = snapshot.docker;
  const failed = services?.filter((s) => s.active_state === 'failed').length ?? 0;
  const memPct =
    stats && stats.mem_total_kb > 0
      ? ((stats.mem_total_kb - stats.mem_available_kb) / stats.mem_total_kb) * 100
      : 0;
  const diskPct =
    stats && stats.root_disk_total_kb > 0
      ? (stats.root_disk_used_kb / stats.root_disk_total_kb) * 100
      : 0;
  const runningContainers = docker?.containers.filter((c) => c.state === 'running').length ?? 0;
  const swarmRoleLabel = docker?.swarm_role && docker.swarm_role !== 'notinswarm'
    ? docker.swarm_role
    : null;

  return (
    <div className="flex flex-wrap items-center gap-x-6 gap-y-2 text-sm">
      <div className="flex items-center gap-2 min-w-[10rem]">
        <ServerIcon className="w-4 h-4 text-slate-500 shrink-0" />
        <span className="font-medium text-slate-100 truncate">{snapshot.hostname}</span>
        {swarmRoleLabel && (
          <span className="text-[10px] uppercase tracking-wide text-blue-300 bg-blue-500/10 px-1.5 py-0.5 rounded">
            {swarmRoleLabel}
          </span>
        )}
      </div>

      <Field
        label="Load"
        value={stats ? `${stats.load_1.toFixed(2)} (${stats.cpu_count}c)` : '—'}
      />
      <Field
        label="Mem"
        value={stats ? `${memPct.toFixed(0)}%` : '—'}
        tone={memPct >= 90 ? 'red' : memPct >= 75 ? 'amber' : undefined}
      />
      <Field
        label="Disk"
        value={stats ? `${diskPct.toFixed(0)}%` : '—'}
        tone={diskPct >= 90 ? 'red' : diskPct >= 75 ? 'amber' : undefined}
      />
      <Field
        label="Uptime"
        value={stats ? formatUptime(stats.uptime_secs) : '—'}
      />
      <Field
        label="Services"
        value={services ? `${services.length}` : '—'}
      />
      {failed > 0 && (
        <span className="inline-flex items-center gap-1 text-xs text-red-300 bg-red-500/10 border border-red-500/30 rounded-full px-2 py-0.5">
          <AlertTriangleIcon className="w-3 h-3" />
          {failed} failed
        </span>
      )}
      <Field
        label="Containers"
        value={
          docker?.available
            ? `${runningContainers} / ${docker.containers.length}`
            : docker
              ? '—'
              : 'loading'
        }
      />
    </div>
  );
}

function Field({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: 'red' | 'amber';
}) {
  const valueColor =
    tone === 'red'
      ? 'text-red-300'
      : tone === 'amber'
        ? 'text-amber-300'
        : 'text-slate-200';
  return (
    <div className="flex flex-col">
      <span className="text-[10px] uppercase tracking-wide text-slate-500">{label}</span>
      <span className={`text-sm font-medium ${valueColor}`}>{value}</span>
    </div>
  );
}
