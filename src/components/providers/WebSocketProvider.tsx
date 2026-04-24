'use client';

import { createContext, useContext, useEffect, useRef, useState, useCallback } from 'react';
import { AgentMessagePayload, UiMessage } from '@/lib/types';

interface WebSocketContextValue {
  agents: string[];
  sendMessage: (msg: UiMessage) => void;
  sendToAgent: (agentId: string, message: AgentMessagePayload) => void;
  lastAgentMessage: { agentId: string; message: AgentMessagePayload } | null;
  isConnected: boolean;
}

const WebSocketContext = createContext<WebSocketContextValue | null>(null);

export function WebSocketProvider({ children }: { children: React.ReactNode }) {
  const [agents, setAgents] = useState<string[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [lastAgentMessage, setLastAgentMessage] = useState<{ agentId: string; message: AgentMessagePayload } | null>(null);
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    const wsUrl = process.env.NEXT_PUBLIC_WS_URL || 'wss://dashboard.example.com/ui/ws';
    const ws = new WebSocket(wsUrl);
    wsRef.current = ws;

    ws.onopen = () => {
      setIsConnected(true);
      // Ask for agents
      ws.send(JSON.stringify({ type: 'ListAgentsRequest' }));
    };

    ws.onclose = () => {
      setIsConnected(false);
      setAgents([]);
    };

    ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data) as UiMessage;
        
        if (msg.type === 'ListAgentsResponse') {
          setAgents(msg.payload.agents);
        } else if (msg.type === 'AgentMessage') {
          setLastAgentMessage({ agentId: msg.payload.agent_id, message: msg.payload.message });
          
          // Also check if it's a new agent registering to refresh the list
          if (msg.payload.message.type === 'RegisterAck') {
            ws.send(JSON.stringify({ type: 'ListAgentsRequest' }));
          }
        }
      } catch (e) {
        console.error('Failed to parse WS message:', e);
      }
    };

    return () => {
      ws.close();
    };
  }, []);

  const sendMessage = useCallback((msg: UiMessage) => {
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg));
    }
  }, []);

  const sendToAgent = useCallback((agentId: string, message: AgentMessagePayload) => {
    sendMessage({
      type: 'SendToAgent',
      payload: { agent_id: agentId, message }
    });
  }, [sendMessage]);

  return (
    <WebSocketContext.Provider value={{ agents, sendMessage, sendToAgent, lastAgentMessage, isConnected }}>
      {children}
    </WebSocketContext.Provider>
  );
}

export function useWebSocket() {
  const ctx = useContext(WebSocketContext);
  if (!ctx) throw new Error('useWebSocket must be used within WebSocketProvider');
  return ctx;
}
