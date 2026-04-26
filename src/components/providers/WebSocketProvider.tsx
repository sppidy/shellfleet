'use client';

import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { AgentMessagePayload, UiMessage } from '@/lib/types';

type AgentMessageHandler = (msg: AgentMessagePayload) => void;

interface WebSocketContextValue {
  agents: string[];
  isConnected: boolean;
  sendMessage: (msg: UiMessage) => void;
  sendToAgent: (agentId: string, message: AgentMessagePayload) => void;
  /** Subscribe to messages from a specific agent. Returns an unsubscribe fn. */
  onAgentMessage: (agentId: string, handler: AgentMessageHandler) => () => void;
}

const WebSocketContext = createContext<WebSocketContextValue | null>(null);

// Resolve the WS URL once on import. Order of precedence:
//   1. NEXT_PUBLIC_WS_URL — explicit override baked at build time, used
//      when web and server live on different hosts.
//   2. window.location — same-origin /ui/ws, derived per request. This
//      makes a fresh deploy "just work" wherever it's hosted, no env
//      var or rebuild needed.
//   3. SSR placeholder — never actually reached by the browser, but
//      keeps TypeScript happy and avoids accidental crashes if the
//      provider is ever evaluated outside a browser.
function resolveWsUrl(): string {
  if (typeof process !== 'undefined' && process.env.NEXT_PUBLIC_WS_URL) {
    return process.env.NEXT_PUBLIC_WS_URL;
  }
  if (typeof window !== 'undefined') {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${proto}//${window.location.host}/ui/ws`;
  }
  return 'wss://dashboard.example.com/ui/ws';
}
const WS_URL = resolveWsUrl();

export function WebSocketProvider({ children }: { children: React.ReactNode }) {
  const [agents, setAgents] = useState<string[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectAttempt = useRef(0);
  const stoppedRef = useRef(false);
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

  useEffect(() => {
    stoppedRef.current = false;

    const connect = () => {
      const ws = new WebSocket(WS_URL);
      wsRef.current = ws;

      ws.onopen = () => {
        reconnectAttempt.current = 0;
        setIsConnected(true);
        ws.send(JSON.stringify({ type: 'ListAgentsRequest' } satisfies UiMessage));
      };

      ws.onclose = () => {
        setIsConnected(false);
        setAgents([]);
        if (stoppedRef.current) return;
        // Exponential backoff capped at 15s. The provider auto-reconnects so
        // momentary network blips don't leave the dashboard stuck.
        const attempt = Math.min(reconnectAttempt.current, 5);
        const delay = Math.min(1000 * 2 ** attempt, 15000);
        reconnectAttempt.current += 1;
        reconnectTimer.current = setTimeout(connect, delay);
      };

      ws.onerror = () => {
        // onclose will fire too; just close to be sure.
        try {
          ws.close();
        } catch {
          /* ignore */
        }
      };

      ws.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data) as UiMessage;
          if (msg.type === 'ListAgentsResponse') {
            setAgents(msg.payload.agents);
          } else if (msg.type === 'AgentMessage') {
            dispatch(msg.payload.agent_id, msg.payload.message);
          }
        } catch (e) {
          console.error('failed to parse WS message:', e);
        }
      };
    };

    connect();

    return () => {
      stoppedRef.current = true;
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      wsRef.current?.close();
    };
  }, [dispatch]);

  const sendMessage = useCallback((msg: UiMessage) => {
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg));
    }
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

  return (
    <WebSocketContext.Provider
      value={{ agents, isConnected, sendMessage, sendToAgent, onAgentMessage }}
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
