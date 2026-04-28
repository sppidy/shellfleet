'use client';

import { useEffect, useRef, useState } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { useWebSocket } from './providers/WebSocketProvider';
import { useCanWrite } from './providers/SessionProvider';
import type {
  K8sPod,
  K8sDeployment,
  K8sService,
  K8sIngress,
  K8sPvc,
  K8sEvent,
  AgentMessagePayload,
} from '@/lib/types';

// ──────────────────────────────────────────────────────────────
// Describe target shared across subtabs
// ──────────────────────────────────────────────────────────────

type DescribeTarget = {
  kind: 'pod' | 'deployment' | 'service' | 'ingress' | 'pvc' | 'event';
  namespace: string | null;
  name: string;
};

type LogsTarget = {
  namespace: string;
  podName: string;
  containers: string[];
};

type ExecTarget = {
  namespace: string;
  podName: string;
  containers: string[];
};

// ──────────────────────────────────────────────────────────────
// Subtab map — kept in lockstep with DockerHub's pattern
// ──────────────────────────────────────────────────────────────

export type K8sSubtab =
  | 'pods'
  | 'deployments'
  | 'services'
  | 'ingresses'
  | 'pvcs'
  | 'events';

export const K8S_SUBTABS: K8sSubtab[] = [
  'pods',
  'deployments',
  'services',
  'ingresses',
  'pvcs',
  'events',
];

const SUBTAB_DEFS: { id: K8sSubtab; label: string; hint?: string }[] = [
  { id: 'pods',        label: 'pods',        hint: 'live workloads' },
  { id: 'deployments', label: 'deployments', hint: 'replica sets' },
  { id: 'services',    label: 'services',    hint: 'cluster-ip / nodeport' },
  { id: 'ingresses',   label: 'ingresses',   hint: 'http routing' },
  { id: 'pvcs',        label: 'pvcs',        hint: 'persistent volumes' },
  { id: 'events',      label: 'events',      hint: 'recent activity' },
];

// ──────────────────────────────────────────────────────────────
// Shared helpers
// ──────────────────────────────────────────────────────────────

function fmtAge(secs: number): string {
  if (secs < 0) return '—';
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`;
  return `${Math.floor(secs / 86400)}d`;
}

function phaseStyle(phase: string): React.CSSProperties {
  switch (phase) {
    case 'Running':
    case 'Bound':
    case 'Normal':
      return { color: 'var(--ok, #7fb069)' };
    case 'Pending':
      return { color: 'var(--warn, #e6b450)' };
    case 'Succeeded':
      return { color: 'var(--fg-2)' };
    case 'Failed':
    case 'Lost':
    case 'Warning':
      return { color: 'var(--err, #e57373)' };
    default:
      return { color: 'var(--fg-3)' };
  }
}

// Poll-on-mount helper for the K8sList* round-trips. The caller
// passes a `pluck` callback that narrows the AgentMessagePayload
// union and extracts the typed list — that way each view stays
// fully typesafe without the helper needing to be generic over the
// union shape.
type K8sListResult<T> = { data: T | null; error: string | null; loading: boolean };

function useK8sList<T>(
  agentId: string,
  reqMessage: AgentMessagePayload,
  pluck: (msg: AgentMessagePayload) => { data: T; error: string | null } | null,
  pollMs = 5000,
): K8sListResult<T> {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    setLoading(true);
    setError(null);
    sendToAgent(agentId, reqMessage);

    const unsub = onAgentMessage(agentId, (msg) => {
      const matched = pluck(msg);
      if (!matched) return;
      setLoading(false);
      if (matched.error) {
        setError(matched.error);
        setData(null);
      } else {
        setError(null);
        setData(matched.data);
      }
    });

    const t = setInterval(() => setTick((n) => n + 1), pollMs);
    return () => {
      unsub();
      clearInterval(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, reqMessage.type]);

  useEffect(() => {
    if (tick === 0) return;
    sendToAgent(agentId, reqMessage);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tick]);

  return { data, error, loading };
}

// Matching dark-monospace table style across all subtabs.
const tdBase: React.CSSProperties = { padding: '6px 10px' };
const tdMuted: React.CSSProperties = { ...tdBase, color: 'var(--fg-2)' };
const thRow: React.CSSProperties = { color: 'var(--fg-3)', textAlign: 'left' };

// Renders a row's name as a button that opens the describe modal.
// Inline so each subtab table can pass its own kind without
// threading describe handlers through generic wrappers.
function NameLink({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        background: 'transparent',
        border: 0,
        padding: 0,
        margin: 0,
        font: 'inherit',
        color: 'var(--fg)',
        cursor: 'pointer',
        textAlign: 'left',
        textDecoration: 'underline dotted var(--fg-3)',
        textUnderlineOffset: 2,
      }}
      title="describe"
    >
      {label}
    </button>
  );
}

// ──────────────────────────────────────────────────────────────
// Describe modal — fetches yaml on mount, renders in monospace
// ──────────────────────────────────────────────────────────────

function DescribeModal({
  agentId,
  target,
  onClose,
}: {
  agentId: string;
  target: DescribeTarget;
  onClose: () => void;
}) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [yaml, setYaml] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setYaml(null);
    setError(null);
    sendToAgent(agentId, {
      type: 'K8sDescribeRequest',
      payload: { kind: target.kind, namespace: target.namespace, name: target.name },
    });

    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type !== 'K8sDescribeResponse') return;
      // Match on identity in case multiple describe round-trips
      // overlap. We only care about the one we asked for.
      if (
        msg.payload.kind !== target.kind ||
        msg.payload.namespace !== target.namespace ||
        msg.payload.name !== target.name
      )
        return;
      if (msg.payload.error) {
        setError(msg.payload.error);
      } else {
        setYaml(msg.payload.yaml);
      }
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);

    return () => {
      unsub();
      window.removeEventListener('keydown', onKey);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, target.kind, target.namespace, target.name]);

  const copyYaml = async () => {
    if (!yaml) return;
    try {
      await navigator.clipboard.writeText(yaml);
    } catch {
      /* clipboard API may be unavailable; ignore */
    }
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0, 0, 0, 0.55)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 90,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(960px, 90vw)',
          maxHeight: '85vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--bg)',
          border: '1px solid var(--line)',
          borderRadius: 4,
          fontFamily: 'var(--mono)',
          fontSize: 12,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            padding: '8px 12px',
            borderBottom: '1px solid var(--line)',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            background: 'var(--bg-1)',
          }}
        >
          <span style={{ color: 'var(--fg-3)' }}>describe</span>
          <span style={{ color: 'var(--fg-2)' }}>{target.kind}</span>
          <span style={{ color: 'var(--fg-3)' }}>/</span>
          {target.namespace && (
            <>
              <span style={{ color: 'var(--fg-2)' }}>{target.namespace}</span>
              <span style={{ color: 'var(--fg-3)' }}>/</span>
            </>
          )}
          <span>{target.name}</span>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            className="btn sm"
            onClick={copyYaml}
            disabled={!yaml}
            title="copy YAML"
            style={{ height: 22, fontSize: 11, padding: '0 8px' }}
          >
            copy
          </button>
          <button
            type="button"
            className="btn sm"
            onClick={onClose}
            title="close (Esc)"
            style={{ height: 22, fontSize: 11, padding: '0 8px' }}
          >
            ×
          </button>
        </div>
        <div
          style={{
            flex: 1,
            overflow: 'auto',
            padding: 12,
            background: '#06090b',
          }}
        >
          {error ? (
            <pre style={{ margin: 0, color: 'var(--err, #e57373)' }}>{error}</pre>
          ) : yaml === null ? (
            <span style={{ color: 'var(--fg-3)' }}>loading…</span>
          ) : (
            <pre style={{ margin: 0, color: 'var(--fg)', whiteSpace: 'pre' }}>
              {yaml}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}

function PanelHead({
  title,
  meta,
}: {
  title: string;
  meta?: string;
}) {
  return (
    <div className="panel-head" style={{ flexShrink: 0 }}>
      <div className="panel-title">
        <span className="ico">⎈</span> {title}
        {meta && <span className="meta">{meta}</span>}
      </div>
    </div>
  );
}

function ErrorPane({ error }: { error: string }) {
  return (
    <div
      style={{
        padding: 16,
        fontFamily: 'var(--mono)',
        fontSize: 12,
        color: 'var(--err, #e57373)',
        whiteSpace: 'pre-wrap',
      }}
    >
      {error}
    </div>
  );
}

function EmptyPane({ label }: { label: string }) {
  return (
    <div
      style={{
        padding: 16,
        fontFamily: 'var(--mono)',
        fontSize: 12,
        color: 'var(--fg-3)',
      }}
    >
      {label}
    </div>
  );
}

const tableStyle: React.CSSProperties = {
  width: '100%',
  fontFamily: 'var(--mono)',
  fontSize: 12,
  borderCollapse: 'collapse',
};

const rowStyle: React.CSSProperties = { borderTop: '1px solid var(--line)' };

// ──────────────────────────────────────────────────────────────
// Logs modal — live tail with follow + container picker
// ──────────────────────────────────────────────────────────────

const MAX_LOG_LINES = 5000;

function newStreamId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  return `s-${Math.random().toString(36).slice(2)}-${Date.now()}`;
}

function LogsModal({
  agentId,
  target,
  onClose,
}: {
  agentId: string;
  target: LogsTarget;
  onClose: () => void;
}) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [container, setContainer] = useState<string>(target.containers[0] ?? '');
  const [follow, setFollow] = useState(true);
  const [streamId, setStreamId] = useState<string>(newStreamId);
  const [lines, setLines] = useState<string[]>([]);
  const [ended, setEnded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // (Re)open the stream whenever the container or follow toggle changes.
  useEffect(() => {
    const sid = newStreamId();
    setStreamId(sid);
    setLines([]);
    setEnded(false);
    setError(null);

    sendToAgent(agentId, {
      type: 'K8sLogsRequest',
      payload: {
        stream_id: sid,
        namespace: target.namespace,
        pod_name: target.podName,
        container: container || null,
        tail_lines: 200,
        follow,
      },
    });

    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'K8sLogsChunk' && msg.payload.stream_id === sid) {
        setLines((prev) => {
          const next = prev.concat(msg.payload.lines);
          // Clamp so a chatty container doesn't melt the DOM.
          return next.length > MAX_LOG_LINES
            ? next.slice(next.length - MAX_LOG_LINES)
            : next;
        });
      } else if (msg.type === 'K8sLogsEnd' && msg.payload.stream_id === sid) {
        setEnded(true);
        setError(msg.payload.error);
      }
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);

    return () => {
      sendToAgent(agentId, { type: 'K8sLogsStop', payload: { stream_id: sid } });
      unsub();
      window.removeEventListener('keydown', onKey);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, target.namespace, target.podName, container, follow]);

  // Auto-scroll to bottom when new lines arrive (only while in follow mode).
  const scrollerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!follow) return;
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines, follow]);

  const stopNow = () => {
    sendToAgent(agentId, { type: 'K8sLogsStop', payload: { stream_id: streamId } });
    setEnded(true);
  };

  const copyAll = async () => {
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
    } catch {
      /* clipboard API may be unavailable */
    }
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0, 0, 0, 0.55)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 90,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(1100px, 92vw)',
          height: 'min(720px, 85vh)',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--bg)',
          border: '1px solid var(--line)',
          borderRadius: 4,
          fontFamily: 'var(--mono)',
          fontSize: 12,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            padding: '8px 12px',
            borderBottom: '1px solid var(--line)',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            background: 'var(--bg-1)',
            flexShrink: 0,
          }}
        >
          <span style={{ color: 'var(--fg-3)' }}>logs</span>
          <span style={{ color: 'var(--fg-2)' }}>{target.namespace}</span>
          <span style={{ color: 'var(--fg-3)' }}>/</span>
          <span>{target.podName}</span>

          {target.containers.length > 1 && (
            <select
              value={container}
              onChange={(e) => setContainer(e.target.value)}
              style={{
                marginLeft: 8,
                background: 'var(--bg)',
                color: 'var(--fg)',
                border: '1px solid var(--line)',
                borderRadius: 3,
                fontFamily: 'var(--mono)',
                fontSize: 11,
                padding: '2px 6px',
              }}
            >
              {target.containers.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          )}

          <label
            style={{
              marginLeft: 8,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              color: 'var(--fg-2)',
              cursor: 'pointer',
              fontSize: 11,
            }}
          >
            <input
              type="checkbox"
              checked={follow}
              onChange={(e) => setFollow(e.target.checked)}
            />
            follow
          </label>

          <span
            style={{
              color: ended ? 'var(--fg-3)' : 'var(--ok, #7fb069)',
              fontSize: 11,
            }}
          >
            {ended ? (error ? `ended: ${error}` : 'ended') : `${lines.length} line${lines.length === 1 ? '' : 's'}`}
          </span>

          <div style={{ flex: 1 }} />

          <button
            type="button"
            className="btn sm"
            onClick={copyAll}
            disabled={lines.length === 0}
            title="copy buffer"
            style={{ height: 22, fontSize: 11, padding: '0 8px' }}
          >
            copy
          </button>
          <button
            type="button"
            className="btn sm"
            onClick={stopNow}
            disabled={ended}
            title="stop streaming"
            style={{ height: 22, fontSize: 11, padding: '0 8px' }}
          >
            stop
          </button>
          <button
            type="button"
            className="btn sm"
            onClick={onClose}
            title="close (Esc)"
            style={{ height: 22, fontSize: 11, padding: '0 8px' }}
          >
            ×
          </button>
        </div>

        <div
          ref={scrollerRef}
          style={{
            flex: 1,
            overflow: 'auto',
            padding: 12,
            background: '#06090b',
          }}
        >
          {lines.length === 0 ? (
            <span style={{ color: 'var(--fg-3)' }}>
              {ended ? 'no log lines' : 'waiting for output…'}
            </span>
          ) : (
            <pre style={{ margin: 0, color: 'var(--fg)', whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
              {lines.join('\n')}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────────────────────
// Exec modal — kube exec PTY in xterm.js
// ──────────────────────────────────────────────────────────────

function ExecModal({
  agentId,
  target,
  onClose,
}: {
  agentId: string;
  target: ExecTarget;
  onClose: () => void;
}) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const canWrite = useCanWrite();
  const [container, setContainer] = useState<string>(target.containers[0] ?? '');
  const sessionIdRef = useRef<string>(newStreamId());
  const termHostRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [status, setStatus] = useState<'opening' | 'open' | 'closed'>('opening');
  const [error, setError] = useState<string | null>(null);

  // Build the xterm + open the kube exec session. Re-opens whenever
  // the container picker changes — the prior session_id is stopped
  // via the unmount cleanup of the previous effect run.
  useEffect(() => {
    if (!canWrite) return;
    if (!termHostRef.current) return;

    const sid = newStreamId();
    sessionIdRef.current = sid;
    setStatus('opening');
    setError(null);

    const term = new XTerm({
      cursorBlink: true,
      fontFamily: "JetBrains Mono, ui-monospace, 'SF Mono', Menlo, monospace",
      fontSize: 12,
      scrollback: 5000,
      theme: {
        background: '#06090b',
        foreground: '#c8d3dc',
        cursor: '#7fb069',
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(termHostRef.current);
    fit.fit();
    xtermRef.current = term;
    fitRef.current = fit;

    term.onData((data) => {
      const bytes = Array.from(new TextEncoder().encode(data));
      sendToAgent(agentId, {
        type: 'TerminalData',
        payload: { session_id: sid, data: bytes },
      });
    });

    const sendSize = () => {
      try {
        fit.fit();
      } catch {
        /* size 0 during a transition */
      }
      sendToAgent(agentId, {
        type: 'TerminalResize',
        payload: { session_id: sid, cols: term.cols, rows: term.rows },
      });
    };
    const ro = new ResizeObserver(() => sendSize());
    ro.observe(termHostRef.current);
    window.addEventListener('resize', sendSize);

    sendToAgent(agentId, {
      type: 'K8sExecRequest',
      payload: {
        session_id: sid,
        namespace: target.namespace,
        pod_name: target.podName,
        container: container || null,
        command: ['/bin/sh'],
      },
    });

    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'K8sExecResponse' && msg.payload.session_id === sid) {
        if (msg.payload.success) {
          setStatus('open');
          setTimeout(sendSize, 80);
        } else {
          setStatus('closed');
          setError(msg.payload.error ?? 'exec failed');
        }
      } else if (msg.type === 'TerminalData' && msg.payload.session_id === sid) {
        xtermRef.current?.write(new Uint8Array(msg.payload.data));
      }
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && (e.target as HTMLElement)?.tagName !== 'INPUT') {
        // Esc only escapes when focus isn't in an input — otherwise xterm
        // would never see the Esc key. Close button still works.
      }
    };
    window.addEventListener('keydown', onKey);

    return () => {
      sendToAgent(agentId, {
        type: 'StopTerminalRequest',
        payload: { session_id: sid },
      });
      unsub();
      ro.disconnect();
      window.removeEventListener('resize', sendSize);
      window.removeEventListener('keydown', onKey);
      term.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, target.namespace, target.podName, container, canWrite]);

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0, 0, 0, 0.55)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 90,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(1100px, 92vw)',
          height: 'min(720px, 85vh)',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--bg)',
          border: '1px solid var(--line)',
          borderRadius: 4,
          fontFamily: 'var(--mono)',
          fontSize: 12,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            padding: '8px 12px',
            borderBottom: '1px solid var(--line)',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            background: 'var(--bg-1)',
            flexShrink: 0,
          }}
        >
          <span style={{ color: 'var(--fg-3)' }}>exec</span>
          <span style={{ color: 'var(--fg-2)' }}>{target.namespace}</span>
          <span style={{ color: 'var(--fg-3)' }}>/</span>
          <span>{target.podName}</span>

          {target.containers.length > 1 && (
            <select
              value={container}
              onChange={(e) => setContainer(e.target.value)}
              style={{
                marginLeft: 8,
                background: 'var(--bg)',
                color: 'var(--fg)',
                border: '1px solid var(--line)',
                borderRadius: 3,
                fontFamily: 'var(--mono)',
                fontSize: 11,
                padding: '2px 6px',
              }}
            >
              {target.containers.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          )}

          <span
            style={{
              color:
                status === 'open'
                  ? 'var(--ok, #7fb069)'
                  : status === 'closed'
                    ? 'var(--err, #e57373)'
                    : 'var(--warn, #e6b450)',
              fontSize: 11,
            }}
          >
            {status === 'open'
              ? 'connected'
              : status === 'closed'
                ? error
                  ? `closed: ${error}`
                  : 'closed'
                : 'opening…'}
          </span>

          <div style={{ flex: 1 }} />

          <button
            type="button"
            className="btn sm"
            onClick={onClose}
            title="close"
            style={{ height: 22, fontSize: 11, padding: '0 8px' }}
          >
            ×
          </button>
        </div>

        {!canWrite ? (
          <div
            style={{
              flex: 1,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--warn)',
              padding: 24,
              textAlign: 'center',
            }}
          >
            viewer role: interactive shells are admin-only.
          </div>
        ) : (
          <div ref={termHostRef} style={{ flex: 1, overflow: 'hidden', padding: 8 }} />
        )}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────────────────────
// Subtab views
// ──────────────────────────────────────────────────────────────

type ViewProps = { agentId: string; onDescribe: (t: DescribeTarget) => void };
type PodViewProps = ViewProps & {
  onLogs: (t: LogsTarget) => void;
  onExec: (t: ExecTarget) => void;
};

function PodsView({ agentId, onDescribe, onLogs, onExec }: PodViewProps) {
  const { data, error, loading } = useK8sList<K8sPod[]>(
    agentId,
    { type: 'K8sListPodsRequest' },
    (msg) =>
      msg.type === 'K8sListPodsResponse'
        ? { data: msg.payload.pods, error: msg.payload.error }
        : null,
  );
  const pods = data ?? [];
  return (
    <>
      <PanelHead
        title="KUBERNETES · pods"
        meta={loading && !data ? 'loading…' : `${pods.length} pod${pods.length === 1 ? '' : 's'}`}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 'var(--pad, 12px)' }}>
        {error ? (
          <ErrorPane error={error} />
        ) : pods.length === 0 && !loading ? (
          <EmptyPane label="no pods on this cluster yet" />
        ) : (
          <table className="tbl" style={tableStyle}>
            <thead>
              <tr style={thRow}>
                <th style={tdBase}>namespace</th>
                <th style={tdBase}>name</th>
                <th style={tdBase}>ready</th>
                <th style={tdBase}>status</th>
                <th style={tdBase}>restarts</th>
                <th style={tdBase}>age</th>
                <th style={tdBase}>node</th>
                <th style={tdBase}></th>
              </tr>
            </thead>
            <tbody>
              {pods.map((p) => (
                <tr key={`${p.namespace}/${p.name}`} style={rowStyle}>
                  <td style={tdMuted}>{p.namespace}</td>
                  <td style={tdBase}>
                    <NameLink
                      label={p.name}
                      onClick={() =>
                        onDescribe({ kind: 'pod', namespace: p.namespace, name: p.name })
                      }
                    />
                  </td>
                  <td style={tdBase}>{p.ready}</td>
                  <td style={{ ...tdBase, ...phaseStyle(p.phase) }}>{p.phase}</td>
                  <td style={tdBase}>{p.restarts}</td>
                  <td style={tdBase}>{fmtAge(p.age_secs)}</td>
                  <td style={tdMuted}>{p.node ?? '—'}</td>
                  <td style={tdBase}>
                    <div style={{ display: 'inline-flex', gap: 4 }}>
                      <button
                        type="button"
                        onClick={() =>
                          onLogs({
                            namespace: p.namespace,
                            podName: p.name,
                            containers: p.containers,
                          })
                        }
                        style={{
                          background: 'transparent',
                          border: '1px solid var(--line)',
                          borderRadius: 3,
                          color: 'var(--fg-2)',
                          cursor: 'pointer',
                          fontFamily: 'var(--mono)',
                          fontSize: 10,
                          padding: '2px 6px',
                        }}
                        title="tail logs"
                      >
                        logs
                      </button>
                      <button
                        type="button"
                        onClick={() =>
                          onExec({
                            namespace: p.namespace,
                            podName: p.name,
                            containers: p.containers,
                          })
                        }
                        style={{
                          background: 'transparent',
                          border: '1px solid var(--line)',
                          borderRadius: 3,
                          color: 'var(--fg-2)',
                          cursor: 'pointer',
                          fontFamily: 'var(--mono)',
                          fontSize: 10,
                          padding: '2px 6px',
                        }}
                        title="open shell (kubectl exec)"
                      >
                        exec
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

function DeploymentsView({ agentId, onDescribe }: ViewProps) {
  const { data, error, loading } = useK8sList<K8sDeployment[]>(
    agentId,
    { type: 'K8sListDeploymentsRequest' },
    (msg) =>
      msg.type === 'K8sListDeploymentsResponse'
        ? { data: msg.payload.deployments, error: msg.payload.error }
        : null,
  );
  const items = data ?? [];
  return (
    <>
      <PanelHead
        title="KUBERNETES · deployments"
        meta={loading && !data ? 'loading…' : `${items.length} deployment${items.length === 1 ? '' : 's'}`}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 'var(--pad, 12px)' }}>
        {error ? (
          <ErrorPane error={error} />
        ) : items.length === 0 && !loading ? (
          <EmptyPane label="no deployments" />
        ) : (
          <table className="tbl" style={tableStyle}>
            <thead>
              <tr style={thRow}>
                <th style={tdBase}>namespace</th>
                <th style={tdBase}>name</th>
                <th style={tdBase}>ready</th>
                <th style={tdBase}>up-to-date</th>
                <th style={tdBase}>available</th>
                <th style={tdBase}>age</th>
                <th style={tdBase}>image</th>
              </tr>
            </thead>
            <tbody>
              {items.map((d) => (
                <tr key={`${d.namespace}/${d.name}`} style={rowStyle}>
                  <td style={tdMuted}>{d.namespace}</td>
                  <td style={tdBase}>
                    <NameLink
                      label={d.name}
                      onClick={() =>
                        onDescribe({ kind: 'deployment', namespace: d.namespace, name: d.name })
                      }
                    />
                  </td>
                  <td style={tdBase}>{d.ready}</td>
                  <td style={tdBase}>{d.up_to_date}</td>
                  <td style={tdBase}>{d.available}</td>
                  <td style={tdBase}>{fmtAge(d.age_secs)}</td>
                  <td style={tdMuted}>{d.image ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

function ServicesView({ agentId, onDescribe }: ViewProps) {
  const { data, error, loading } = useK8sList<K8sService[]>(
    agentId,
    { type: 'K8sListServicesRequest' },
    (msg) =>
      msg.type === 'K8sListServicesResponse'
        ? { data: msg.payload.services, error: msg.payload.error }
        : null,
  );
  const items = data ?? [];
  return (
    <>
      <PanelHead
        title="KUBERNETES · services"
        meta={loading && !data ? 'loading…' : `${items.length} service${items.length === 1 ? '' : 's'}`}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 'var(--pad, 12px)' }}>
        {error ? (
          <ErrorPane error={error} />
        ) : items.length === 0 && !loading ? (
          <EmptyPane label="no services" />
        ) : (
          <table className="tbl" style={tableStyle}>
            <thead>
              <tr style={thRow}>
                <th style={tdBase}>namespace</th>
                <th style={tdBase}>name</th>
                <th style={tdBase}>type</th>
                <th style={tdBase}>cluster-ip</th>
                <th style={tdBase}>external-ip</th>
                <th style={tdBase}>ports</th>
                <th style={tdBase}>age</th>
              </tr>
            </thead>
            <tbody>
              {items.map((s) => (
                <tr key={`${s.namespace}/${s.name}`} style={rowStyle}>
                  <td style={tdMuted}>{s.namespace}</td>
                  <td style={tdBase}>
                    <NameLink
                      label={s.name}
                      onClick={() =>
                        onDescribe({ kind: 'service', namespace: s.namespace, name: s.name })
                      }
                    />
                  </td>
                  <td style={tdBase}>{s.kind}</td>
                  <td style={tdMuted}>{s.cluster_ip ?? '—'}</td>
                  <td style={tdMuted}>
                    {s.external_ips.length > 0 ? s.external_ips.join(', ') : '—'}
                  </td>
                  <td style={tdMuted}>
                    {s.ports.length > 0 ? s.ports.join(', ') : '—'}
                  </td>
                  <td style={tdBase}>{fmtAge(s.age_secs)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

function IngressesView({ agentId, onDescribe }: ViewProps) {
  const { data, error, loading } = useK8sList<K8sIngress[]>(
    agentId,
    { type: 'K8sListIngressesRequest' },
    (msg) =>
      msg.type === 'K8sListIngressesResponse'
        ? { data: msg.payload.ingresses, error: msg.payload.error }
        : null,
  );
  const items = data ?? [];
  return (
    <>
      <PanelHead
        title="KUBERNETES · ingresses"
        meta={loading && !data ? 'loading…' : `${items.length} ingress${items.length === 1 ? '' : 'es'}`}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 'var(--pad, 12px)' }}>
        {error ? (
          <ErrorPane error={error} />
        ) : items.length === 0 && !loading ? (
          <EmptyPane label="no ingresses" />
        ) : (
          <table className="tbl" style={tableStyle}>
            <thead>
              <tr style={thRow}>
                <th style={tdBase}>namespace</th>
                <th style={tdBase}>name</th>
                <th style={tdBase}>class</th>
                <th style={tdBase}>hosts</th>
                <th style={tdBase}>address</th>
                <th style={tdBase}>age</th>
              </tr>
            </thead>
            <tbody>
              {items.map((i) => (
                <tr key={`${i.namespace}/${i.name}`} style={rowStyle}>
                  <td style={tdMuted}>{i.namespace}</td>
                  <td style={tdBase}>
                    <NameLink
                      label={i.name}
                      onClick={() =>
                        onDescribe({ kind: 'ingress', namespace: i.namespace, name: i.name })
                      }
                    />
                  </td>
                  <td style={tdMuted}>{i.class ?? '—'}</td>
                  <td style={tdMuted}>
                    {i.hosts.length > 0 ? i.hosts.join(', ') : '—'}
                  </td>
                  <td style={tdMuted}>
                    {i.addresses.length > 0 ? i.addresses.join(', ') : '—'}
                  </td>
                  <td style={tdBase}>{fmtAge(i.age_secs)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

function PvcsView({ agentId, onDescribe }: ViewProps) {
  const { data, error, loading } = useK8sList<K8sPvc[]>(
    agentId,
    { type: 'K8sListPvcsRequest' },
    (msg) =>
      msg.type === 'K8sListPvcsResponse'
        ? { data: msg.payload.pvcs, error: msg.payload.error }
        : null,
  );
  const items = data ?? [];
  return (
    <>
      <PanelHead
        title="KUBERNETES · pvcs"
        meta={loading && !data ? 'loading…' : `${items.length} pvc${items.length === 1 ? '' : 's'}`}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 'var(--pad, 12px)' }}>
        {error ? (
          <ErrorPane error={error} />
        ) : items.length === 0 && !loading ? (
          <EmptyPane label="no persistent volume claims" />
        ) : (
          <table className="tbl" style={tableStyle}>
            <thead>
              <tr style={thRow}>
                <th style={tdBase}>namespace</th>
                <th style={tdBase}>name</th>
                <th style={tdBase}>status</th>
                <th style={tdBase}>volume</th>
                <th style={tdBase}>capacity</th>
                <th style={tdBase}>access</th>
                <th style={tdBase}>storageclass</th>
                <th style={tdBase}>age</th>
              </tr>
            </thead>
            <tbody>
              {items.map((p) => (
                <tr key={`${p.namespace}/${p.name}`} style={rowStyle}>
                  <td style={tdMuted}>{p.namespace}</td>
                  <td style={tdBase}>
                    <NameLink
                      label={p.name}
                      onClick={() =>
                        onDescribe({ kind: 'pvc', namespace: p.namespace, name: p.name })
                      }
                    />
                  </td>
                  <td style={{ ...tdBase, ...phaseStyle(p.status) }}>{p.status}</td>
                  <td style={tdMuted}>{p.volume_name ?? '—'}</td>
                  <td style={tdBase}>{p.capacity ?? '—'}</td>
                  <td style={tdMuted}>
                    {p.access_modes.length > 0 ? p.access_modes.join(',') : '—'}
                  </td>
                  <td style={tdMuted}>{p.storage_class ?? '—'}</td>
                  <td style={tdBase}>{fmtAge(p.age_secs)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

// Events don't have a `name` field of their own — operators usually
// want to describe the involved object, not the event row itself.
// We keep events read-only for now; describe affordance lands when
// we fold a kind/name parser onto event.object.
function EventsView({ agentId }: { agentId: string }) {
  const { data, error, loading } = useK8sList<K8sEvent[]>(
    agentId,
    { type: 'K8sListEventsRequest' },
    (msg) =>
      msg.type === 'K8sListEventsResponse'
        ? { data: msg.payload.events, error: msg.payload.error }
        : null,
  );
  const items = data ?? [];
  return (
    <>
      <PanelHead
        title="KUBERNETES · events"
        meta={loading && !data ? 'loading…' : `${items.length} event${items.length === 1 ? '' : 's'} (capped @200)`}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 'var(--pad, 12px)' }}>
        {error ? (
          <ErrorPane error={error} />
        ) : items.length === 0 && !loading ? (
          <EmptyPane label="no recent events" />
        ) : (
          <table className="tbl" style={tableStyle}>
            <thead>
              <tr style={thRow}>
                <th style={tdBase}>last seen</th>
                <th style={tdBase}>type</th>
                <th style={tdBase}>reason</th>
                <th style={tdBase}>object</th>
                <th style={tdBase}>×</th>
                <th style={tdBase}>message</th>
              </tr>
            </thead>
            <tbody>
              {items.map((e, idx) => (
                <tr key={idx} style={rowStyle}>
                  <td style={tdMuted}>{fmtAge(e.age_secs)} ago</td>
                  <td style={{ ...tdBase, ...phaseStyle(e.kind) }}>{e.kind}</td>
                  <td style={tdBase}>{e.reason}</td>
                  <td style={tdMuted}>
                    {e.namespace ? `${e.namespace}/` : ''}
                    {e.object}
                  </td>
                  <td style={tdBase}>{e.count}</td>
                  <td style={tdMuted}>{e.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

// ──────────────────────────────────────────────────────────────
// Shell — inner sidebar + content router (mirrors DockerHub)
// ──────────────────────────────────────────────────────────────

type Props = {
  agentId: string;
  subtab: K8sSubtab;
  onSubtabChange: (s: K8sSubtab) => void;
};

export default function KubernetesHub({ agentId, subtab, onSubtabChange }: Props) {
  const [describeTarget, setDescribeTarget] = useState<DescribeTarget | null>(null);
  const [logsTarget, setLogsTarget] = useState<LogsTarget | null>(null);
  const [execTarget, setExecTarget] = useState<ExecTarget | null>(null);
  return (
    <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
      <nav
        aria-label="kubernetes subnav"
        style={{
          width: 168,
          flexShrink: 0,
          borderRight: '1px solid var(--line)',
          background: 'var(--bg-1)',
          padding: '8px 0',
          overflowY: 'auto',
          display: 'flex',
          flexDirection: 'column',
          gap: 2,
        }}
      >
        <div
          style={{
            padding: '4px 12px 8px',
            color: 'var(--fg-3)',
            fontFamily: 'var(--mono)',
            fontSize: 10,
            letterSpacing: '0.08em',
            textTransform: 'uppercase',
          }}
        >
          kubernetes
        </div>
        {SUBTAB_DEFS.map((s, i) => {
          const active = s.id === subtab;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => onSubtabChange(s.id)}
              style={{
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'flex-start',
                gap: 2,
                width: '100%',
                padding: '6px 12px',
                border: 0,
                background: active ? 'var(--bg-2)' : 'transparent',
                borderLeft: `2px solid ${active ? 'var(--accent)' : 'transparent'}`,
                cursor: 'pointer',
                fontFamily: 'var(--mono)',
                color: active ? 'var(--fg)' : 'var(--fg-2)',
                textAlign: 'left',
              }}
            >
              <span style={{ display: 'inline-flex', gap: 8, alignItems: 'baseline', fontSize: 12 }}>
                <span style={{ color: 'var(--fg-3)', fontSize: 10 }}>
                  {String(i + 1).padStart(2, '0')}
                </span>
                <span>{s.label}</span>
              </span>
              {s.hint && (
                <span style={{ fontSize: 10, color: 'var(--fg-3)', paddingLeft: 22 }}>
                  {s.hint}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
        {subtab === 'pods'        && <PodsView        agentId={agentId} onDescribe={setDescribeTarget} onLogs={setLogsTarget} onExec={setExecTarget} />}
        {subtab === 'deployments' && <DeploymentsView agentId={agentId} onDescribe={setDescribeTarget} />}
        {subtab === 'services'    && <ServicesView    agentId={agentId} onDescribe={setDescribeTarget} />}
        {subtab === 'ingresses'   && <IngressesView   agentId={agentId} onDescribe={setDescribeTarget} />}
        {subtab === 'pvcs'        && <PvcsView        agentId={agentId} onDescribe={setDescribeTarget} />}
        {subtab === 'events'      && <EventsView      agentId={agentId} />}
      </div>

      {describeTarget && (
        <DescribeModal
          agentId={agentId}
          target={describeTarget}
          onClose={() => setDescribeTarget(null)}
        />
      )}
      {logsTarget && (
        <LogsModal
          agentId={agentId}
          target={logsTarget}
          onClose={() => setLogsTarget(null)}
        />
      )}
      {execTarget && (
        <ExecModal
          agentId={agentId}
          target={execTarget}
          onClose={() => setExecTarget(null)}
        />
      )}
    </div>
  );
}
