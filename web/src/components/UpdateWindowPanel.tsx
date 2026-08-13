'use client';

import { useCallback, useEffect, useState } from 'react';
import { useUi } from './providers/UiProvider';
import { useCanWrite } from './providers/SessionProvider';
import { apiFetch } from '@/lib/api';
import type { UpdateWindow } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

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
  const canWrite = useCanWrite();
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
      const res = await apiFetch('/api/update-windows');
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
      const res = await apiFetch('/api/update-windows', {
        method: 'POST',
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
      const res = await apiFetch(`/api/update-windows/${encodeURIComponent(agentId)}`, {
        method: 'DELETE',
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
      const res = await apiFetch(
        `/api/update-windows/${encodeURIComponent(agentId)}/run`,
        { method: 'POST' },
      );
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      ui.toast('info', 'Triggered apt upgrade — check back in a minute');
      setTimeout(refresh, 5_000);
    } catch (e) {
      ui.toast('error', `Run-now failed: ${(e as Error).message}`);
    } finally {
      setRunning(false);
    }
  };

  if (loading) {
    return (
      <div className="panel">
        <div className="empty">
          <Loader2Icon className="w-4 h-4 animate-spin" />
        </div>
      </div>
    );
  }

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">⏰</span> AUTO-UPDATE WINDOW
        </div>
        <div className="panel-actions">
          {existing && (
            <span className={`pill ${existing.enabled ? 'live' : ''}`}>
              <span className={`dot ${existing.enabled ? 'pulse' : ''}`} />
              {existing.enabled ? 'enabled' : 'disabled'}
            </span>
          )}
        </div>
      </div>
      <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div className="grid-3">
          <div className="field" style={{ gridColumn: 'span 2' }}>
            <label>cron expression (UTC)</label>
            <input
              className="input"
              type="text"
              value={cronExpr}
              onChange={(e) => setCronExpr(e.target.value)}
              placeholder="0 0 3 * * Sun *"
              spellCheck={false}
            />
            <div className="muted" style={{ fontSize: 10.5 }}>
              format: <code>sec min hour dom month dow year</code>
            </div>
          </div>
          <div className="field">
            <label>presets</label>
            <select
              className="select"
              value=""
              onChange={(e) => e.target.value && setCronExpr(e.target.value)}
            >
              <option value="">— pick a preset —</option>
              {PRESETS.map((p) => (
                <option key={p.expr} value={p.expr}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>
        </div>

        <label
          className="row"
          style={{ gap: 6, fontSize: 11.5, color: 'var(--fg-1)' }}
        >
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
          />
          enabled
        </label>

        <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
          <button
            className="btn primary"
            onClick={save}
            disabled={saving || !canWrite}
            title={!canWrite ? 'viewer role: read-only' : undefined}
          >
            {saving ? '…' : existing ? '▼ save changes' : '＋ create schedule'}
          </button>
          <button
            className="btn"
            onClick={runNow}
            disabled={running || !canWrite}
            title={!canWrite ? 'viewer role: read-only' : undefined}
          >
            {running ? '…' : '▶ run now'}
          </button>
          {existing && (
            <button
              className="btn danger"
              onClick={remove}
              disabled={removing || !canWrite}
              title={!canWrite ? 'viewer role: read-only' : undefined}
            >
              {removing ? '…' : '× delete'}
            </button>
          )}
          <button className="btn sm" onClick={refresh} title="Refresh">
            ↻
          </button>
        </div>

        {existing && (
          <div
            style={{
              borderTop: '1px solid var(--line)',
              paddingTop: 8,
              fontSize: 11,
              fontFamily: 'var(--mono)',
              color: 'var(--fg-1)',
              display: 'flex',
              flexDirection: 'column',
              gap: 4,
            }}
          >
            <div>
              next run: <span style={{ color: 'var(--fg)' }}>{fmtTs(existing.next_run_at)}</span>
            </div>
            <div>
              last run: <span style={{ color: 'var(--fg)' }}>{fmtTs(existing.last_run_at)}</span>{' '}
              {existing.last_status && <StatusBadge status={existing.last_status} />}
            </div>
            {existing.last_log && (
              <details style={{ marginTop: 6 }}>
                <summary
                  className="muted"
                  style={{ cursor: 'pointer', fontSize: 11 }}
                >
                  last apt log
                </summary>
                <pre className="code" style={{ marginTop: 4, fontSize: 10.5 }}>
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
  if (status === 'success') return <span className="ok">✓ success</span>;
  if (status === 'failed') return <span className="err-c">× failed</span>;
  if (status === 'running') return <span className="warn-c">… running</span>;
  return <span className="muted">{status}</span>;
}
