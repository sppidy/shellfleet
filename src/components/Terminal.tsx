'use client';

import { useEffect, useRef } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';

export default function Terminal({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    if (!terminalRef.current) return;

    const term = new XTerm({
      cursorBlink: true,
      theme: {
        background: '#020617',
        foreground: '#f8fafc',
      },
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(terminalRef.current);
    fitAddon.fit();

    xtermRef.current = term;
    fitAddonRef.current = fitAddon;

    term.onData((data) => {
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      sendToAgent(agentId, {
        type: 'TerminalData',
        payload: { data: bytes },
      });
    });

    const handleResize = () => {
      fitAddon.fit();
      sendToAgent(agentId, {
        type: 'TerminalResize',
        payload: { cols: term.cols, rows: term.rows },
      });
    };
    window.addEventListener('resize', handleResize);

    sendToAgent(agentId, { type: 'StartTerminalRequest' });
    setTimeout(() => handleResize(), 100);

    // Subscribe directly so every TerminalData chunk is delivered. The
    // earlier "lastAgentMessage" approach lost output when several chunks
    // arrived in the same React tick.
    const unsubscribe = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'TerminalData') {
        const bytes = new Uint8Array(msg.payload.data);
        xtermRef.current?.write(bytes);
      }
    });

    return () => {
      unsubscribe();
      window.removeEventListener('resize', handleResize);
      term.dispose();
    };
  }, [agentId, sendToAgent, onAgentMessage]);

  return (
    <div className="h-full w-full p-2 flex flex-col">
      <div className="flex justify-between items-center mb-2 px-2">
        <h3 className="text-slate-300 font-medium">Terminal</h3>
      </div>
      <div ref={terminalRef} className="flex-1 overflow-hidden" />
    </div>
  );
}
