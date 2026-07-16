'use client';

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import {
  FleetApiError,
  fetchFleet,
  snapshotsByAgent,
  type CoreAgentSnapshot,
  type CoreLiveStatus,
  type FleetHost,
} from '@/lib/coreFleet';
import { useSession } from './SessionProvider';

type CoreFleetContextValue = {
  hosts: FleetHost[];
  snapshots: Record<string, CoreAgentSnapshot>;
  liveStatus: CoreLiveStatus;
  loading: boolean;
  error: string | null;
  refresh: () => void;
};

const CoreFleetContext = createContext<CoreFleetContextValue | null>(null);
const SSE_REFRESH_DELAY_MS = 1_000;

function errorMessage(error: unknown): string {
  if (error instanceof FleetApiError) return error.code;
  if (error instanceof Error) return error.message;
  return 'fleet_unavailable';
}

export function CoreFleetProvider({ children }: { children: React.ReactNode }) {
  const { status } = useSession();
  const [hosts, setHosts] = useState<FleetHost[]>([]);
  const [liveStatus, setLiveStatus] = useState<CoreLiveStatus>('connecting');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlightRef = useRef<Promise<void> | null>(null);
  const trailingRefreshRef = useRef(false);
  const abortRef = useRef<AbortController | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const eventRefreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const generationRef = useRef(0);

  const load = useCallback(function loadFleet() {
    if (inFlightRef.current) {
      trailingRefreshRef.current = true;
      return;
    }

    const generation = generationRef.current;
    const controller = new AbortController();
    abortRef.current = controller;
    setLoading(true);

    const request = fetchFleet(controller.signal)
      .then((fleet) => {
        if (generation !== generationRef.current) return;
        setHosts(fleet.hosts);
        setError(null);
      })
      .catch((requestError: unknown) => {
        if (controller.signal.aborted || generation !== generationRef.current) return;
        setError(errorMessage(requestError));
      })
      .finally(() => {
        if (inFlightRef.current !== request) return;
        inFlightRef.current = null;
        if (abortRef.current === controller) abortRef.current = null;
        if (generation !== generationRef.current) return;
        setLoading(false);
        if (trailingRefreshRef.current) {
          trailingRefreshRef.current = false;
          loadFleet();
        }
      });
    inFlightRef.current = request;
  }, []);

  useEffect(() => {
    generationRef.current += 1;
    const generation = generationRef.current;

    if (status !== 'authed') {
      abortRef.current?.abort();
      eventSourceRef.current?.close();
      if (eventRefreshTimerRef.current !== null) {
        clearTimeout(eventRefreshTimerRef.current);
      }
      abortRef.current = null;
      eventSourceRef.current = null;
      eventRefreshTimerRef.current = null;
      inFlightRef.current = null;
      trailingRefreshRef.current = false;
      setHosts([]);
      setError(null);
      setLoading(false);
      setLiveStatus('connecting');
      return;
    }

    setLiveStatus('connecting');
    load();
    const eventSource = new EventSource('/api/core/v1/events', { withCredentials: true });
    eventSourceRef.current = eventSource;
    eventSource.onopen = () => {
      if (generation === generationRef.current) setLiveStatus('live');
    };
    eventSource.onerror = () => {
      if (generation === generationRef.current) setLiveStatus('degraded');
    };
    eventSource.addEventListener('fleet', () => {
      if (generation !== generationRef.current) return;
      if (eventRefreshTimerRef.current !== null) {
        clearTimeout(eventRefreshTimerRef.current);
      }
      eventRefreshTimerRef.current = setTimeout(() => {
        eventRefreshTimerRef.current = null;
        if (generation === generationRef.current) load();
      }, SSE_REFRESH_DELAY_MS);
    });

    return () => {
      generationRef.current += 1;
      abortRef.current?.abort();
      eventSourceRef.current?.close();
      if (eventRefreshTimerRef.current !== null) {
        clearTimeout(eventRefreshTimerRef.current);
      }
      abortRef.current = null;
      eventSourceRef.current = null;
      eventRefreshTimerRef.current = null;
      inFlightRef.current = null;
      trailingRefreshRef.current = false;
    };
  }, [load, status]);

  const snapshots = useMemo(() => snapshotsByAgent(hosts), [hosts]);
  const value = useMemo<CoreFleetContextValue>(
    () => ({ hosts, snapshots, liveStatus, loading, error, refresh: load }),
    [error, hosts, liveStatus, load, loading, snapshots],
  );

  return <CoreFleetContext.Provider value={value}>{children}</CoreFleetContext.Provider>;
}

export function useCoreFleet(): CoreFleetContextValue {
  const context = useContext(CoreFleetContext);
  if (!context) throw new Error('useCoreFleet must be used within CoreFleetProvider');
  return context;
}
