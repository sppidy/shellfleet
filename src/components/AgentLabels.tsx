'use client';

import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { useUi } from './providers/UiProvider';
import type { LabelsResponse } from '@/lib/types';

export default function AgentLabels({ agentId }: { agentId: string }) {
  const ui = useUi();
  const [labels, setLabels] = useState<string[]>([]);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState('');

  const refresh = useCallback(async () => {
    try {
      const res = await apiFetch(
        `/api/agent-labels?agent_id=${encodeURIComponent(agentId)}`,
      );
      if (!res.ok) return;
      const data: LabelsResponse = await res.json();
      setLabels(data.by_agent[agentId] ?? []);
    } catch {
      /* swallow */
    }
  }, [agentId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const add = async (e: React.FormEvent) => {
    e.preventDefault();
    const label = draft.trim();
    if (!label) return;
    try {
      const res = await apiFetch('/api/agent-labels', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ agent_id: agentId, label }),
      });
      if (!res.ok) {
        const t = await res.text();
        throw new Error(t || `HTTP ${res.status}`);
      }
      setDraft('');
      setAdding(false);
      void refresh();
    } catch (e) {
      ui.toast('error', `Add failed: ${(e as Error).message}`);
    }
  };

  const remove = async (label: string) => {
    try {
      const res = await apiFetch(
        `/api/agent-labels/${encodeURIComponent(agentId)}/${encodeURIComponent(label)}`,
        { method: 'DELETE' },
      );
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      void refresh();
    } catch (e) {
      ui.toast('error', `Remove failed: ${(e as Error).message}`);
    }
  };

  return (
    <>
      {labels.map((l) => (
        <span key={l} className="label-chip">
          {l}
          <span className="x" onClick={() => remove(l)} title={`Remove ${l}`}>
            ×
          </span>
        </span>
      ))}
      {adding ? (
        <form onSubmit={add} style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
          <input
            type="text"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => {
              if (!draft) setAdding(false);
            }}
            autoFocus
            placeholder="label"
            className="input"
            style={{ height: 22, fontSize: 11, padding: '2px 6px', width: 96 }}
          />
        </form>
      ) : (
        <span className="label-chip add" onClick={() => setAdding(true)}>
          + label
        </span>
      )}
    </>
  );
}
