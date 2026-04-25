'use client';

import { useWebSocket } from './providers/WebSocketProvider';
import { ServerIcon } from 'lucide-react';

export default function AgentList({
  selectedAgent,
  onSelectAgent,
}: {
  selectedAgent: string | null;
  onSelectAgent: (agentId: string) => void;
}) {
  const { agents } = useWebSocket();

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
        return (
          <li key={agent}>
            <button
              className={`w-full flex items-center gap-3 px-4 py-2.5 text-left transition-colors ${
                isSelected
                  ? 'bg-blue-600 text-white'
                  : 'text-slate-300 hover:bg-slate-800'
              }`}
              onClick={() => onSelectAgent(agent)}
            >
              <ServerIcon
                className={`w-4 h-4 shrink-0 ${
                  isSelected ? 'text-white' : 'text-slate-500'
                }`}
              />
              <span className="truncate text-sm">{label}</span>
              <span
                className={`ml-auto w-1.5 h-1.5 rounded-full ${
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
