'use client';

import { useEffect, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useFleetSnapshots } from './providers/FleetSnapshotsProvider';
import { apiFetch } from '@/lib/api';

interface TokenInfo {
  hostname: string | null;
  last_seen: number;
}

export default function AgentList({
  selectedAgent,
  onSelectAgent,
}: {
  selectedAgent: string | null;
  onSelectAgent: (agentId: string) => void;
}) {
  const { agents } = useWebSocket();
  const { snapshots } = useFleetSnapshots();
  const [knownHosts, setKnownHosts] = useState<TokenInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    apiFetch('/api/tokens')
      .then((r) => r.json())
      .then((rows: TokenInfo[]) => {
        if (!cancelled) setKnownHosts(rows);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [agents.length]);

  const onlineSet = new Set(agents);
  const offlineHosts = knownHosts
    .filter((t) => t.hostname && !onlineSet.has(`${t.hostname}-id`))
    .map((t) => ({
      id: `${t.hostname}-id`,
      label: t.hostname!,
      lastSeen: t.last_seen,
    }));

  if (agents.length === 0 && offlineHosts.length === 0) {
    return (
      <div style={{ padding: '12px 14px', color: 'var(--fg-3)', fontSize: 11, lineHeight: 1.6 }}>
        No agents connected. Use{' '}
        <span style={{ color: 'var(--fg-1)' }}>Connect agent</span> above to pair a new host.
      </div>
    );
  }

  return (
    <>
      {agents.map((agent) => {
        const label = agent.replace(/-id$/, '');
        const snap = snapshots[agent];
        const failed =
          snap?.services?.filter((s) => s.active_state === 'failed').length ?? 0;
        const swarmRole = snap?.docker?.swarm_role;
        const isActive = selectedAgent === agent;
        const dotCls = failed > 0 ? 'warn' : '';
        const roleChipCls =
          swarmRole === 'manager'
            ? 'chip role-mgr'
            : swarmRole === 'worker'
              ? 'chip role-wrk'
              : 'chip';
        const roleLabel =
          swarmRole === 'manager' ? 'MGR' : swarmRole === 'worker' ? 'WRK' : '';

        return (
          <button
            key={agent}
            type="button"
            className={`agent-row ${isActive ? 'active' : ''}`}
            onClick={() => onSelectAgent(agent)}
            title={swarmRole && swarmRole !== 'notinswarm' ? `swarm role: ${swarmRole}` : undefined}
          >
            <span className={`dot ${dotCls}`} />
            <span className="name">{label}</span>
            <span className="chips">
              {roleLabel && <span className={roleChipCls}>{roleLabel}</span>}
              {failed > 0 && <span className="chip failed">&#x26A0;{failed}</span>}
            </span>
          </button>
        );
      })}
      {offlineHosts.map((h) => {
        const ago = formatAgo(h.lastSeen);
        return (
          <button
            key={h.id}
            type="button"
            className="agent-row offline"
            title={`Last seen ${ago}`}
            disabled
          >
            <span className="dot off" />
            <span className="name" style={{ opacity: 0.45 }}>{h.label}</span>
            <span className="chips">
              <span className="chip" style={{ opacity: 0.45 }}>{ago}</span>
            </span>
          </button>
        );
      })}
    </>
  );
}

function formatAgo(epochSecs: number): string {
  const diff = Math.floor(Date.now() / 1000) - epochSecs;
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}
