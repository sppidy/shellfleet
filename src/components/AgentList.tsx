'use client';

import { useWebSocket } from './providers/WebSocketProvider';
import { ServerIcon } from 'lucide-react';

export default function AgentList({
  selectedAgent,
  onSelectAgent
}: {
  selectedAgent: string | null;
  onSelectAgent: (agentId: string) => void;
}) {
  const { agents } = useWebSocket();

  if (agents.length === 0) {
    return (
      <div className="p-4 text-sm text-slate-500">
        No agents connected.
      </div>
    );
  }

  return (
    <ul className="py-2">
      {agents.map((agent) => (
        <li key={agent}>
          <button
            className={`w-full flex items-center px-4 py-3 text-left transition-colors ${
              selectedAgent === agent 
                ? 'bg-blue-600 text-white' 
                : 'hover:bg-slate-800 text-slate-300'
            }`}
            onClick={() => onSelectAgent(agent)}
          >
            <ServerIcon className="w-5 h-5 mr-3 opacity-75" />
            <span className="truncate">{agent.replace('-id', '')}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}
