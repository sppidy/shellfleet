'use client';

import { createContext, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { useWebSocket } from './WebSocketProvider';
import {
  DockerListPayload,
  ServiceInfo,
  SystemStatsPayload,
} from '@/lib/types';

export type AgentSnapshot = {
  agentId: string;
  hostname: string;
  stats?: SystemStatsPayload;
  services?: ServiceInfo[];
  docker?: DockerListPayload;
};

interface FleetSnapshotsContextValue {
  snapshots: Record<string, AgentSnapshot>;
  refresh: () => void;
}

const Ctx = createContext<FleetSnapshotsContextValue | null>(null);

const POLL_MS = 5_000;

const seed = (agentId: string): AgentSnapshot => ({
  agentId,
  hostname: agentId.replace(/-id$/, ''),
});

export function FleetSnapshotsProvider({ children }: { children: React.ReactNode }) {
  const { agents, sendToAgent, onAgentMessage } = useWebSocket();
  const [snapshots, setSnapshots] = useState<Record<string, AgentSnapshot>>({});

  // Stable key so the effect rebinds only when the agent set actually changes.
  const agentsKey = useMemo(() => agents.slice().sort().join(','), [agents]);
  const agentsRef = useRef(agents);
  agentsRef.current = agents;

  useEffect(() => {
    if (agents.length === 0) {
      setSnapshots({});
      return;
    }

    setSnapshots((prev) => {
      const next: Record<string, AgentSnapshot> = {};
      for (const a of agents) next[a] = prev[a] ?? seed(a);
      return next;
    });

    const unsubs: Array<() => void> = [];
    for (const agentId of agents) {
      const unsub = onAgentMessage(agentId, (msg) => {
        if (msg.type === 'SystemStatsResponse') {
          setSnapshots((prev) => ({
            ...prev,
            [agentId]: { ...(prev[agentId] ?? seed(agentId)), stats: msg.payload },
          }));
        } else if (msg.type === 'ListServicesResponse') {
          setSnapshots((prev) => ({
            ...prev,
            [agentId]: {
              ...(prev[agentId] ?? seed(agentId)),
              services: msg.payload.services,
            },
          }));
        } else if (msg.type === 'DockerListResponse') {
          setSnapshots((prev) => ({
            ...prev,
            [agentId]: {
              ...(prev[agentId] ?? seed(agentId)),
              docker: msg.payload,
            },
          }));
        }
      });
      unsubs.push(unsub);
    }

    const poll = () => {
      for (const agentId of agentsRef.current) {
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentsKey, sendToAgent, onAgentMessage]);

  const value: FleetSnapshotsContextValue = useMemo(
    () => ({
      snapshots,
      refresh: () => {
        for (const agentId of agentsRef.current) {
          sendToAgent(agentId, { type: 'SystemStatsRequest' });
          sendToAgent(agentId, { type: 'ListServicesRequest' });
          sendToAgent(agentId, { type: 'DockerListRequest' });
        }
      },
    }),
    [snapshots, sendToAgent],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useFleetSnapshots() {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error('useFleetSnapshots must be used within FleetSnapshotsProvider');
  return ctx;
}
