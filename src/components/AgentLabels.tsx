'use client';

import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { useUi } from './providers/UiProvider';
import type { LabelsResponse } from '@/lib/types';
import { TagIcon, XIcon, PlusIcon } from 'lucide-react';

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
      /* swallow — chip just won't render */
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
    <div className="flex items-center gap-1.5 flex-wrap">
      {labels.map((l) => (
        <span
          key={l}
          className="inline-flex items-center gap-1 text-[11px] px-1.5 py-0.5 rounded border border-slate-700 bg-slate-900 text-slate-300"
        >
          <TagIcon className="w-3 h-3 text-slate-500" />
          {l}
          <button
            type="button"
            onClick={() => remove(l)}
            className="text-slate-500 hover:text-red-300"
            title={`Remove ${l}`}
          >
            <XIcon className="w-3 h-3" />
          </button>
        </span>
      ))}
      {adding ? (
        <form onSubmit={add} className="flex items-center gap-1">
          <input
            type="text"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => {
              if (!draft) setAdding(false);
            }}
            autoFocus
            placeholder="label"
            className="text-[11px] bg-slate-950 border border-slate-700 rounded px-1.5 py-0.5 text-slate-100 w-24 focus:outline-none focus:ring-1 focus:ring-blue-500"
          />
        </form>
      ) : (
        <button
          type="button"
          onClick={() => setAdding(true)}
          title="Add label"
          className="inline-flex items-center gap-1 text-[11px] px-1.5 py-0.5 rounded border border-dashed border-slate-700 text-slate-500 hover:text-slate-200 hover:border-slate-500"
        >
          <PlusIcon className="w-3 h-3" />
          label
        </button>
      )}
    </div>
  );
}
