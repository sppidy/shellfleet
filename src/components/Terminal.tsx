'use client';

import { useEffect, useRef } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';

type TerminalProps = {
  agentId: string;
  containerId?: string;
  shell?: string;
  title?: string;
};

export default function Terminal({ agentId, containerId, shell, title }: TerminalProps) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    if (!terminalRef.current) return;

    const term = new XTerm({
      cursorBlink: true,
      fontFamily: "JetBrains Mono, ui-monospace, 'SF Mono', Menlo, monospace",
      fontSize: 12,
      theme: {
        background: '#06090b',
        foreground: '#c8d3dc',
        cursor: '#7fb069',
        cursorAccent: '#06090b',
        selectionBackground: 'rgba(127,176,105,0.25)',
        black: '#0a0d0f',
        red: '#e57373',
        green: '#7fb069',
        yellow: '#e6b450',
        blue: '#82a8d4',
        magenta: '#c885c4',
        cyan: '#6ec1c1',
        white: '#d8dee5',
        brightBlack: '#4a525b',
        brightRed: '#e57373',
        brightGreen: '#a8d5a0',
        brightYellow: '#f0c878',
        brightBlue: '#82a8d4',
        brightMagenta: '#d9a3d6',
        brightCyan: '#93d4d4',
        brightWhite: '#ffffff',
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

    if (containerId) {
      sendToAgent(agentId, {
        type: 'DockerExecStartRequest',
        payload: { container_id: containerId, shell: shell ?? 'sh' },
      });
    } else {
      sendToAgent(agentId, { type: 'StartTerminalRequest' });
    }
    setTimeout(() => handleResize(), 100);

    const unsubscribe = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'TerminalData') {
        const bytes = new Uint8Array(msg.payload.data);
        xtermRef.current?.write(bytes);
      }
    });

    return () => {
      unsubscribe();
      window.removeEventListener('resize', handleResize);
      if (containerId) {
        sendToAgent(agentId, { type: 'DockerExecStopRequest' });
      }
      term.dispose();
    };
  }, [agentId, sendToAgent, onAgentMessage, containerId, shell]);

  return (
    <div
      style={{
        height: '100%',
        width: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: '#06090b',
      }}
    >
      <div
        className="panel-head"
        style={{ background: 'var(--bg-1)', flexShrink: 0 }}
      >
        <div className="panel-title">
          <span className="ico">›_</span> {title ?? 'SHELL'}
          <span className="meta">root@{agentId.replace(/-id$/, '')}</span>
        </div>
      </div>
      <div ref={terminalRef} style={{ flex: 1, overflow: 'hidden', padding: 8 }} />
    </div>
  );
}
