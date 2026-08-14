'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useCoreFleet } from './providers/CoreFleetProvider';
import { SystemStatsPayload } from '@/lib/types';

const STATS_INTERVAL_MS = 5_000;
const STATS_TIMEOUT_MS = 10_000;

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
  const { sendToAgent, onAgentMessage, isConnected, liveAgents } = useWebSocket();
  const { snapshots } = useCoreFleet();
  const [stats, setStats] = useState<SystemStatsPayload | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const durableStats = snapshots[agentId]?.stats ?? null;
  const isAgentLive = isConnected && liveAgents.includes(agentId);
  const displayedStats = stats ?? durableStats;

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

    if (!isAgentLive) return unsubscribe;

    const request = () => sendToAgent(agentId, { type: 'SystemStatsRequest' });
    request();
    timeoutRef.current = setTimeout(() => setUnsupported(true), STATS_TIMEOUT_MS);
    const interval = setInterval(request, STATS_INTERVAL_MS);

    return () => {
      unsubscribe();
      clearInterval(interval);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [agentId, isAgentLive, sendToAgent, onAgentMessage]);

  if (unsupported && !displayedStats) {
    return (
      <div
        style={{
          padding: 10,
          background: 'var(--warn-bg)',
          border: '1px solid var(--warn-bd)',
          borderRadius: 'var(--r)',
          color: 'var(--warn)',
          fontFamily: 'var(--mono)',
          fontSize: 11,
        }}
      >
        ⚠ This live agent doesn&apos;t expose system stats. Upgrade with{' '}
        <code style={{ background: 'rgba(0,0,0,0.2)', padding: '0 4px', borderRadius: 2 }}>
          apt install --only-upgrade shellfleet-agent
        </code>
        .
      </div>
    );
  }

  if (!displayedStats) {
    return (
      <div className="system-stats">
        {!isAgentLive && (
          <div className="live-data-note" role="status">
            Live stats are reconnecting. No durable system snapshot is available yet.
          </div>
        )}
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
      {(!isAgentLive || (unsupported && !stats)) && (
        <div className="live-data-note" role="status">
          {!isAgentLive
            ? 'Showing the latest durable system snapshot while live stats reconnect.'
            : 'Live refresh timed out. Showing the latest durable system snapshot.'}
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
