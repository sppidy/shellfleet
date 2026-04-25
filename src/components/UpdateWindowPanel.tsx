'use client';

import { useCallback, useEffect, useState } from 'react';
import { useUi } from './providers/UiProvider';
import type { UpdateWindow } from '@/lib/types';
import {
  CalendarClockIcon,
  PlayIcon,
  Loader2Icon,
  SaveIcon,
  Trash2Icon,
  AlertCircleIcon,
  CheckCircleIcon,
} from 'lucide-react';

const PRESETS: { label: string; expr: string }[] = [
  { label: 'Sundays @ 03:00', expr: '0 0 3 * * Sun *' },
  { label: 'Daily @ 04:00', expr: '0 0 4 * * * *' },
  { label: 'Mon–Fri @ 02:30', expr: '0 30 2 * * Mon-Fri *' },
  { label: '1st of month @ 05:00', expr: '0 0 5 1 * * *' },
];

function fmtTs(secs: number | null | undefined) {
  if (!secs) return '—';
  return new Date(secs * 1000).toLocaleString();
}

export default function UpdateWindowPanel({ agentId }: { agentId: string }) {
  const ui = useUi();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [running, setRunning] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [existing, setExisting] = useState<UpdateWindow | null>(null);
  const [cronExpr, setCronExpr] = useState('0 0 3 * * Sun *');
  const [enabled, setEnabled] = useState(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const res = await fetch('/api/update-windows', { credentials: 'include' });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const rows: UpdateWindow[] = await res.json();
      const mine = rows.find((r) => r.agent_id === agentId) ?? null;
      setExisting(mine);
      if (mine) {
        setCronExpr(mine.cron_expr);
        setEnabled(mine.enabled);
      }
    } catch (e) {
      ui.toast('error', `Failed to load update window: ${(e as Error).message}`);
    } finally {
      setLoading(false);
    }
  }, [agentId, ui]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const save = async () => {
    setSaving(true);
    try {
      const res = await fetch('/api/update-windows', {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ agent_id: agentId, cron_expr: cronExpr, enabled }),
      });
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      const row: UpdateWindow = await res.json();
      setExisting(row);
      ui.toast('success', 'Update window saved');
    } catch (e) {
      ui.toast('error', `Save failed: ${(e as Error).message}`);
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    const ok = await ui.confirm({
      title: 'Delete auto-update schedule?',
      description: 'This host will no longer run apt upgrades automatically.',
      destructive: true,
      confirmLabel: 'Delete',
    });
    if (!ok) return;
    setRemoving(true);
    try {
      const res = await fetch(`/api/update-windows/${encodeURIComponent(agentId)}`, {
        method: 'DELETE',
        credentials: 'include',
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setExisting(null);
      ui.toast('success', 'Schedule removed');
    } catch (e) {
      ui.toast('error', `Delete failed: ${(e as Error).message}`);
    } finally {
      setRemoving(false);
    }
  };

  const runNow = async () => {
    setRunning(true);
    try {
      const res = await fetch(
        `/api/update-windows/${encodeURIComponent(agentId)}/run`,
        { method: 'POST', credentials: 'include' },
      );
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      ui.toast('info', 'Triggered apt upgrade — check back in a minute');
      // Result lands asynchronously when the agent replies.
      setTimeout(refresh, 5_000);
    } catch (e) {
      ui.toast('error', `Run-now failed: ${(e as Error).message}`);
    } finally {
      setRunning(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-6 text-slate-500">
        <Loader2Icon className="w-4 h-4 animate-spin" />
      </div>
    );
  }

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-slate-800">
        <CalendarClockIcon className="w-4 h-4 text-slate-400" />
        <h3 className="text-sm font-semibold">Auto-update window</h3>
        {existing && (
          <span
            className={`text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded ${
              existing.enabled
                ? 'bg-emerald-500/15 text-emerald-300 border border-emerald-500/30'
                : 'bg-slate-700/40 text-slate-400 border border-slate-600/50'
            }`}
          >
            {existing.enabled ? 'enabled' : 'disabled'}
          </span>
        )}
      </div>

      <div className="px-3 py-3 space-y-3">
        <div>
          <label className="text-xs text-slate-400">Cron expression (UTC)</label>
          <input
            type="text"
            value={cronExpr}
            onChange={(e) => setCronExpr(e.target.value)}
            className="mt-1 w-full font-mono text-sm bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-slate-100 focus:outline-none focus:ring-1 focus:ring-blue-500"
            placeholder="0 0 3 * * Sun *"
            spellCheck={false}
          />
          <div className="mt-1 text-[11px] text-slate-500">
            Format: <code>sec min hour dom month dow year</code>. Examples:
          </div>
          <div className="mt-1 flex flex-wrap gap-1">
            {PRESETS.map((p) => (
              <button
                key={p.expr}
                type="button"
                onClick={() => setCronExpr(p.expr)}
                className="text-[11px] px-1.5 py-0.5 rounded border border-slate-700 text-slate-400 hover:bg-slate-800"
              >
                {p.label}
              </button>
            ))}
          </div>
        </div>

        <label className="flex items-center gap-2 text-xs text-slate-300 select-none">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            className="accent-blue-600"
          />
          Enabled
        </label>

        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={save}
            disabled={saving}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 text-white rounded-md transition-colors"
          >
            {saving ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <SaveIcon className="w-3.5 h-3.5" />
            )}
            {existing ? 'Save changes' : 'Create schedule'}
          </button>
          <button
            type="button"
            onClick={runNow}
            disabled={running}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 disabled:opacity-50 text-slate-200 rounded-md border border-slate-700 transition-colors"
          >
            {running ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <PlayIcon className="w-3.5 h-3.5" />
            )}
            Run now
          </button>
          {existing && (
            <button
              type="button"
              onClick={remove}
              disabled={removing}
              className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-red-600/20 hover:bg-red-600/40 disabled:opacity-50 text-red-200 rounded-md border border-red-600/40 transition-colors"
            >
              {removing ? (
                <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <Trash2Icon className="w-3.5 h-3.5" />
              )}
              Delete
            </button>
          )}
          <button
            type="button"
            onClick={refresh}
            className="text-xs px-2 py-1.5 text-slate-400 hover:text-slate-200"
            title="Refresh status"
          >
            ↻
          </button>
        </div>

        {existing && (
          <div className="text-xs text-slate-400 space-y-1 pt-2 border-t border-slate-800">
            <div>
              Next run:{' '}
              <span className="text-slate-200">{fmtTs(existing.next_run_at)}</span>
            </div>
            <div>
              Last run:{' '}
              <span className="text-slate-200">{fmtTs(existing.last_run_at)}</span>{' '}
              {existing.last_status && (
                <StatusBadge status={existing.last_status} />
              )}
            </div>
            {existing.last_log && (
              <details className="mt-2 rounded border border-slate-800 bg-slate-950">
                <summary className="cursor-pointer px-2 py-1 text-slate-400 hover:text-slate-200">
                  Last apt log
                </summary>
                <pre className="text-[11px] px-2 py-2 text-slate-300 whitespace-pre-wrap max-h-64 overflow-auto border-t border-slate-800">
                  {existing.last_log || '(empty)'}
                </pre>
              </details>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  if (status === 'success') {
    return (
      <span className="inline-flex items-center gap-1 text-emerald-300">
        <CheckCircleIcon className="w-3 h-3" /> success
      </span>
    );
  }
  if (status === 'failed') {
    return (
      <span className="inline-flex items-center gap-1 text-red-300">
        <AlertCircleIcon className="w-3 h-3" /> failed
      </span>
    );
  }
  if (status === 'running') {
    return (
      <span className="inline-flex items-center gap-1 text-amber-300">
        <Loader2Icon className="w-3 h-3 animate-spin" /> running
      </span>
    );
  }
  return <span className="text-slate-400">{status}</span>;
}
