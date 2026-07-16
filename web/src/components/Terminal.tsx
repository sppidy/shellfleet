'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useCanWrite } from './providers/SessionProvider';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';

type TerminalProps = {
  agentId: string;
  containerId?: string;
  shell?: string;
  title?: string;
  /**
   * Stable id for this PTY on the agent. Required for host shells so
   * multiple tabs against the same agent each get their own PTY. Auto-
   * generated if absent. Container-exec sessions ignore this and use
   * the wire-level empty-string sentinel.
   */
  sessionId?: string;
  /**
   * When false, the host renders the component but hides it (used by
   * the multi-tab multiplexer to keep PTY state across tab switches).
   * Defaults to true. The xterm instance refits whenever this flips
   * back to true so the dimensions match the now-visible container.
   */
  visible?: boolean;
};

export default function Terminal({
  agentId,
  containerId,
  shell,
  title,
  sessionId,
  visible = true,
}: TerminalProps) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const canWrite = useCanWrite();
  const wrapperRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  // Wire-level session id. Container-exec uses the empty-string
  // sentinel (agent routes on that); host shells get a UUID per
  // mount, either supplied by the parent or generated here.
  const sessionIdRef = useRef<string>(
    containerId ? '' : (sessionId ?? crypto.randomUUID())
  );

  const [isFullscreen, setIsFullscreen] = useState(false);

  useEffect(() => {
    if (!canWrite) return;
    if (!terminalRef.current) return;

    const term = new XTerm({
      cursorBlink: true,
      fontFamily: "JetBrains Mono, ui-monospace, 'SF Mono', Menlo, monospace",
      fontSize: 12,
      scrollback: 5000,
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

    const sid = sessionIdRef.current;

    term.onData((data) => {
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      sendToAgent(agentId, {
        type: 'TerminalData',
        payload: { session_id: sid, data: bytes },
      });
    });

    // ResizeObserver covers everything: window resize, manual splitter
    // drag, fullscreen toggle, multiplexer tab switch (display:none →
    // visible). Single mechanism keeps cols/rows in sync without
    // sprinkling fit() calls everywhere.
    const sendSize = () => {
      try {
        fitAddon.fit();
      } catch {
        /* container with zero size during a transition; ignore */
      }
      sendToAgent(agentId, {
        type: 'TerminalResize',
        payload: { session_id: sid, cols: term.cols, rows: term.rows },
      });
    };
    const ro = new ResizeObserver(() => sendSize());
    if (terminalRef.current) ro.observe(terminalRef.current);
    window.addEventListener('resize', sendSize);

    if (containerId) {
      sendToAgent(agentId, {
        type: 'DockerExecStartRequest',
        payload: { container_id: containerId, shell: shell ?? 'sh' },
      });
    } else {
      sendToAgent(agentId, {
        type: 'StartTerminalRequest',
        payload: { session_id: sid },
      });
    }
    setTimeout(sendSize, 100);

    const unsubscribe = onAgentMessage(agentId, (msg) => {
      // Only render bytes tagged with our session_id. Host shells use
      // their UUID; container-exec uses the empty-string sentinel.
      if (msg.type === 'TerminalData' && msg.payload.session_id === sid) {
        const bytes = new Uint8Array(msg.payload.data);
        xtermRef.current?.write(bytes);
      }
    });

    return () => {
      unsubscribe();
      ro.disconnect();
      window.removeEventListener('resize', sendSize);
      // Tell the agent to drop the PTY rather than waiting for the
      // next WS disconnect to reap it. Container-exec already had
      // this; host shells now do too via StopTerminalRequest, scoped
      // to our session_id so sibling tabs aren't affected.
      if (containerId) {
        sendToAgent(agentId, { type: 'DockerExecStopRequest' });
      } else {
        sendToAgent(agentId, {
          type: 'StopTerminalRequest',
          payload: { session_id: sid },
        });
      }
      term.dispose();
    };
  }, [agentId, sendToAgent, onAgentMessage, containerId, shell, canWrite]);

  // Refit + refocus whenever the pane becomes visible again. Used by
  // the multiplexer when the operator switches tabs.
  useEffect(() => {
    if (!visible) return;
    const t = setTimeout(() => {
      try {
        fitAddonRef.current?.fit();
        xtermRef.current?.focus();
      } catch {
        /* noop */
      }
    }, 0);
    return () => clearTimeout(t);
  }, [visible]);

  // Refit when toggling fullscreen — the available area changes
  // significantly. Also wire ESC to exit.
  useEffect(() => {
    if (!isFullscreen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setIsFullscreen(false);
      }
    };
    window.addEventListener('keydown', onKey);
    const t = setTimeout(() => {
      try {
        fitAddonRef.current?.fit();
        xtermRef.current?.focus();
      } catch {
        /* noop */
      }
    }, 0);
    return () => {
      window.removeEventListener('keydown', onKey);
      clearTimeout(t);
    };
  }, [isFullscreen]);

  const wrapperStyle: React.CSSProperties = isFullscreen
    ? {
        position: 'fixed',
        inset: 0,
        zIndex: 100,
        background: '#06090b',
        display: 'flex',
        flexDirection: 'column',
      }
    : {
        height: '100%',
        width: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: '#06090b',
      };

  return (
    <div ref={wrapperRef} style={{ ...wrapperStyle, display: visible ? wrapperStyle.display : 'none' }}>
      <div
        className="panel-head"
        style={{ background: 'var(--bg-1)', flexShrink: 0 }}
      >
        <div className="panel-title">
          <span className="ico">›_</span> {title ?? 'ROOT SHELL'}
          <span className="meta">root@{agentId.replace(/-id$/, '')}</span>
        </div>
        <div className="panel-actions">
          <button
            type="button"
            className="btn sm icon"
            onClick={() => setIsFullscreen((f) => !f)}
            title={isFullscreen ? 'Exit fullscreen (Esc)' : 'Fullscreen'}
          >
            {isFullscreen ? '⤡' : '⛶'}
          </button>
        </div>
      </div>
      {!canWrite ? (
        <div
          style={{
            flex: 1,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--warn)',
            fontFamily: 'var(--mono)',
            fontSize: 12,
            padding: 24,
            textAlign: 'center',
          }}
        >
          viewer role: interactive shells are admin-only.
          <br />
          ask an admin to promote you at <code>/admin</code>.
        </div>
      ) : (
        <div ref={terminalRef} style={{ flex: 1, overflow: 'hidden', padding: 8 }} />
      )}
    </div>
  );
}
