'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { XIcon, Loader2Icon, AlertCircleIcon, PauseIcon, PlayIcon } from 'lucide-react';

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
        setLines((prev) => {
          const next = prev.length >= MAX_LINES
            ? [...prev.slice(prev.length - MAX_LINES + 1), msg.payload.data]
            : [...prev, msg.payload.data];
          return next;
        });
      } else if (msg.type === 'DockerLogsEnd' && msg.payload.container_id === containerId) {
        setStreaming(false);
        if (msg.payload.error) {
          setEndError(msg.payload.error);
        }
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
    <div
      className="fixed inset-0 z-40 bg-slate-950/80 backdrop-blur-sm flex items-center justify-center p-6"
      onClick={onClose}
    >
      <div
        className="bg-slate-900 border border-slate-800 rounded-lg shadow-2xl w-full max-w-5xl h-[80vh] flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-slate-800 bg-slate-900">
          <div className="min-w-0">
            <div className="text-sm font-semibold text-slate-100 truncate" title={containerName}>
              {containerName}
            </div>
            <div className="text-[11px] text-slate-500 flex items-center gap-3 mt-0.5">
              <code>{containerId.slice(0, 12)}</code>
              {streaming ? (
                <span className="inline-flex items-center gap-1 text-emerald-300">
                  <Loader2Icon className="w-3 h-3 animate-spin" /> streaming
                </span>
              ) : endError ? (
                <span className="inline-flex items-center gap-1 text-red-300">
                  <AlertCircleIcon className="w-3 h-3" /> {endError}
                </span>
              ) : (
                <span className="text-slate-500">stream ended</span>
              )}
              <span>
                {lines.length} line{lines.length === 1 ? '' : 's'}
                {lines.length >= MAX_LINES && ' (capped)'}
              </span>
            </div>
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => setAutoscroll((v) => !v)}
              title={autoscroll ? 'Pause autoscroll' : 'Resume autoscroll'}
              className="p-1.5 rounded-md text-slate-400 hover:text-slate-100 hover:bg-slate-800 transition-colors"
            >
              {autoscroll ? <PauseIcon className="w-4 h-4" /> : <PlayIcon className="w-4 h-4" />}
            </button>
            <button
              type="button"
              onClick={onClose}
              className="p-1.5 rounded-md text-slate-400 hover:text-slate-100 hover:bg-slate-800 transition-colors"
              title="Close"
            >
              <XIcon className="w-4 h-4" />
            </button>
          </div>
        </div>
        <div
          ref={scrollRef}
          className="flex-1 overflow-y-auto bg-slate-950 px-3 py-2 font-mono text-[12px] leading-relaxed text-slate-300"
          onWheel={(e) => {
            // Pause autoscroll if the user scrolls up.
            const el = e.currentTarget;
            const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 4;
            setAutoscroll(atBottom);
          }}
        >
          {lines.length === 0 ? (
            <div className="text-slate-600 italic">Waiting for output…</div>
          ) : (
            lines.map((l, i) => (
              <div key={i} className="whitespace-pre-wrap break-all">
                {l}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
