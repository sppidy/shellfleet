'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { Loader2Icon } from 'lucide-react';

const MAX_LINES = 5_000;

export default function LogViewer({
  agentId,
  containerId,
  containerName,
  onClose,
}: {
  agentId: string;
  containerId: string;
  containerName: string;
  onClose: () => void;
}) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [lines, setLines] = useState<string[]>([]);
  const [streaming, setStreaming] = useState(true);
  const [endError, setEndError] = useState<string | null>(null);
  const [autoscroll, setAutoscroll] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'DockerLogsChunk' && msg.payload.container_id === containerId) {
        setLines((prev) =>
          prev.length >= MAX_LINES
            ? [...prev.slice(prev.length - MAX_LINES + 1), msg.payload.data]
            : [...prev, msg.payload.data],
        );
      } else if (
        msg.type === 'DockerLogsEnd' &&
        msg.payload.container_id === containerId
      ) {
        setStreaming(false);
        if (msg.payload.error) setEndError(msg.payload.error);
      }
    });

    sendToAgent(agentId, {
      type: 'DockerLogsRequest',
      payload: { container_id: containerId, tail: 200, follow: true },
    });

    return () => {
      sendToAgent(agentId, {
        type: 'DockerLogsStop',
        payload: { container_id: containerId },
      });
      unsub();
    };
  }, [agentId, containerId, sendToAgent, onAgentMessage]);

  useEffect(() => {
    if (!autoscroll || !scrollRef.current) return;
    scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [lines, autoscroll]);

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div
        className="modal"
        style={{
          width: 'min(1100px, 95vw)',
          height: '85vh',
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">≡</span> docker logs -f
            <span className="meta">
              {containerName} · {containerId.slice(0, 12)} ·{' '}
              {streaming ? (
                <span className="ok">streaming</span>
              ) : endError ? (
                <span className="err-c">{endError}</span>
              ) : (
                'stream ended'
              )}
              {' · '}
              {lines.length} line{lines.length === 1 ? '' : 's'}
              {lines.length >= MAX_LINES && ' (capped)'}
            </span>
          </div>
          <div className="panel-actions">
            <button
              className="btn sm"
              onClick={() => setAutoscroll((v) => !v)}
              title={autoscroll ? 'Pause autoscroll' : 'Resume autoscroll'}
            >
              {autoscroll ? '❚❚' : '▶'}
            </button>
            <button className="icon-btn" onClick={onClose} title="Close">
              ×
            </button>
          </div>
        </div>
        <div
          ref={scrollRef}
          style={{
            flex: 1,
            overflow: 'auto',
            background: '#06090b',
            padding: '8px 14px',
            fontFamily: 'var(--mono)',
            fontSize: 12,
            lineHeight: 1.55,
            color: '#c8d3dc',
          }}
          onWheel={(e) => {
            const el = e.currentTarget;
            const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 4;
            setAutoscroll(atBottom);
          }}
        >
          {lines.length === 0 ? (
            <div style={{ color: 'var(--fg-3)', fontStyle: 'italic' }}>
              Waiting for output…
              <Loader2Icon className="w-3 h-3 inline ml-2 animate-spin" />
            </div>
          ) : (
            lines.map((l, i) => (
              <div key={i} style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
                {l}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
