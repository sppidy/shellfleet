import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import FleetOverview from '../FleetOverview';

const refresh = vi.fn();
const durableFleet = {
  hosts: [
    {
      agent_id: 'swarm-master-id',
      hostname: 'swarm-master',
      status: 'online' as const,
      protocol_version: 19,
      capabilities: ['systemd', 'docker', 'swarm'],
      metadata: {},
      first_seen_at: 100,
      last_seen_at: 200,
      disconnected_at: null,
      system: null,
      services: null,
      docker: null,
      swarm: null,
    },
    {
      agent_id: 'swarm-worker-1-id',
      hostname: 'swarm-worker-1',
      status: 'offline' as const,
      protocol_version: 19,
      capabilities: ['systemd', 'docker', 'swarm'],
      metadata: {},
      first_seen_at: 100,
      last_seen_at: 150,
      disconnected_at: 151,
      system: null,
      services: null,
      docker: null,
      swarm: null,
    },
  ],
  snapshots: {
    'swarm-master-id': {
      agentId: 'swarm-master-id',
      hostname: 'swarm-master',
      status: 'online' as const,
      lastSeenAt: 200,
      stats: {
        hostname: 'swarm-master',
        kernel: '6.12',
        uptime_secs: 3600,
        cpu_count: 4,
        load_1: 0.5,
        load_5: 0.4,
        load_15: 0.3,
        mem_total_kb: 1000,
        mem_available_kb: 400,
        swap_total_kb: 0,
        swap_free_kb: 0,
        root_disk_total_kb: 2000,
        root_disk_used_kb: 800,
      },
      services: [],
      docker: { available: true, swarm_role: 'manager' as const, containers: [], error: null },
      swarm: { available: true, is_manager: true, services: [], nodes: [], error: null },
    },
    'swarm-worker-1-id': {
      agentId: 'swarm-worker-1-id',
      hostname: 'swarm-worker-1',
      status: 'offline' as const,
      lastSeenAt: 150,
      stats: {
        hostname: 'swarm-worker-1',
        kernel: '6.12',
        uptime_secs: 7200,
        cpu_count: 2,
        load_1: 0.2,
        load_5: 0.2,
        load_15: 0.1,
        mem_total_kb: 1000,
        mem_available_kb: 700,
        swap_total_kb: 0,
        swap_free_kb: 0,
        root_disk_total_kb: 2000,
        root_disk_used_kb: 600,
      },
      services: [],
      docker: { available: true, swarm_role: 'worker' as const, containers: [], error: null },
      swarm: { available: true, is_manager: false, services: [], nodes: [], error: null },
    },
  },
  liveStatus: 'degraded' as const,
  loading: false,
  error: null,
  refresh,
};

vi.mock('../providers/CoreFleetProvider', () => ({
  useCoreFleet: () => durableFleet,
}));

vi.mock('../providers/WebSocketProvider', () => ({
  useWebSocket: () => ({ agents: [] }),
}));

describe('FleetOverview', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      ),
    );
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it('renders durable online and offline hosts while live updates are degraded', async () => {
    render(<FleetOverview />);

    expect(screen.getByText('2 agents · 1 online')).toBeInTheDocument();
    expect(screen.getByText('swarm-master')).toBeInTheDocument();
    expect(screen.getByText('swarm-worker-1')).toBeInTheDocument();
    expect(screen.getByText('offline')).toBeInTheDocument();
    expect(screen.getAllByText('DOCKER')).toHaveLength(2);
    expect(screen.getAllByText('SWARM')).toHaveLength(2);
    expect(screen.getByText(/live updates disconnected/i)).toBeInTheDocument();
  });
});
