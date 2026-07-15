import '@testing-library/jest-dom/vitest';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CoreFleetProvider, useCoreFleet } from '../CoreFleetProvider';

const session = vi.hoisted(() => ({ status: 'authed' }));

vi.mock('../SessionProvider', () => ({
  useSession: () => ({ status: session.status }),
}));

type EventHandler = (event: Event) => void;

class MockEventSource {
  static current: MockEventSource | null = null;

  onopen: EventHandler | null = null;
  onerror: EventHandler | null = null;
  readonly close = vi.fn();
  private readonly listeners = new Map<string, Set<EventHandler>>();

  constructor() {
    MockEventSource.current = this;
  }

  addEventListener(type: string, listener: EventHandler) {
    const listeners = this.listeners.get(type) ?? new Set<EventHandler>();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }

  open() {
    this.onopen?.(new Event('open'));
  }

  fail() {
    this.onerror?.(new Event('error'));
  }

  emitFleet() {
    for (const listener of this.listeners.get('fleet') ?? []) {
      listener(new Event('fleet'));
    }
  }
}

function host(agentId: string, status: 'online' | 'offline' = 'offline') {
  return {
    agent_id: agentId,
    hostname: agentId.replace(/-id$/, ''),
    status,
    protocol_version: 19,
    capabilities: ['systemd'],
    metadata: {},
    first_seen_at: 100,
    last_seen_at: 101,
    disconnected_at: status === 'offline' ? 102 : null,
    system: null,
    services: null,
    docker: null,
    swarm: null,
  };
}

function fleetResponse(hosts: ReturnType<typeof host>[]) {
  return new Response(
    JSON.stringify({ generated_at: 110, offline_after_seconds: 45, hosts }),
    { status: 200, headers: { 'content-type': 'application/json' } },
  );
}

function Probe() {
  const fleet = useCoreFleet();
  return (
    <div>
      <span>{fleet.liveStatus}</span>
      {fleet.hosts.map((item) => (
        <span key={item.agent_id}>{`${item.agent_id}:${item.status}`}</span>
      ))}
    </div>
  );
}

describe('CoreFleetProvider', () => {
  beforeEach(() => {
    session.status = 'authed';
    MockEventSource.current = null;
    vi.stubGlobal('EventSource', MockEventSource);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('keeps durable hosts visible when the event stream fails', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(fleetResponse([host('node-a-id')])));
    render(
      <CoreFleetProvider>
        <Probe />
      </CoreFleetProvider>,
    );

    expect(await screen.findByText('node-a-id:offline')).toBeInTheDocument();
    act(() => MockEventSource.current?.fail());

    expect(screen.getByText('node-a-id:offline')).toBeInTheDocument();
    expect(screen.getByText('degraded')).toBeInTheDocument();
  });

  it('refetches durable state after a fleet event', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(fleetResponse([host('node-a-id')]))
      .mockResolvedValueOnce(fleetResponse([host('node-a-id', 'online')]));
    vi.stubGlobal('fetch', fetchMock);
    render(
      <CoreFleetProvider>
        <Probe />
      </CoreFleetProvider>,
    );

    expect(await screen.findByText('node-a-id:offline')).toBeInTheDocument();
    act(() => MockEventSource.current?.emitFleet());

    expect(await screen.findByText('node-a-id:online')).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('closes the stream and clears fleet state when the session ends', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(fleetResponse([host('node-a-id')])));
    const view = render(
      <CoreFleetProvider>
        <Probe />
      </CoreFleetProvider>,
    );

    expect(await screen.findByText('node-a-id:offline')).toBeInTheDocument();
    const stream = MockEventSource.current;
    session.status = 'guest';
    view.rerender(
      <CoreFleetProvider>
        <Probe />
      </CoreFleetProvider>,
    );

    await waitFor(() => expect(screen.queryByText('node-a-id:offline')).not.toBeInTheDocument());
    expect(stream?.close).toHaveBeenCalledOnce();
  });
});
