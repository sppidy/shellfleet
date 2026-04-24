'use client';

import { useEffect, useRef } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';

export default function Terminal({ agentId }: { agentId: string }) {
  const { sendToAgent, lastAgentMessage } = useWebSocket();
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    if (!terminalRef.current) return;

    // Initialize xterm.js
    const term = new XTerm({
      cursorBlink: true,
      theme: {
        background: '#020617', // slate-950
        foreground: '#f8fafc', // slate-50
      },
    });
    
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(terminalRef.current);
    fitAddon.fit();

    xtermRef.current = term;
    fitAddonRef.current = fitAddon;

    // Handle user input
    term.onData((data) => {
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      sendToAgent(agentId, {
        type: 'TerminalData',
        payload: { data: bytes }
      });
    });

    // Handle resize
    const handleResize = () => {
      fitAddon.fit();
      sendToAgent(agentId, {
        type: 'TerminalResize',
        payload: { cols: term.cols, rows: term.rows }
      });
    };
    window.addEventListener('resize', handleResize);
    
    // Start terminal session
    sendToAgent(agentId, { type: 'StartTerminalRequest' });

    // Initial resize to inform backend
    setTimeout(() => handleResize(), 100);

    return () => {
      window.removeEventListener('resize', handleResize);
      term.dispose();
    };
  }, [agentId, sendToAgent]);

  // Listen for terminal data from the agent
  useEffect(() => {
    if (lastAgentMessage && lastAgentMessage.agentId === agentId) {
      const { message } = lastAgentMessage;
      if (message.type === 'TerminalData' && xtermRef.current) {
        const bytes = new Uint8Array(message.payload.data);
        xtermRef.current.write(bytes);
      }
    }
  }, [lastAgentMessage, agentId]);

  return (
    <div className="h-full w-full p-2 flex flex-col">
      <div className="flex justify-between items-center mb-2 px-2">
        <h3 className="text-slate-300 font-medium">Terminal</h3>
      </div>
      <div ref={terminalRef} className="flex-1 overflow-hidden" />
    </div>
  );
}
