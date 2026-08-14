import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AgentList from '../AgentList';

const providerState = vi.hoisted(() => ({
  agents: [] as string[],
  hosts: [] as Array<Record<string, unknown>>,
  snapshots: {} as Record<string, unknown>,
}));

vi.mock('../providers/WebSocketProvider', () => ({
  useWebSocket: () => ({ agents: providerState.agents }),
}));

vi.mock('../providers/CoreFleetProvider', () => ({
  useCoreFleet: () => ({
    hosts: providerState.hosts,
    snapshots: providerState.snapshots,
  }),
}));

describe('AgentList', () => {
  beforeEach(() => {
    providerState.agents = ['swarm-master-id'];
    providerState.hosts = [];
    providerState.snapshots = {
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
    };
  });

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

  it('uses the durable host identity and status for offline rows without guessing from tokens', () => {
    providerState.agents = [];
    providerState.snapshots = {};
    providerState.hosts = [
      {
        agent_id: 'custom-node-7',
        hostname: 'retired-node',
        status: 'offline',
        protocol_version: 19,
        capabilities: ['systemd'],
        metadata: {},
        first_seen_at: 100,
        last_seen_at: Math.floor(Date.now() / 1000) - 120,
        disconnected_at: 102,
        system: null,
        services: null,
        docker: null,
        swarm: null,
      },
    ];
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);

    render(<AgentList selectedAgent={null} onSelectAgent={vi.fn()} />);

    expect(screen.getByText('retired-node')).toBeInTheDocument();
    expect(screen.getByText('2m ago')).toBeInTheDocument();
    expect(screen.getByRole('button')).toBeDisabled();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
