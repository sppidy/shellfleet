'use client';

import { useWebSocket } from './providers/WebSocketProvider';
import { useFleetSnapshots } from './providers/FleetSnapshotsProvider';

export default function AgentList({
  selectedAgent,
  onSelectAgent,
}: {
  selectedAgent: string | null;
  onSelectAgent: (agentId: string) => void;
}) {
  const { agents } = useWebSocket();
  const { snapshots } = useFleetSnapshots();

  if (agents.length === 0) {
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
              {failed > 0 && <span className="chip failed">⚠{failed}</span>}
            </span>
          </button>
        );
      })}
    </>
  );
}
