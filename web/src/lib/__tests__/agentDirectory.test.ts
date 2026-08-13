import { describe, expect, it } from 'vitest';
import { effectiveAgentDirectory } from '../agentDirectory';
import type { FleetHost } from '../coreFleet';

function host(
  agentId: string,
  status: FleetHost['status'],
  capabilities: string[],
): FleetHost {
  return {
    agent_id: agentId,
    hostname: agentId.replace(/-id$/, ''),
    status,
    protocol_version: 19,
    capabilities,
    metadata: {},
    first_seen_at: 1,
    last_seen_at: 2,
    disconnected_at: status === 'offline' ? 2 : null,
    system: null,
    services: null,
    docker: null,
    swarm: null,
  };
}

describe('effectiveAgentDirectory', () => {
  it('retains durable online agents and capabilities when the UI socket is unavailable', () => {
    const result = effectiveAgentDirectory(
      [host('swarm-worker-1-id', 'online', ['systemd', 'docker', 'swarm'])],
      [],
      {},
    );

    expect(result.agents).toEqual(['swarm-worker-1-id']);
    expect(result.capabilities['swarm-worker-1-id']).toEqual(['systemd', 'docker', 'swarm']);
  });

  it('uses the durable capability set and supplements it with socket-only agents', () => {
    const result = effectiveAgentDirectory(
      [
        host('swarm-master-id', 'online', ['systemd', 'docker', 'swarm']),
        host('retired-id', 'offline', ['docker']),
      ],
      ['swarm-master-id', 'new-agent-id'],
      {
        'swarm-master-id': ['systemd'],
        'new-agent-id': ['systemd'],
      },
    );

    expect(result.agents).toEqual(['swarm-master-id', 'new-agent-id']);
    expect(result.capabilities).toEqual({
      'swarm-master-id': ['systemd', 'docker', 'swarm'],
      'new-agent-id': ['systemd'],
    });
  });
});
