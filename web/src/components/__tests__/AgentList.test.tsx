import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import AgentList from '../AgentList';

vi.mock('../providers/WebSocketProvider', () => ({
  useWebSocket: () => ({ agents: ['swarm-master-id'] }),
}));

vi.mock('../providers/CoreFleetProvider', () => ({
  useCoreFleet: () => ({
    snapshots: {
      'swarm-master-id': {
        agentId: 'swarm-master-id',
        hostname: 'swarm-master',
        status: 'online',
        lastSeenAt: 200,
        services: [
          {
            name: 'healthy.service',
            description: 'Healthy service',
            load_state: 'loaded',
            active_state: 'active',
            sub_state: 'running',
          },
          {
            name: 'failed.service',
            description: 'Failed service',
            load_state: 'loaded',
            active_state: 'failed',
            sub_state: 'failed',
          },
        ],
        docker: {
          available: true,
          swarm_role: 'manager',
          containers: [],
          error: null,
        },
      },
    },
  }),
}));

vi.mock('../providers/FleetSnapshotsProvider', () => ({
  useFleetSnapshots: () => ({ snapshots: {} }),
}));

describe('AgentList', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('renders service health and Swarm role from durable fleet snapshots', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      ),
    );

    render(<AgentList selectedAgent={null} onSelectAgent={vi.fn()} />);

    expect(screen.getByText('swarm-master')).toBeInTheDocument();
    expect(screen.getByText('MGR')).toBeInTheDocument();
    expect(screen.getByText('⚠1')).toBeInTheDocument();
  });
});
