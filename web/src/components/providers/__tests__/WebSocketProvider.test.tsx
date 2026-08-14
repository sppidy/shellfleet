import '@testing-library/jest-dom/vitest';
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WebSocketProvider, useWebSocket } from '../WebSocketProvider';

const session = vi.hoisted(() => ({ status: 'authed' }));
const ui = vi.hoisted(() => ({ toast: vi.fn() }));

vi.mock('../SessionProvider', () => ({
  useSession: () => ({ status: session.status }),
}));

vi.mock('../CoreFleetProvider', () => ({
  useCoreFleet: () => ({ hosts: [] }),
}));

vi.mock('../UiProvider', () => ({
  useUi: () => ({ toast: ui.toast }),
}));

type EventListener<T> = (event: T) => void;

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readyState = MockWebSocket.CONNECTING;
  readonly sent: string[] = [];
  onopen: EventListener<Event> | null = null;
  onclose: EventListener<CloseEvent> | null = null;
  onerror: EventListener<Event> | null = null;
  onmessage: EventListener<MessageEvent> | null = null;

  constructor(readonly url: string) {
    MockWebSocket.instances.push(this);
  }

  open() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.(new Event('open'));
  }

  message(value: unknown) {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(value) }));
  }

  send(value: string) {
    if (this.readyState !== MockWebSocket.OPEN) throw new Error('socket is not open');
    this.sent.push(value);
  }

  close() {
    if (this.readyState === MockWebSocket.CLOSED) return;
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.(new CloseEvent('close'));
  }

  serverClose() {
    this.close();
  }
}

function Probe() {
  const { agents, isConnected, liveAgents } = useWebSocket();
  return (
    <div>
      <span>{isConnected ? 'connected' : 'disconnected'}</span>
      <span>{`agents:${agents.join(',')}`}</span>
      <span>{`live:${liveAgents.join(',')}`}</span>
    </div>
  );
}

function renderProvider() {
  return render(
    <WebSocketProvider>
      <Probe />
    </WebSocketProvider>,
  );
}

describe('WebSocketProvider', () => {
  beforeEach(() => {
    session.status = 'authed';
    ui.toast.mockReset();
    MockWebSocket.instances = [];
    vi.useFakeTimers();
    vi.stubGlobal('WebSocket', MockWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('keeps the live directory synchronized with an observable request/response heartbeat', () => {
    renderProvider();
    const socket = MockWebSocket.instances[0];

    act(() => socket.open());
    expect(screen.getByText('disconnected')).toBeInTheDocument();
    expect(JSON.parse(socket.sent[0])).toEqual({ type: 'ListAgentsRequest' });

    act(() => {
      socket.message({
        type: 'ListAgentsResponse',
        payload: { agents: ['node-a-id'], capabilities: { 'node-a-id': ['systemd'] } },
      });
    });
    expect(screen.getByText('connected')).toBeInTheDocument();
    expect(screen.getByText('agents:node-a-id')).toBeInTheDocument();
    expect(screen.getByText('live:node-a-id')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(15_000));
    expect(socket.sent).toHaveLength(2);
    expect(JSON.parse(socket.sent[1])).toEqual({ type: 'ListAgentsRequest' });
  });

  it('retires a half-open socket and reconnects when directory responses stop', () => {
    renderProvider();
    const socket = MockWebSocket.instances[0];
    act(() => {
      socket.open();
      socket.message({
        type: 'ListAgentsResponse',
        payload: { agents: ['node-a-id'], capabilities: {} },
      });
    });

    act(() => vi.advanceTimersByTime(45_000));
    expect(socket.readyState).toBe(MockWebSocket.CLOSED);
    expect(screen.getByText('disconnected')).toBeInTheDocument();
    expect(screen.getByText('agents:')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(MockWebSocket.instances).toHaveLength(2);
    expect(MockWebSocket.instances[1].readyState).toBe(MockWebSocket.CONNECTING);
  });

  it('does not wait forever for the opening handshake', () => {
    renderProvider();
    const socket = MockWebSocket.instances[0];

    act(() => vi.advanceTimersByTime(12_000));
    expect(socket.readyState).toBe(MockWebSocket.CLOSED);

    act(() => vi.advanceTimersByTime(1_000));
    expect(MockWebSocket.instances).toHaveLength(2);
  });

  it('reconnects immediately when the browser reports that the network returned', () => {
    renderProvider();
    const socket = MockWebSocket.instances[0];
    act(() => {
      socket.open();
      socket.serverClose();
    });
    expect(MockWebSocket.instances).toHaveLength(1);

    act(() => window.dispatchEvent(new Event('online')));
    expect(MockWebSocket.instances).toHaveLength(2);
  });

  it('still probes the real transport when navigator.onLine is stale', () => {
    vi.spyOn(window.navigator, 'onLine', 'get').mockReturnValue(false);

    renderProvider();

    expect(MockWebSocket.instances).toHaveLength(1);
    expect(MockWebSocket.instances[0].url).toBe('ws://localhost:3000/ui/ws');
  });

  it('ignores late messages from a retired socket generation', () => {
    renderProvider();
    const first = MockWebSocket.instances[0];
    act(() => {
      first.open();
      first.serverClose();
      window.dispatchEvent(new Event('online'));
    });
    const second = MockWebSocket.instances[1];
    act(() => {
      second.open();
      second.message({
        type: 'ListAgentsResponse',
        payload: { agents: ['current-id'], capabilities: {} },
      });
      first.message({
        type: 'ListAgentsResponse',
        payload: { agents: ['stale-id'], capabilities: {} },
      });
    });

    expect(screen.getByText('agents:current-id')).toBeInTheDocument();
    expect(screen.queryByText('agents:stale-id')).not.toBeInTheDocument();
  });

  it('does not let a disposed connection schedule a stale reconnect', () => {
    const view = renderProvider();
    const socket = MockWebSocket.instances[0];

    session.status = 'guest';
    view.rerender(
      <WebSocketProvider>
        <Probe />
      </WebSocketProvider>,
    );
    expect(socket.readyState).toBe(MockWebSocket.CLOSED);

    act(() => vi.advanceTimersByTime(60_000));
    expect(MockWebSocket.instances).toHaveLength(1);
  });
});
