'use client';

import { useState } from 'react';
import { apiFetch } from '@/lib/api';
import { Loader2Icon } from 'lucide-react';

interface Props {
  agentId: string;
}

export default function AiAnalysis({ agentId }: Props) {
  const [source, setSource] = useState('journal');
  const [prompt, setPrompt] = useState('');
  const [response, setResponse] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const analyze = async () => {
    if (!prompt.trim()) return;
    setLoading(true);
    setError(null);
    setResponse('');

    try {
      const res = await apiFetch('/api/ee/ai/analyze', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ agent_id: agentId, source, prompt: prompt.trim() }),
      });

      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `HTTP ${res.status}`);
      }

      const data = await res.json();
      setResponse(data.content || 'No response.');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    } finally {
      setLoading(false);
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
                Ask a question about this agent's logs. The AI will analyze recent output and respond.
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
