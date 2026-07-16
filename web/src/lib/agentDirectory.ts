import type { FleetHost } from './coreFleet';

export type AgentDirectory = {
  agents: string[];
  capabilities: Record<string, string[]>;
};

/**
 * Build the operator-facing online-agent directory from the durable read model,
 * with the interactive socket as a fallback for hosts not collected yet.
 * Durable capabilities win because they survive UI socket reconnects and are
 * refreshed through the core SSE stream when an agent re-advertises.
 */
export function effectiveAgentDirectory(
  hosts: FleetHost[],
  socketAgents: string[],
  socketCapabilities: Record<string, string[]>,
): AgentDirectory {
  const durableHosts = hosts.filter((host) => host.status === 'online');
  const agents = [...new Set([...durableHosts.map((host) => host.agent_id), ...socketAgents])];
  const online = new Set(agents);
  const capabilities: Record<string, string[]> = {};

  for (const [agentId, values] of Object.entries(socketCapabilities)) {
    if (online.has(agentId)) capabilities[agentId] = [...values];
  }
  for (const host of durableHosts) {
    capabilities[host.agent_id] = [...host.capabilities];
  }

  return { agents, capabilities };
}
