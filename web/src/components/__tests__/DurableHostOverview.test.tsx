import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ServiceList from '../ServiceList';
import SystemStats from '../SystemStats';

const socket = vi.hoisted(() => ({
  sendToAgent: vi.fn(),
  onAgentMessage: vi.fn(() => vi.fn()),
}));
const core = vi.hoisted(() => ({
  liveStatus: 'live' as 'live' | 'connecting' | 'degraded',
  loading: false,
  refresh: vi.fn(),
  serviceActiveState: 'active',
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
    liveStatus: core.liveStatus,
    loading: core.loading,
    refresh: core.refresh,
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
            active_state: core.serviceActiveState,
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
    core.liveStatus = 'live';
    core.loading = false;
    core.serviceActiveState = 'active';
    core.refresh.mockClear();
  });

  it('renders SSE-backed system stats as live when the interactive socket is down', () => {
    render(<SystemStats agentId="node-a-id" />);

    expect(screen.getByText('0.50')).toBeInTheDocument();
    expect(screen.queryByText(/durable.*snapshot/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/upgrade with/i)).not.toBeInTheDocument();
    expect(socket.sendToAgent).not.toHaveBeenCalled();
  });

  it('keeps service data live and refreshable while interactive controls reconnect', () => {
    render(<ServiceList agentId="node-a-id" />);

    expect(screen.getByText('sshd.service')).toBeInTheDocument();
    expect(screen.getByText(/service data is live.*controls are reconnecting/i)).toBeInTheDocument();
    for (const control of screen.getAllByTitle('Live controls are reconnecting')) {
      expect(control).toBeDisabled();
    }
    fireEvent.click(screen.getByTitle('Refresh service state'));
    expect(core.refresh).toHaveBeenCalledOnce();
    expect(socket.sendToAgent).not.toHaveBeenCalled();
  });

  it('labels durable state as stale only when the fleet event stream is degraded', () => {
    core.liveStatus = 'degraded';

    render(<SystemStats agentId="node-a-id" />);

    expect(screen.getByText(/live system updates are reconnecting/i)).toBeInTheDocument();
    expect(screen.getByText(/latest durable snapshot/i)).toBeInTheDocument();
  });

  it('renders newer durable service samples instead of masking them with component state', () => {
    const view = render(<ServiceList agentId="node-a-id" />);
    expect(within(screen.getByRole('row')).getByText('active')).toBeInTheDocument();

    core.serviceActiveState = 'failed';
    view.rerender(<ServiceList agentId="node-a-id" />);

    expect(within(screen.getByRole('row')).getByText('failed')).toBeInTheDocument();
    expect(socket.sendToAgent).not.toHaveBeenCalled();
  });
});
