import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ServiceList from '../ServiceList';
import SystemStats from '../SystemStats';

const socket = vi.hoisted(() => ({
  sendToAgent: vi.fn(),
  onAgentMessage: vi.fn(() => vi.fn()),
}));

vi.mock('../providers/WebSocketProvider', () => ({
  useWebSocket: () => ({
    isConnected: false,
    liveAgents: [],
    sendToAgent: socket.sendToAgent,
    onAgentMessage: socket.onAgentMessage,
  }),
}));

vi.mock('../providers/SessionProvider', () => ({
  useCanWrite: () => true,
}));

vi.mock('../providers/CoreFleetProvider', () => ({
  useCoreFleet: () => ({
    snapshots: {
      'node-a-id': {
        agentId: 'node-a-id',
        hostname: 'node-a',
        status: 'online',
        lastSeenAt: 1_000,
        stats: {
          hostname: 'node-a',
          kernel: '6.12.0',
          uptime_secs: 90_000,
          cpu_count: 4,
          load_1: 0.5,
          load_5: 0.4,
          load_15: 0.3,
          mem_total_kb: 8_000,
          mem_available_kb: 3_000,
          swap_total_kb: 0,
          swap_free_kb: 0,
          root_disk_total_kb: 20_000,
          root_disk_used_kb: 5_000,
        },
        services: [
          {
            name: 'sshd.service',
            description: 'OpenSSH server',
            load_state: 'loaded',
            active_state: 'active',
            sub_state: 'running',
          },
        ],
      },
    },
  }),
}));

describe('durable selected-host overview', () => {
  afterEach(() => {
    cleanup();
    socket.sendToAgent.mockClear();
    socket.onAgentMessage.mockClear();
  });

  it('renders durable system stats instead of an agent-upgrade warning when the live link is down', () => {
    render(<SystemStats agentId="node-a-id" />);

    expect(screen.getByText('0.50')).toBeInTheDocument();
    expect(screen.getByText(/latest durable system snapshot/i)).toBeInTheDocument();
    expect(screen.queryByText(/upgrade with/i)).not.toBeInTheDocument();
    expect(socket.sendToAgent).not.toHaveBeenCalled();
  });

  it('renders durable services and disables live controls while reconnecting', () => {
    render(<ServiceList agentId="node-a-id" />);

    expect(screen.getByText('sshd.service')).toBeInTheDocument();
    expect(screen.getByText(/latest durable service snapshot/i)).toBeInTheDocument();
    for (const control of screen.getAllByTitle('Live controls are reconnecting')) {
      expect(control).toBeDisabled();
    }
    expect(socket.sendToAgent).not.toHaveBeenCalled();
  });
});
