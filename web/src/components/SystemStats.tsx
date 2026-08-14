'use client';

import { useCoreFleet } from './providers/CoreFleetProvider';

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
  if (secs <= 0) return '—';
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3_600);
  const m = Math.floor((secs % 3_600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

function bar(pct: number, opts: { hideOver100?: boolean } = {}) {
  const cls = pct >= 90 ? 'err' : pct >= 75 ? 'warn' : '';
  return (
    <div className="bar">
      <i
        className={cls}
        style={{
          width: `${opts.hideOver100 ? Math.min(100, Math.max(0, pct)) : Math.max(0, Math.min(100, pct))}%`,
        }}
      />
    </div>
  );
}

export default function SystemStats({ agentId }: { agentId: string }) {
  const { snapshots, liveStatus } = useCoreFleet();
  const snapshot = snapshots[agentId];
  const displayedStats = snapshot?.stats ?? null;
  const isAgentOnline = snapshot?.status === 'online';
  const dataIsLive = isAgentOnline && liveStatus === 'live';

  if (!displayedStats) {
    return (
      <div className="system-stats">
        <div className="live-data-note" role="status">
          {isAgentOnline
            ? liveStatus === 'live'
              ? 'Waiting for the next live system update.'
              : 'Live system updates are reconnecting. No durable snapshot is available yet.'
            : 'This agent is offline. No durable system snapshot is available yet.'}
        </div>
        <div className="system-stats-grid">
          {[0, 1, 2, 3].map((i) => (
            <div
              key={i}
              className="panel"
              style={{ height: 88, opacity: 0.5 }}
            />
          ))}
        </div>
      </div>
    );
  }

  const memUsed = displayedStats.mem_total_kb - displayedStats.mem_available_kb;
  const memPct = displayedStats.mem_total_kb > 0 ? (memUsed / displayedStats.mem_total_kb) * 100 : 0;
  const diskPct =
    displayedStats.root_disk_total_kb > 0
      ? (displayedStats.root_disk_used_kb / displayedStats.root_disk_total_kb) * 100
      : 0;
  const loadPct = displayedStats.cpu_count > 0
    ? (displayedStats.load_1 / displayedStats.cpu_count) * 100
    : 0;
  const swapUsed = displayedStats.swap_total_kb - displayedStats.swap_free_kb;

  return (
    <div className="system-stats">
      {!dataIsLive && (
        <div className="live-data-note" role="status">
          {isAgentOnline
            ? 'Live system updates are reconnecting. Showing the latest durable snapshot.'
            : 'This agent is offline. Showing its last durable system snapshot.'}
        </div>
      )}
      <div className="system-stats-grid">
        <div className="panel">
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">⌬</span> LOAD
            </div>
          </div>
          <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div className="stat-value">
              {displayedStats.load_1.toFixed(2)}
              <span className="unit"> / {displayedStats.cpu_count}</span>
            </div>
            {bar(loadPct, { hideOver100: true })}
            <div className="muted" style={{ fontSize: 10.5 }}>
              5m {displayedStats.load_5.toFixed(2)} · 15m {displayedStats.load_15.toFixed(2)}
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">▦</span> MEM
            </div>
          </div>
          <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div className="stat-value">
              {memPct.toFixed(0)}
              <span className="unit">%</span>
            </div>
            {bar(memPct)}
            <div className="muted" style={{ fontSize: 10.5 }}>
              {formatBytes(memUsed)} / {formatBytes(displayedStats.mem_total_kb)}
              {displayedStats.swap_total_kb > 0 && (
                <>
                  {' · swap '}
                  {formatBytes(swapUsed)} / {formatBytes(displayedStats.swap_total_kb)}
                </>
              )}
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">▰</span> DISK
            </div>
          </div>
          <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div className="stat-value">
              {diskPct.toFixed(0)}
              <span className="unit">%</span>
            </div>
            {bar(diskPct)}
            <div className="muted" style={{ fontSize: 10.5 }}>
              {formatBytes(displayedStats.root_disk_used_kb)} / {formatBytes(displayedStats.root_disk_total_kb)}
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">⏲</span> UPTIME
            </div>
          </div>
          <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div className="stat-value" style={{ fontSize: 18 }}>
              {formatUptime(displayedStats.uptime_secs)}
            </div>
            <div className="muted" style={{ fontSize: 10.5 }}>
              kernel {displayedStats.kernel}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
