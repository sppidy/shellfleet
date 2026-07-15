'use client';

import { useEffect, useMemo, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useFleetSnapshots, AgentSnapshot } from './providers/FleetSnapshotsProvider';
import type { HealthSnapshotRow } from '@/lib/types';

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

function bar(pct: number) {
  const cls = pct >= 90 ? 'err' : pct >= 75 ? 'warn' : '';
  return (
    <div className="bar">
      <i className={cls} style={{ width: `${Math.max(0, Math.min(100, pct))}%` }} />
    </div>
  );
}

function progPct(label: string, pct: number) {
  return (
    <div className="prog">
      <span style={{ width: 54, color: 'var(--fg-2)' }}>{label}</span>
      {bar(pct)}
      <span className="pct">{Math.round(pct)}%</span>
    </div>
  );
}

export default function FleetOverview({
  onSelectAgent,
}: {
  onSelectAgent?: (agentId: string) => void;
}) {
  const { agents } = useWebSocket();
  const { snapshots, refresh } = useFleetSnapshots();
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<'all' | 'mgr' | 'wrk' | 'warn'>('all');
  const [healthByAgent, setHealthByAgent] = useState<Record<string, HealthSnapshotRow>>({});

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const res = await fetch('/api/health-probes/snapshot', { credentials: 'include' });
        if (!res.ok) return;
        const rows: HealthSnapshotRow[] = await res.json();
        if (cancelled) return;
        const map: Record<string, HealthSnapshotRow> = {};
        for (const r of rows) map[r.agent_id] = r;
        setHealthByAgent(map);
      } catch {
        /* ignore */
      }
    };
    void load();
    const t = setInterval(load, 10_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

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
    let load1 = 0;
    let agentsWithStats = 0;
    for (const s of Object.values(snapshots)) {
      if (s.stats) {
        cpu += s.stats.cpu_count;
        memTotal += s.stats.mem_total_kb;
        memUsed += s.stats.mem_total_kb - s.stats.mem_available_kb;
        diskTotal += s.stats.root_disk_total_kb;
        diskUsed += s.stats.root_disk_used_kb;
        load1 += s.stats.load_1;
        agentsWithStats += 1;
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
      load1,
      agentsWithStats,
    };
  }, [snapshots]);

  const memPct = totals.memTotal > 0 ? (totals.memUsed / totals.memTotal) * 100 : 0;
  const diskPct = totals.diskTotal > 0 ? (totals.diskUsed / totals.diskTotal) * 100 : 0;

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

  const filteredAgents = useMemo(() => {
    return agents.filter((a) => {
      const snap = snapshots[a];
      const role = snap?.docker?.swarm_role;
      const failed = snap?.services?.filter((s) => s.active_state === 'failed').length ?? 0;
      if (filter === 'mgr') return role === 'manager';
      if (filter === 'wrk') return role === 'worker';
      if (filter === 'warn') return failed > 0;
      return true;
    });
  }, [agents, snapshots, filter]);

  return (
    <div className="pane">
      <div className="stat-grid">
        <div className="stat">
          <div className="stat-label">
            <span>CPUS / LOAD AVG</span>
            <span className="muted">{totals.cpu} cores</span>
          </div>
          <div className="stat-value">
            {totals.load1.toFixed(2)}
            <span className="unit"> / {totals.cpu}</span>
          </div>
          <div className="stat-sub">
            <span>{totals.agentsWithStats} reporting</span>
            <span>{agents.length} total</span>
          </div>
        </div>

        <div className="stat">
          <div className="stat-label">
            <span>MEMORY USED</span>
            <span className="muted">avg across fleet</span>
          </div>
          <div className="stat-value">
            {Math.round(memPct)}
            <span className="unit">%</span>
          </div>
          {bar(memPct)}
          <div className="stat-sub">
            <span>{formatBytes(totals.memUsed)}</span>
            <span>/ {formatBytes(totals.memTotal)}</span>
          </div>
        </div>

        <div className="stat">
          <div className="stat-label">
            <span>DISK USED</span>
            <span className="muted">root volume</span>
          </div>
          <div className="stat-value">
            {Math.round(diskPct)}
            <span className="unit">%</span>
          </div>
          {bar(diskPct)}
          <div className="stat-sub">
            <span>{formatBytes(totals.diskUsed)}</span>
            <span>/ {formatBytes(totals.diskTotal)}</span>
          </div>
        </div>

        <div className="stat">
          <div className="stat-label">
            <span>CONTAINERS</span>
            <span className="muted">running</span>
          </div>
          <div className="stat-value">
            {totals.containersRunning}
            <span className="unit"> / {totals.containers}</span>
          </div>
          <div
            className="row"
            style={{ gap: 16, fontSize: 11, color: 'var(--fg-2)', marginTop: 'auto' }}
          >
            <span>{totals.svcTotal} svcs</span>
            <span className={totals.svcFailed > 0 ? 'err-c' : 'muted'}>
              {totals.svcFailed} failed
            </span>
            <span className="ok">
              {agents.length}/{agents.length} agents
            </span>
          </div>
        </div>
      </div>

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▤</span> HOSTS
            <span className="meta">
              {agents.length} agents · {agents.length} online
            </span>
          </div>
          <div className="panel-actions">
            <div className="search-input" style={{ width: 280 }}>
              <span style={{ color: 'var(--accent)' }}>⌕</span>
              <input
                placeholder="filter hosts, services, containers…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <div className="seg">
              <button className={filter === 'all' ? 'on' : ''} onClick={() => setFilter('all')}>
                ALL
              </button>
              <button className={filter === 'mgr' ? 'on' : ''} onClick={() => setFilter('mgr')}>
                MGR
              </button>
              <button className={filter === 'wrk' ? 'on' : ''} onClick={() => setFilter('wrk')}>
                WRK
              </button>
              <button className={filter === 'warn' ? 'on' : ''} onClick={() => setFilter('warn')}>
                WARN
              </button>
            </div>
            <button className="btn" onClick={refresh} title="Refresh">
              ↻
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {searchHits ? (
            <SearchResults hits={searchHits} onSelectAgent={onSelectAgent} />
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ width: 110 }}>STATUS</th>
                  <th>HOST</th>
                  <th className="right" style={{ width: 80 }}>
                    LOAD
                  </th>
                  <th style={{ width: 170 }}>MEM</th>
                  <th style={{ width: 170 }}>DISK</th>
                  <th style={{ width: 110 }}>UPTIME</th>
                  <th style={{ width: 90 }}>FAILED</th>
                  <th style={{ width: 90 }}>PROBES</th>
                </tr>
              </thead>
              <tbody>
                {filteredAgents.length === 0 ? (
                  <tr>
                    <td colSpan={8} className="muted" style={{ padding: 32, textAlign: 'center' }}>
                      No hosts match the current filter.
                    </td>
                  </tr>
                ) : (
                  filteredAgents.map((agentId) => (
                    <HostRow
                      key={agentId}
                      snapshot={
                        snapshots[agentId] ?? {
                          agentId,
                          hostname: agentId.replace(/-id$/, ''),
                        }
                      }
                      health={healthByAgent[agentId]}
                      onClick={() => onSelectAgent?.(agentId)}
                    />
                  ))
                )}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
}

function HostRow({
  snapshot,
  health,
  onClick,
}: {
  snapshot: AgentSnapshot;
  health?: HealthSnapshotRow;
  onClick: () => void;
}) {
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
  const swarmRole = docker?.swarm_role && docker.swarm_role !== 'notinswarm' ? docker.swarm_role : null;
  const roleChip =
    swarmRole === 'manager' ? (
      <span className="chip role-mgr">MGR</span>
    ) : swarmRole === 'worker' ? (
      <span className="chip role-wrk">WRK</span>
    ) : null;

  const probeState = health
    ? health.red === 0 && health.unknown === 0
      ? 'ok'
      : 'err-c'
    : 'muted';
  const probeText = health
    ? health.total > 0
      ? `✓ ${health.green}/${health.total}`
      : '—'
    : '—';

  return (
    <tr onClick={onClick} style={{ cursor: 'pointer' }}>
      <td>
        <span className="status ok">
          <span className="dot" />
          online
        </span>
      </td>
      <td className="mono">
        {snapshot.hostname} {roleChip}
      </td>
      <td className="right mono">{stats ? stats.load_1.toFixed(2) : '—'}</td>
      <td>{stats ? progPct('mem', memPct) : <span className="muted">—</span>}</td>
      <td>{stats ? progPct('disk', diskPct) : <span className="muted">—</span>}</td>
      <td className="mono">{stats ? formatUptime(stats.uptime_secs) : '—'}</td>
      <td className={`${failed ? 'err-c' : 'muted'} mono`}>{failed ? `⚠ ${failed}` : '—'}</td>
      <td className={`${probeState} mono`}>{probeText}</td>
    </tr>
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
    return <div className="empty">No matches.</div>;
  }
  return (
    <table className="tbl">
      <thead>
        <tr>
          <th style={{ width: 80 }}>KIND</th>
          <th style={{ width: 160 }}>HOST</th>
          <th>NAME</th>
          <th>DETAIL</th>
          <th style={{ width: 90 }}>STATE</th>
        </tr>
      </thead>
      <tbody>
        {hits.map((h, i) => (
          <tr
            key={`${h.agentId}-${h.kind}-${h.name}-${i}`}
            style={{ cursor: 'pointer' }}
            onClick={() => onSelectAgent?.(h.agentId)}
          >
            <td className="mono muted">{h.kind}</td>
            <td className="mono">{h.hostname}</td>
            <td className="mono" style={{ color: 'var(--fg)' }}>
              {h.name}
            </td>
            <td className="mono muted">{h.detail}</td>
            <td
              className={`mono ${
                h.state === 'active' || h.state === 'running'
                  ? 'ok'
                  : h.state === 'failed' || h.state === 'dead'
                    ? 'err-c'
                    : 'muted'
              }`}
            >
              {h.state || '—'}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
