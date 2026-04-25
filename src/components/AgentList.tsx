'use client';

import { useWebSocket } from './providers/WebSocketProvider';
import { useFleetSnapshots } from './providers/FleetSnapshotsProvider';
import { ServerIcon, AlertTriangleIcon } from 'lucide-react';

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
      <div className="px-4 py-6 text-xs text-slate-500 leading-relaxed">
        No agents connected. Use{' '}
        <span className="text-slate-300">Connect agent</span> above to pair a new host.
      </div>
    );
  }

  return (
    <ul className="py-2">
      {agents.map((agent) => {
        const label = agent.replace(/-id$/, '');
        const isSelected = selectedAgent === agent;
        const snap = snapshots[agent];
        const failed =
          snap?.services?.filter((s) => s.active_state === 'failed').length ?? 0;
        const swarmRole = snap?.docker?.swarm_role;
        return (
          <li key={agent}>
            <button
              className={`w-full flex items-center gap-3 px-4 py-2.5 text-left transition-colors ${
                isSelected
                  ? 'bg-blue-600 text-white'
                  : 'text-slate-300 hover:bg-slate-800'
              }`}
              onClick={() => onSelectAgent(agent)}
              title={swarmRole && swarmRole !== 'notinswarm' ? `swarm role: ${swarmRole}` : undefined}
            >
              <ServerIcon
                className={`w-4 h-4 shrink-0 ${
                  isSelected ? 'text-white' : 'text-slate-500'
                }`}
              />
              <span className="truncate text-sm flex-1 min-w-0">{label}</span>
              {failed > 0 && (
                <span
                  className={`inline-flex items-center gap-0.5 text-[10px] font-medium px-1.5 py-0.5 rounded ${
                    isSelected
                      ? 'bg-red-500/30 text-red-100'
                      : 'bg-red-500/15 text-red-300 border border-red-500/30'
                  }`}
                  title={`${failed} failed unit${failed === 1 ? '' : 's'}`}
                >
                  <AlertTriangleIcon className="w-2.5 h-2.5" />
                  {failed}
                </span>
              )}
              {swarmRole && swarmRole !== 'notinswarm' && (
                <span
                  className={`text-[9px] uppercase tracking-wide px-1 py-0.5 rounded ${
                    isSelected
                      ? 'bg-white/20 text-white'
                      : 'bg-blue-500/10 text-blue-300'
                  }`}
                >
                  {swarmRole === 'manager' ? 'mgr' : 'wrk'}
                </span>
              )}
              <span
                className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                  isSelected ? 'bg-white/80' : 'bg-emerald-400'
                }`}
                title="Online"
              />
            </button>
          </li>
        );
      })}
    </ul>
  );
}
