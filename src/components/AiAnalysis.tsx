'use client';

import { useState } from 'react';
import { apiFetch } from '@/lib/api';
import { useWebSocket } from './providers/WebSocketProvider';
import { Loader2Icon } from 'lucide-react';

interface Props {
  agentId: string;
}

// One-shot log gather has to finish even if an old agent ignores the request
// (the protocol says pre-v7 agents silently drop JournalStreamRequest), so
// every fetch resolves on its End message OR this timeout, whichever is first.
const GATHER_TIMEOUT_MS = 8000;

export default function AiAnalysis({ agentId }: Props) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [source, setSource] = useState('journal');
  const [container, setContainer] = useState('');
  const [prompt, setPrompt] = useState('');
  const [response, setResponse] = useState('');
  const [loading, setLoading] = useState(false);
  const [stage, setStage] = useState('');
  const [error, setError] = useState<string | null>(null);

  // Pull recent journal lines over the WS (free-form journalctl, no follow).
  const fetchJournal = () =>
    new Promise<string>((resolve) => {
      const streamId = 'ai-' + Math.random().toString(36).slice(2);
      const lines: string[] = [];
      let done = false;
      let unsub = () => {};
      const finish = () => {
        if (done) return;
        done = true;
        unsub();
        sendToAgent(agentId, { type: 'JournalStreamStop', payload: { stream_id: streamId } });
        resolve(lines.join('\n'));
      };
      unsub = onAgentMessage(agentId, (msg) => {
        if (msg.type === 'JournalStreamChunk' && msg.payload.stream_id === streamId) {
          lines.push(...msg.payload.lines);
        } else if (msg.type === 'JournalStreamEnd' && msg.payload.stream_id === streamId) {
          finish();
        }
      });
      sendToAgent(agentId, {
        type: 'JournalStreamRequest',
        payload: { stream_id: streamId, units: [], since: '1h ago', lines: 400, follow: false },
      });
      setTimeout(finish, GATHER_TIMEOUT_MS);
    });

  // Pull a container's recent logs over the WS (no follow).
  const fetchDocker = () =>
    new Promise<string>((resolve) => {
      const cid = container.trim();
      if (!cid) {
        resolve('');
        return;
      }
      const parts: string[] = [];
      let done = false;
      let unsub = () => {};
      const finish = () => {
        if (done) return;
        done = true;
        unsub();
        sendToAgent(agentId, { type: 'DockerLogsStop', payload: { container_id: cid } });
        resolve(parts.join(''));
      };
      unsub = onAgentMessage(agentId, (msg) => {
        if (msg.type === 'DockerLogsChunk' && msg.payload.container_id === cid) {
          parts.push(msg.payload.data);
        } else if (msg.type === 'DockerLogsEnd' && msg.payload.container_id === cid) {
          finish();
        }
      });
      sendToAgent(agentId, {
        type: 'DockerLogsRequest',
        payload: { container_id: cid, tail: 400, follow: false },
      });
      setTimeout(finish, GATHER_TIMEOUT_MS);
    });

  // Audit is a plain REST read (no agent round-trip).
  const fetchAudit = async (): Promise<string> => {
    try {
      const res = await apiFetch('/api/ee/audit?limit=200');
      if (!res.ok) return '';
      const rows = await res.json();
      if (!Array.isArray(rows)) return '';
      return rows
        .map((r) => {
          const when = r.at ? new Date(r.at * 1000).toISOString() : '';
          return `${when} ${r.actor || '-'} ${r.kind || ''} agent=${r.agent_id || '-'} ok=${r.ok} ${r.detail || ''}`.trim();
        })
        .join('\n');
    } catch {
      return '';
    }
  };

  const gatherLogs = async (): Promise<string> => {
    if (source === 'journal') return fetchJournal();
    if (source === 'docker_logs') return fetchDocker();
    if (source === 'audit') return fetchAudit();
    return '';
  };

  const analyze = async () => {
    if (!prompt.trim()) return;
    setLoading(true);
    setError(null);
    setResponse('');

    setStage(source === 'audit' ? 'reading audit trail…' : 'gathering logs from agent…');
    let logs = '';
    try {
      logs = await gatherLogs();
    } catch {
      logs = '';
    }

    const lineCount = logs ? logs.split('\n').filter(Boolean).length : 0;
    setStage(lineCount ? `analyzing ${lineCount} lines…` : 'no logs gathered — asking the model anyway…');

    try {
      const res = await apiFetch('/api/ee/ai/analyze', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ agent_id: agentId, source, prompt: prompt.trim(), logs }),
      });

      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `HTTP ${res.status}`);
      }

      const data = await res.json();
      const note =
        lineCount > 0
          ? `\n\n— analyzed ${lineCount} ${source.replace('_', ' ')} lines`
          : `\n\n— no ${source.replace('_', ' ')} lines were available; answer is from the question alone`;
      setResponse((data.content || 'No response.') + note);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    } finally {
      setLoading(false);
      setStage('');
    }
  };

  return (
    <div className="scroll">
      <div className="pane">
        <div className="panel">
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">✦</span> AI ANALYSIS
              <span className="meta">EE</span>
            </div>
          </div>
          <div className="panel-body" style={{ padding: 16 }}>
            <div style={{ display: 'flex', gap: 8, marginBottom: 12, flexWrap: 'wrap' }}>
              <select
                className="input"
                value={source}
                onChange={(e) => setSource(e.target.value)}
                style={{ width: 130 }}
              >
                <option value="journal">journal logs</option>
                <option value="docker_logs">docker logs</option>
                <option value="audit">audit trail</option>
              </select>
              {source === 'docker_logs' && (
                <input
                  className="input"
                  value={container}
                  onChange={(e) => setContainer(e.target.value)}
                  placeholder="container id or name"
                  style={{ width: 180 }}
                />
              )}
              <input
                className="input"
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && !e.shiftKey && analyze()}
                placeholder="e.g. What errors happened in the last hour?"
                style={{ flex: 1, minWidth: 200 }}
              />
              <button
                className="btn btn-accent"
                onClick={analyze}
                disabled={loading || !prompt.trim()}
              >
                {loading ? <Loader2Icon className="w-4 h-4 animate-spin" /> : 'analyze'}
              </button>
            </div>

            {loading && stage && (
              <div className="mono muted" style={{ fontSize: 12, marginBottom: 12 }}>
                {stage}
              </div>
            )}

            {error && (
              <div
                style={{
                  padding: 10,
                  background: 'var(--err-bg)',
                  border: '1px solid var(--err-bd)',
                  borderRadius: 'var(--r)',
                  color: 'var(--err)',
                  fontFamily: 'var(--mono)',
                  fontSize: 12,
                  marginBottom: 12,
                }}
              >
                {error}
              </div>
            )}

            {response && (
              <div
                style={{
                  padding: 16,
                  background: 'var(--bg-2)',
                  border: '1px solid var(--bd)',
                  borderRadius: 'var(--r)',
                  fontFamily: 'var(--mono)',
                  fontSize: 13,
                  lineHeight: 1.6,
                  whiteSpace: 'pre-wrap',
                  color: 'var(--fg)',
                }}
              >
                {response}
              </div>
            )}

            {!response && !error && !loading && (
              <div className="mono muted" style={{ fontSize: 12 }}>
                Pick a source and ask a question. The dashboard pulls this agent&apos;s recent{' '}
                {source === 'audit' ? 'audit events' : source === 'docker_logs' ? 'container logs' : 'journal output'} and
                hands them to the model with your question.
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
