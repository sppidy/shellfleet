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
import { AgentMessagePayload, UiMessage } from '@/lib/types';
import { effectiveAgentDirectory } from '@/lib/agentDirectory';
import { reconnectDelay } from '@/lib/backoff';
import { apiFetch } from '@/lib/api';
import { useSession } from './SessionProvider';
import { useCoreFleet } from './CoreFleetProvider';
import { useUi } from './UiProvider';

type AgentMessageHandler = (msg: AgentMessagePayload) => void;

interface WebSocketContextValue {
  agents: string[];
  /** Agents currently reachable through the interactive WebSocket. */
  liveAgents: string[];
  /**
   * agent_id → capability list. Empty list means a pre-v15 agent that
   * registered before capability advertisement was added; the dashboard
   * treats that as "show every tab" so legacy agents keep working.
   */
  agentCapabilities: Record<string, string[]>;
  isConnected: boolean;
  sendMessage: (msg: UiMessage) => void;
  sendToAgent: (agentId: string, message: AgentMessagePayload) => void;
  /** Subscribe to messages from a specific agent. Returns an unsubscribe fn. */
  onAgentMessage: (agentId: string, handler: AgentMessageHandler) => () => void;
}

const WebSocketContext = createContext<WebSocketContextValue | null>(null);
const CONNECT_TIMEOUT_MS = 12_000;
const DIRECTORY_SYNC_INTERVAL_MS = 15_000;
const DIRECTORY_STALE_AFTER_MS = 45_000;
const HTTP_SEND_BATCH_DELAY_MS = 12;
const HTTP_SEND_MAX_BATCH = 128;

type HttpControlSession = {
  clientId: number;
  clientToken: string;
};

type HttpControlResponse = {
  messages: UiMessage[];
};

// Resolve at connection time, in the browser, from the page's current
// origin. NEXT_PUBLIC_* values are frozen into Next.js client bundles at build
// time, so a runtime container environment variable can silently point the
// dashboard at a stale or placeholder host. ShellFleet deliberately exposes
// /ui/ws on the same public origin as the dashboard and API.
function resolveWsUrl(): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${window.location.host}/ui/ws`;
}

export function WebSocketProvider({ children }: { children: React.ReactNode }) {
  const { status } = useSession();
  const { hosts } = useCoreFleet();
  const { toast } = useUi();
  // Keep toast in a ref so the WS effect doesn't tear down and reconnect
  // every time React rebinds the callback identity.
  const toastRef = useRef(toast);
  useEffect(() => {
    toastRef.current = toast;
  }, [toast]);
  const [socketAgents, setSocketAgents] = useState<string[]>([]);
  const [socketCapabilities, setSocketCapabilities] = useState<Record<string, string[]>>({});
  const [isConnected, setIsConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const connectTimeout = useRef<ReturnType<typeof setTimeout> | null>(null);
  const directorySyncTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const reconnectAttempt = useRef(0);
  // Installed by the connection effect while the HTTPS control fallback is
  // active. sendMessage stays stable for every consumer and selects WS first.
  const httpSendRef = useRef<((message: UiMessage) => void) | null>(null);
  // Subscribers stored in a ref so message dispatch never races with React's
  // render cycle. The previous implementation kept the "last message" in
  // useState, which dropped events when several messages arrived in the
  // same tick.
  const subscribers = useRef<Map<string, Set<AgentMessageHandler>>>(new Map());

  const dispatch = useCallback((agentId: string, msg: AgentMessagePayload) => {
    const set = subscribers.current.get(agentId);
    if (!set) return;
    for (const handler of set) {
      try {
        handler(msg);
      } catch (e) {
        console.error('agent message handler threw:', e);
      }
    }
  }, []);

  const onAgentMessage = useCallback(
    (agentId: string, handler: AgentMessageHandler) => {
      let set = subscribers.current.get(agentId);
      if (!set) {
        set = new Set();
        subscribers.current.set(agentId, set);
      }
      set.add(handler);
      return () => {
        const current = subscribers.current.get(agentId);
        current?.delete(handler);
        if (current && current.size === 0) {
          subscribers.current.delete(agentId);
        }
      };
    },
    [],
  );

  const handleIncomingMessage = useCallback((msg: UiMessage) => {
    if (msg.type === 'ListAgentsResponse') {
      setIsConnected(true);
      setSocketAgents(msg.payload.agents);
      setSocketCapabilities(msg.payload.capabilities ?? {});
    } else if (msg.type === 'AgentMessage') {
      dispatch(msg.payload.agent_id, msg.payload.message);
    } else if (msg.type === 'PermissionDenied') {
      const { variant_type, reason } = msg.payload;
      if (variant_type === 'approval_pending') {
        toastRef.current('info', reason);
      } else {
        toastRef.current('error', `${variant_type} denied: ${reason}`);
      }
    }
  }, [dispatch]);

  useEffect(() => {
    // Only open the WS once the session is fully authed. Connecting
    // earlier (during /login, /mfa, /security with a pending-MFA
    // cookie) just gets us 403'd by the server's WS-RBAC layer and
    // produces a reconnect storm in the console + audit log.
    if (status !== 'authed') {
      setIsConnected(false);
      setSocketAgents([]);
      setSocketCapabilities({});
      return;
    }
    let disposed = false;
    let lastDirectoryResponseAt = 0;
    let httpSession: HttpControlSession | null = null;
    let httpStarting = false;
    let httpSending = false;
    let httpQueue: UiMessage[] = [];
    let httpFlushTimer: ReturnType<typeof setTimeout> | null = null;
    let httpPollAbort: AbortController | null = null;
    let httpDirectoryTimer: ReturnType<typeof setInterval> | null = null;
    reconnectAttempt.current = 0;

    const clearReconnectTimer = () => {
      if (reconnectTimer.current !== null) {
        clearTimeout(reconnectTimer.current);
        reconnectTimer.current = null;
      }
    };

    const clearConnectionTimers = () => {
      if (connectTimeout.current !== null) {
        clearTimeout(connectTimeout.current);
        connectTimeout.current = null;
      }
      if (directorySyncTimer.current !== null) {
        clearInterval(directorySyncTimer.current);
        directorySyncTimer.current = null;
      }
      if (httpDirectoryTimer !== null) {
        clearInterval(httpDirectoryTimer);
        httpDirectoryTimer = null;
      }
    };

    const resetSocketState = () => {
      setIsConnected(false);
      setSocketAgents([]);
      setSocketCapabilities({});
    };

    const receiveUiMessage = (message: UiMessage) => {
      if (message.type === 'ListAgentsResponse') {
        lastDirectoryResponseAt = Date.now();
      }
      handleIncomingMessage(message);
    };

    const disconnectHttpSession = (session: HttpControlSession) => {
      // `keepalive` gives browsers a chance to release the server-side slot
      // during reload/navigation. The origin's idle reaper remains the final
      // cleanup path if the network is already unavailable.
      void apiFetch('/api/ui/disconnect', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          client_id: session.clientId,
          client_token: session.clientToken,
        }),
        keepalive: true,
      }).catch(() => {});
    };

    const failHttpTransport = () => {
      if (!httpSession && !httpStarting) return;
      const failedSession = httpSession;
      httpSession = null;
      httpStarting = false;
      httpSendRef.current = null;
      httpPollAbort?.abort();
      httpPollAbort = null;
      if (httpFlushTimer !== null) {
        clearTimeout(httpFlushTimer);
        httpFlushTimer = null;
      }
      if (httpDirectoryTimer !== null) {
        clearInterval(httpDirectoryTimer);
        httpDirectoryTimer = null;
      }
      // A failed POST may have reached the origin even if its response did
      // not reach us. Never replay queued control actions automatically.
      httpQueue = [];
      if (failedSession) disconnectHttpSession(failedSession);
      resetSocketState();
      scheduleReconnect();
    };

    const flushHttpQueue = async () => {
      if (disposed || httpSending || !httpSession || httpQueue.length === 0) return;
      httpSending = true;
      const session = httpSession;
      const messages = httpQueue.splice(0, HTTP_SEND_MAX_BATCH);
      try {
        const response = await apiFetch('/api/ui/send', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            client_id: session.clientId,
            client_token: session.clientToken,
            messages,
          }),
        });
        if (!response.ok) throw new Error(`HTTP control send returned ${response.status}`);
      } catch (error) {
        if (!disposed && httpSession === session) {
          console.warn('[shellfleet] HTTPS control send failed:', error);
          failHttpTransport();
        }
      } finally {
        httpSending = false;
        if (!disposed && httpSession && httpQueue.length > 0 && httpFlushTimer === null) {
          httpFlushTimer = setTimeout(() => {
            httpFlushTimer = null;
            void flushHttpQueue();
          }, HTTP_SEND_BATCH_DELAY_MS);
        }
      }
    };

    const enqueueHttpMessage = (message: UiMessage) => {
      if (!httpSession || disposed) return;
      httpQueue.push(message);
      if (httpFlushTimer === null) {
        httpFlushTimer = setTimeout(() => {
          httpFlushTimer = null;
          void flushHttpQueue();
        }, HTTP_SEND_BATCH_DELAY_MS);
      }
    };

    const pollHttp = async (session: HttpControlSession) => {
      while (!disposed && httpSession === session) {
        const controller = new AbortController();
        httpPollAbort = controller;
        try {
          const response = await apiFetch('/api/ui/poll', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              client_id: session.clientId,
              client_token: session.clientToken,
            }),
            signal: controller.signal,
          });
          if (!response.ok) throw new Error(`HTTP control poll returned ${response.status}`);
          const body = (await response.json()) as HttpControlResponse;
          for (const message of body.messages ?? []) receiveUiMessage(message);
        } catch (error) {
          if (!disposed && !controller.signal.aborted && httpSession === session) {
            console.warn('[shellfleet] HTTPS control poll failed:', error);
            failHttpTransport();
          }
          return;
        } finally {
          if (httpPollAbort === controller) httpPollAbort = null;
        }
      }
    };

    async function startHttpFallback() {
      if (disposed || httpStarting || httpSession) return;
      httpStarting = true;
      clearReconnectTimer();
      const current = wsRef.current;
      wsRef.current = null;
      try {
        current?.close();
      } catch {
        /* ignore the failed WebSocket while switching transports */
      }
      try {
        const response = await apiFetch('/api/ui/connect', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: '{}',
        });
        if (!response.ok) throw new Error(`HTTP control connect returned ${response.status}`);
        const body = (await response.json()) as HttpControlResponse & {
          client_id: number;
          client_token: string;
        };
        if (!Number.isSafeInteger(body.client_id) || !body.client_token) {
          throw new Error('HTTP control connect returned an invalid session');
        }
        if (disposed) return;
        const session = { clientId: body.client_id, clientToken: body.client_token };
        httpSession = session;
        httpSendRef.current = enqueueHttpMessage;
        reconnectAttempt.current = 0;
        for (const message of body.messages ?? []) receiveUiMessage(message);
        httpDirectoryTimer = setInterval(() => {
          if (Date.now() - lastDirectoryResponseAt >= DIRECTORY_STALE_AFTER_MS) {
            failHttpTransport();
            return;
          }
          enqueueHttpMessage({ type: 'ListAgentsRequest' });
        }, DIRECTORY_SYNC_INTERVAL_MS);
        console.info('[shellfleet] interactive controls using HTTPS fallback');
        void pollHttp(session);
      } catch (error) {
        if (!disposed) {
          console.warn('[shellfleet] HTTPS control fallback unavailable:', error);
          httpStarting = false;
          scheduleReconnect();
        }
      } finally {
        httpStarting = false;
      }
    }

    const requestDirectory = (ws: WebSocket) => {
      if (ws.readyState !== WebSocket.OPEN) return false;
      try {
        ws.send(JSON.stringify({ type: 'ListAgentsRequest' } satisfies UiMessage));
        return true;
      } catch {
        retire(ws);
        return false;
      }
    };

    const scheduleReconnect = () => {
      if (disposed || httpSession || httpStarting || reconnectTimer.current !== null) {
        return;
      }
      const delay = reconnectDelay(reconnectAttempt.current);
      reconnectAttempt.current += 1;
      reconnectTimer.current = setTimeout(() => {
        reconnectTimer.current = null;
        connect();
      }, delay);
    };

    const retire = (ws: WebSocket, closeSocket = true) => {
      if (wsRef.current !== ws) return;
      wsRef.current = null;
      clearConnectionTimers();
      resetSocketState();
      if (closeSocket) {
        try {
          ws.close();
        } catch {
          /* the browser can throw while a socket is still being created */
        }
      }
      void startHttpFallback();
    };

    const connect = () => {
      if (disposed || httpSession || httpStarting) {
        return;
      }
      const current = wsRef.current;
      if (
        current &&
        (current.readyState === WebSocket.CONNECTING || current.readyState === WebSocket.OPEN)
      ) {
        return;
      }

      let ws: WebSocket;
      try {
        ws = new WebSocket(resolveWsUrl());
      } catch (error) {
        console.error('[shellfleet] failed to create UI WebSocket:', error);
        void startHttpFallback();
        return;
      }
      wsRef.current = ws;
      lastDirectoryResponseAt = 0;

      connectTimeout.current = setTimeout(() => {
        if (wsRef.current === ws && ws.readyState === WebSocket.CONNECTING) {
          retire(ws);
        }
      }, CONNECT_TIMEOUT_MS);

      ws.onopen = () => {
        if (disposed || wsRef.current !== ws) {
          try {
            ws.close();
          } catch {
            /* ignore a stale socket */
          }
          return;
        }
        if (connectTimeout.current !== null) {
          clearTimeout(connectTimeout.current);
          connectTimeout.current = null;
        }
        reconnectAttempt.current = 0;
        lastDirectoryResponseAt = Date.now();
        if (!requestDirectory(ws)) return;
        directorySyncTimer.current = setInterval(() => {
          if (wsRef.current !== ws) return;
          if (
            ws.readyState !== WebSocket.OPEN ||
            Date.now() - lastDirectoryResponseAt >= DIRECTORY_STALE_AFTER_MS
          ) {
            retire(ws);
            return;
          }
          // This existing request/response pair doubles as an application-level
          // heartbeat that browser JavaScript can observe. Protocol-level Pong
          // frames are handled internally by the browser and cannot detect a
          // half-open connection from this provider.
          requestDirectory(ws);
        }, DIRECTORY_SYNC_INTERVAL_MS);
      };

      ws.onclose = (event) => {
        if (!disposed && event.code !== 1000) {
          console.warn(
            `[shellfleet] UI WebSocket closed (code=${event.code}, reason=${event.reason || 'none'})`,
          );
        }
        if (!disposed) retire(ws, false);
      };

      ws.onerror = () => {
        retire(ws);
      };

      ws.onmessage = (event) => {
        if (disposed || wsRef.current !== ws) return;
        try {
          const msg = JSON.parse(event.data) as UiMessage;
          receiveUiMessage(msg);
        } catch (e) {
          console.error('failed to parse WS message:', e);
        }
      };
    };

    const recoverNow = () => {
      if (disposed) return;
      if (httpSession) {
        enqueueHttpMessage({ type: 'ListAgentsRequest' });
        return;
      }
      clearReconnectTimer();
      const ws = wsRef.current;
      if (!ws || ws.readyState === WebSocket.CLOSED || ws.readyState === WebSocket.CLOSING) {
        connect();
        return;
      }
      if (ws.readyState === WebSocket.OPEN) {
        if (Date.now() - lastDirectoryResponseAt >= DIRECTORY_STALE_AFTER_MS) {
          retire(ws);
        } else {
          requestDirectory(ws);
        }
      }
    };

    const handleVisibility = () => {
      if (document.visibilityState === 'visible') recoverNow();
    };

    // `navigator.onLine` and the corresponding `offline` event are only
    // connectivity hints. Mobile browsers, VPNs, and captive portals can
    // report offline while same-origin HTTP is already succeeding. Let the
    // actual WebSocket handshake decide reachability; failed attempts remain
    // bounded by the reconnect backoff.
    window.addEventListener('online', recoverNow);
    document.addEventListener('visibilitychange', handleVisibility);

    connect();

    return () => {
      disposed = true;
      window.removeEventListener('online', recoverNow);
      document.removeEventListener('visibilitychange', handleVisibility);
      clearReconnectTimer();
      clearConnectionTimers();
      httpSendRef.current = null;
      const activeHttpSession = httpSession;
      httpSession = null;
      httpStarting = false;
      httpPollAbort?.abort();
      if (httpFlushTimer !== null) clearTimeout(httpFlushTimer);
      httpQueue = [];
      if (activeHttpSession) disconnectHttpSession(activeHttpSession);
      const ws = wsRef.current;
      wsRef.current = null;
      try {
        ws?.close();
      } catch {
        /* ignore cleanup errors */
      }
    };
  }, [handleIncomingMessage, status]);

  const sendMessage = useCallback((msg: UiMessage) => {
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      try {
        wsRef.current.send(JSON.stringify(msg));
      } catch {
        // The liveness loop owns reconnecting. A send can race with a network
        // transition between the readyState check and the browser write.
        try {
          wsRef.current.close();
        } catch {
          /* ignore */
        }
      }
      return;
    }
    httpSendRef.current?.(msg);
  }, []);

  const sendToAgent = useCallback(
    (agentId: string, message: AgentMessagePayload) => {
      sendMessage({
        type: 'SendToAgent',
        payload: { agent_id: agentId, message },
      });
    },
    [sendMessage],
  );

  const directory = useMemo(
    () => effectiveAgentDirectory(hosts, socketAgents, socketCapabilities),
    [hosts, socketAgents, socketCapabilities],
  );

  return (
    <WebSocketContext.Provider
      value={{
        agents: directory.agents,
        liveAgents: socketAgents,
        agentCapabilities: directory.capabilities,
        isConnected,
        sendMessage,
        sendToAgent,
        onAgentMessage,
      }}
    >
      {children}
    </WebSocketContext.Provider>
  );
}

export function useWebSocket() {
  const ctx = useContext(WebSocketContext);
  if (!ctx) throw new Error('useWebSocket must be used within WebSocketProvider');
  return ctx;
}
