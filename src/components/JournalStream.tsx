'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useCanWrite } from './providers/SessionProvider';

const PRIORITIES = ['', 'emerg', 'alert', 'crit', 'err', 'warning', 'notice', 'info', 'debug'];

const COMMON_UNITS = [
  'sshd',
  'nginx',
  'docker',
  'systemd',
  'cron',
  'sys-manager-agent',
];

const MAX_BUFFER_LINES = 10_000;

type StreamState = 'idle' | 'streaming';

interface ChunkMessage {
  type: 'JournalStreamChunk';
  payload: { stream_id: string; lines: string[] };
}
interface EndMessage {
  type: 'JournalStreamEnd';
  payload: { stream_id: string; error: string | null };
}
type AnyMsg = ChunkMessage | EndMessage | { type: string };

function newStreamId(): string {
  // Random enough to avoid collisions across operator tabs without
  // pulling in a uuid lib.
  return `js-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export default function JournalStream({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const canWrite = useCanWrite();

  const [streamState, setStreamState] = useState<StreamState>('idle');
  const [units, setUnits] = useState('');
  const [priority, setPriority] = useState('');
  const [since, setSince] = useState('');
  const [grep, setGrep] = useState('');
  const [identifier, setIdentifier] = useState('');
  const [lines, setLines] = useState(200);
  const [follow, setFollow] = useState(true);
  const [filter, setFilter] = useState('');
  const [autoScroll, setAutoScroll] = useState(true);
  const [paused, setPaused] = useState(false);
  const [wrap, setWrap] = useState(false);

  const [error, setError] = useState<string | null>(null);
  const [streamId, setStreamId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const pendingRef = useRef<string[]>([]);
  const outputRef = useRef<HTMLDivElement>(null);

  // WS subscriber: collect chunks for our active stream_id only.
  useEffect(() => {
    if (!streamId) return;
    const unsub = onAgentMessage(agentId, (msg) => {
      const m = msg as AnyMsg;
      if (m.type === 'JournalStreamChunk') {
        const c = m as ChunkMessage;
        if (c.payload.stream_id !== streamId) return;
        if (paused) {
          pendingRef.current.push(...c.payload.lines);
        } else {
          setLogs((prev) => {
            const next = prev.concat(c.payload.lines);
            return next.length > MAX_BUFFER_LINES
              ? next.slice(next.length - MAX_BUFFER_LINES)
              : next;
          });
        }
      } else if (m.type === 'JournalStreamEnd') {
        const e = m as EndMessage;
        if (e.payload.stream_id !== streamId) return;
        setStreamState('idle');
        if (e.payload.error) setError(e.payload.error);
      }
    });
    return unsub;
  }, [agentId, onAgentMessage, streamId, paused]);

  // Auto-scroll while streaming.
  useEffect(() => {
    if (!autoScroll || paused) return;
    const el = outputRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs, autoScroll, paused]);

  // Stop the stream when the operator navigates away.
  useEffect(() => {
    return () => {
      if (streamId) {
        sendToAgent(agentId, {
          type: 'JournalStreamStop',
          payload: { stream_id: streamId },
        });
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const start = useCallback(() => {
    setError(null);
    setLogs([]);
    pendingRef.current = [];
    const id = newStreamId();
    const unitList = units
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    sendToAgent(agentId, {
      type: 'JournalStreamRequest',
      payload: {
        stream_id: id,
        units: unitList,
        priority: priority || null,
        since: since.trim() || null,
        grep: grep.trim() || null,
        identifier: identifier.trim() || null,
        lines,
        follow,
      },
    });
    setStreamId(id);
    setStreamState('streaming');
  }, [agentId, sendToAgent, units, priority, since, grep, identifier, lines, follow]);

  const stop = useCallback(() => {
    if (streamId) {
      sendToAgent(agentId, {
        type: 'JournalStreamStop',
        payload: { stream_id: streamId },
      });
    }
    setStreamState('idle');
  }, [agentId, sendToAgent, streamId]);

  const togglePause = useCallback(() => {
    setPaused((p) => {
      if (p) {
        // Unpausing — flush the staged lines.
        setLogs((prev) => {
          const next = prev.concat(pendingRef.current);
          pendingRef.current = [];
          return next.length > MAX_BUFFER_LINES
            ? next.slice(next.length - MAX_BUFFER_LINES)
            : next;
        });
      }
      return !p;
    });
  }, []);

  const clear = useCallback(() => {
    setLogs([]);
    pendingRef.current = [];
  }, []);

  const filtered = useMemo(() => {
    if (!filter.trim()) return logs;
    const q = filter.toLowerCase();
    return logs.filter((l) => l.toLowerCase().includes(q));
  }, [logs, filter]);

  const colorFor = (line: string): string => {
    // crude severity colouring on the short-iso output.
    if (/\bemerg|alert|crit\b/i.test(line)) return 'var(--err)';
    if (/\berr(or)?\b/i.test(line)) return 'var(--err)';
    if (/\bwarn(ing)?\b/i.test(line)) return 'var(--warn)';
    return 'var(--fg-1)';
  };

  return (
    <div className="pane" style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      {/* Filter form */}
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">≡</span> JOURNAL STREAM
            <span
              className="meta"
              style={{
                color: streamState === 'streaming' ? 'var(--accent)' : 'var(--fg-2)',
              }}
            >
              {streamState === 'streaming' ? '● live' : '○ idle'}
              {logs.length > 0 ? ` · ${logs.length} lines buffered` : ''}
            </span>
          </div>
        </div>
        <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <div className="grid-3" style={{ gap: 10 }}>
            <div className="field">
              <label>units (comma-separated, optional)</label>
              <input
                className="input"
                type="text"
                value={units}
                onChange={(e) => setUnits(e.target.value)}
                placeholder={`e.g. ${COMMON_UNITS.slice(0, 3).join(', ')}`}
                disabled={streamState === 'streaming'}
                spellCheck={false}
              />
            </div>
            <div className="field">
              <label>priority</label>
              <select
                className="select"
                value={priority}
                onChange={(e) => setPriority(e.target.value)}
                disabled={streamState === 'streaming'}
              >
                {PRIORITIES.map((p) => (
                  <option key={p} value={p}>
                    {p || '— any —'}
                  </option>
                ))}
              </select>
            </div>
            <div className="field">
              <label>since</label>
              <input
                className="input"
                type="text"
                value={since}
                onChange={(e) => setSince(e.target.value)}
                placeholder="e.g. 1h ago, 2024-01-01 09:00"
                disabled={streamState === 'streaming'}
                spellCheck={false}
              />
            </div>
          </div>
          <div className="grid-3" style={{ gap: 10 }}>
            <div className="field">
              <label>grep (regex)</label>
              <input
                className="input"
                type="text"
                value={grep}
                onChange={(e) => setGrep(e.target.value)}
                placeholder="e.g. timeout|connection refused"
                disabled={streamState === 'streaming'}
                spellCheck={false}
              />
            </div>
            <div className="field">
              <label>identifier (SYSLOG_IDENTIFIER)</label>
              <input
                className="input"
                type="text"
                value={identifier}
                onChange={(e) => setIdentifier(e.target.value)}
                placeholder="e.g. cron"
                disabled={streamState === 'streaming'}
                spellCheck={false}
              />
            </div>
            <div className="field">
              <label>backlog lines</label>
              <input
                className="input"
                type="number"
                min={1}
                max={10000}
                value={lines}
                onChange={(e) => setLines(Math.max(1, Math.min(10000, Number(e.target.value) || 1)))}
                disabled={streamState === 'streaming'}
              />
            </div>
          </div>
          <div className="row" style={{ gap: 12, flexWrap: 'wrap', alignItems: 'center' }}>
            <label className="row" style={{ gap: 6, fontSize: 11.5, color: 'var(--fg-1)' }}>
              <input
                type="checkbox"
                checked={follow}
                onChange={(e) => setFollow(e.target.checked)}
                disabled={streamState === 'streaming'}
              />
              follow (--follow)
            </label>
            <div style={{ flex: 1 }} />
            {streamState === 'idle' ? (
              <button
                className="btn primary"
                onClick={start}
                disabled={!canWrite}
                title={!canWrite ? 'viewer role: read-only' : undefined}
              >
                ▶ start
              </button>
            ) : (
              <button className="btn" onClick={stop}>
                ■ stop
              </button>
            )}
          </div>
          {error && (
            <div className="mono" style={{ color: 'var(--err)', fontSize: 11 }}>
              {error}
            </div>
          )}
        </div>
      </div>

      {/* Output */}
      <div
        className="panel"
        style={{
          marginTop: 10,
          flex: 1,
          minHeight: 0,
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▤</span> OUTPUT
            <span className="meta">
              {filtered.length}/{logs.length} lines
              {paused ? ` · paused (+${pendingRef.current.length})` : ''}
            </span>
          </div>
          <div className="panel-actions">
            <div className="search-input" style={{ width: 200 }}>
              <span style={{ color: 'var(--accent)' }}>⌕</span>
              <input
                placeholder="filter…"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
              />
            </div>
            <button className="btn sm" onClick={togglePause} title="Pause / resume buffer">
              {paused ? '▶ resume' : '⏸ pause'}
            </button>
            <button className="btn sm" onClick={clear} title="Clear buffer">
              clear
            </button>
            <label
              className="row"
              style={{ gap: 6, fontSize: 11, color: 'var(--fg-2)' }}
            >
              <input
                type="checkbox"
                checked={autoScroll}
                onChange={(e) => setAutoScroll(e.target.checked)}
              />
              autoscroll
            </label>
            <label
              className="row"
              style={{ gap: 6, fontSize: 11, color: 'var(--fg-2)' }}
            >
              <input
                type="checkbox"
                checked={wrap}
                onChange={(e) => setWrap(e.target.checked)}
              />
              wrap
            </label>
          </div>
        </div>
        <div
          ref={outputRef}
          style={{
            flex: 1,
            minHeight: 0,
            overflow: 'auto',
            padding: '8px 12px',
            background: 'var(--bg-1)',
            fontFamily: 'var(--mono)',
            fontSize: 11.5,
            lineHeight: 1.5,
            whiteSpace: wrap ? 'pre-wrap' : 'pre',
            wordBreak: wrap ? 'break-word' : 'normal',
          }}
        >
          {filtered.length === 0 ? (
            <div className="muted" style={{ padding: '20px 0', textAlign: 'center' }}>
              {streamState === 'streaming'
                ? 'waiting for log lines…'
                : 'configure filters above and click ▶ start'}
            </div>
          ) : (
            filtered.map((line, i) => (
              <div key={i} style={{ color: colorFor(line) }}>
                {line || ' '}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
