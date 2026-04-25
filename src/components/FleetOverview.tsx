'use client';

import { useMemo, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useFleetSnapshots, AgentSnapshot } from './providers/FleetSnapshotsProvider';
import {
  ServerIcon,
  CpuIcon,
  MemoryStickIcon,
  HardDriveIcon,
  AlertTriangleIcon,
  BoxIcon,
  RefreshCwIcon,
  SearchIcon,
  XIcon,
} from 'lucide-react';

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

export default function FleetOverview({
  onSelectAgent,
}: {
  onSelectAgent?: (agentId: string) => void;
}) {
  const { agents } = useWebSocket();
  const { snapshots, refresh } = useFleetSnapshots();
  const [search, setSearch] = useState('');

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

  // Cross-host search: match against systemd unit names + descriptions and
  // container names + images. Returns up to 50 hits per host so the page
  // doesn't blow up on broad queries.
  const searchHits = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return null;
    const out: Array<{
      agentId: string;
      hostname: string;
      kind: 'service' | 'container';
      name: string;
      detail: string;
      state: string;
    }> = [];
    for (const s of Object.values(snapshots)) {
      if (s.services) {
        for (const svc of s.services) {
          if (
            svc.name.toLowerCase().includes(q) ||
            svc.description.toLowerCase().includes(q)
          ) {
            out.push({
              agentId: s.agentId,
              hostname: s.hostname,
              kind: 'service',
              name: svc.name,
              detail: svc.description,
              state: svc.active_state,
            });
            if (out.length >= 200) break;
          }
        }
      }
      if (s.docker?.available) {
        for (const c of s.docker.containers) {
          if (
            (c.names && c.names.toLowerCase().includes(q)) ||
            (c.image && c.image.toLowerCase().includes(q))
          ) {
            out.push({
              agentId: s.agentId,
              hostname: s.hostname,
              kind: 'container',
              name: c.names || c.id.slice(0, 12),
              detail: c.image,
              state: c.state,
            });
            if (out.length >= 200) break;
          }
        }
      }
      if (out.length >= 200) break;
    }
    return out;
  }, [snapshots, search]);

  return (
    <div className="px-6 py-6 max-w-6xl mx-auto w-full">
      <div className="flex items-baseline justify-between mb-1">
        <h1 className="text-2xl font-semibold">Fleet overview</h1>
        <button
          type="button"
          onClick={refresh}
          className="inline-flex items-center gap-1.5 text-xs font-medium py-1 px-2.5 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 transition-colors"
        >
          <RefreshCwIcon className="w-3.5 h-3.5" />
          Refresh
        </button>
      </div>
      <p className="text-sm text-slate-500 mb-4">
        Aggregated stats across {agents.length} {agents.length === 1 ? 'agent' : 'agents'}, polled every 5 s.
      </p>

      <div className="relative mb-6 max-w-xl">
        <SearchIcon className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-500" />
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search services + containers across the fleet…"
          className="w-full pl-8 pr-7 py-1.5 text-sm bg-slate-900 border border-slate-700 rounded-md focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500 text-slate-100 placeholder:text-slate-500"
        />
        {search && (
          <button
            type="button"
            onClick={() => setSearch('')}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-200"
            aria-label="Clear"
          >
            <XIcon className="w-4 h-4" />
          </button>
        )}
      </div>

      {searchHits ? (
        <SearchResults hits={searchHits} onSelectAgent={onSelectAgent} />
      ) : (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-2 mb-6">
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
                  <li key={agentId}>
                    <button
                      type="button"
                      onClick={() => onSelectAgent?.(agentId)}
                      className="w-full text-left bg-slate-900 border border-slate-800 hover:border-slate-700 rounded-md px-4 py-3 transition-colors"
                    >
                      <HostRow snapshot={snap ?? { agentId, hostname: agentId.replace(/-id$/, '') }} />
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </>
      )}
    </div>
  );
}

function SearchResults({
  hits,
  onSelectAgent,
}: {
  hits: Array<{
    agentId: string;
    hostname: string;
    kind: 'service' | 'container';
    name: string;
    detail: string;
    state: string;
  }>;
  onSelectAgent?: (agentId: string) => void;
}) {
  if (hits.length === 0) {
    return (
      <div className="border border-dashed border-slate-800 rounded-md p-8 text-center text-slate-500 text-sm">
        No matches.
      </div>
    );
  }
  return (
    <div>
      <div className="text-xs text-slate-500 mb-2">
        {hits.length} match{hits.length === 1 ? '' : 'es'} across the fleet
      </div>
      <ul className="space-y-1.5">
        {hits.map((h, i) => (
          <li key={`${h.agentId}-${h.kind}-${h.name}-${i}`}>
            <button
              type="button"
              onClick={() => onSelectAgent?.(h.agentId)}
              className="w-full text-left bg-slate-900 border border-slate-800 hover:border-slate-700 rounded-md px-3 py-2 transition-colors flex items-center gap-3"
            >
              <span className="text-[10px] uppercase tracking-wide text-slate-500 w-16 shrink-0">
                {h.kind}
              </span>
              <ServerIcon className="w-3.5 h-3.5 text-slate-500 shrink-0" />
              <span className="text-xs text-slate-400 w-32 truncate" title={h.hostname}>
                {h.hostname}
              </span>
              <span className="text-sm text-slate-100 truncate flex-1" title={h.name}>
                {h.name}
              </span>
              <span className="text-xs text-slate-500 truncate max-w-xs hidden md:inline" title={h.detail}>
                {h.detail}
              </span>
              <span
                className={`text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded shrink-0 ${
                  h.state === 'active' || h.state === 'running'
                    ? 'bg-emerald-500/20 text-emerald-300'
                    : h.state === 'failed' || h.state === 'dead'
                      ? 'bg-red-500/20 text-red-300'
                      : 'bg-slate-800 text-slate-400'
                }`}
              >
                {h.state || '—'}
              </span>
            </button>
          </li>
        ))}
      </ul>
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
      <Field label="Services" value={services ? `${services.length}` : '—'} />
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
