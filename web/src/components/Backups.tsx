'use client';

import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { useUi } from './providers/UiProvider';
import { useCanWrite } from './providers/SessionProvider';
import type { BackupJob, BackupArchive, BackupRestoreResponse } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

const PRESETS: { label: string; expr: string }[] = [
  { label: 'Daily @ 02:00', expr: '0 0 2 * * * *' },
  { label: 'Weekly Sun @ 03:00', expr: '0 0 3 * * Sun *' },
  { label: '1st of month @ 04:00', expr: '0 0 4 1 * * *' },
];

function fmtBytes(n: number | null | undefined): string {
  if (!n) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

function fmtTs(secs: number | null | undefined): string {
  if (!secs) return '—';
  return new Date(secs * 1000).toLocaleString();
}

export default function Backups({ agentId }: { agentId: string }) {
  const ui = useUi();
  const canWrite = useCanWrite();
  const [loading, setLoading] = useState(true);
  const [jobs, setJobs] = useState<BackupJob[]>([]);
  const [creating, setCreating] = useState(false);
  const [running, setRunning] = useState<number | null>(null);
  const [archivesFor, setArchivesFor] = useState<BackupJob | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const res = await apiFetch('/api/backups');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const rows: BackupJob[] = await res.json();
      setJobs(rows.filter((j) => j.agent_id === agentId));
    } catch (e) {
      ui.toast('error', `Load failed: ${(e as Error).message}`);
    } finally {
      setLoading(false);
    }
  }, [agentId, ui]);

  useEffect(() => {
    void refresh();
    const t = setInterval(refresh, 5_000);
    return () => clearInterval(t);
  }, [refresh]);

  const runNow = async (job: BackupJob) => {
    setRunning(job.id);
    try {
      const res = await apiFetch(`/api/backups/${job.id}/run`, { method: 'POST' });
      if (!res.ok) {
        const t = await res.text();
        throw new Error(t || `HTTP ${res.status}`);
      }
      ui.toast('info', `Backup '${job.name}' triggered`);
      setTimeout(refresh, 2_000);
    } catch (e) {
      ui.toast('error', `Run failed: ${(e as Error).message}`);
    } finally {
      setRunning(null);
    }
  };

  const remove = async (job: BackupJob) => {
    const ok = await ui.confirm({
      title: `Delete backup job "${job.name}"?`,
      destructive: true,
      confirmLabel: 'Delete',
    });
    if (!ok) return;
    try {
      const res = await apiFetch(`/api/backups/${job.id}`, { method: 'DELETE' });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      ui.toast('success', `Removed '${job.name}'`);
      void refresh();
    } catch (e) {
      ui.toast('error', `Delete failed: ${(e as Error).message}`);
    }
  };

  const totalSize = jobs.reduce((s, j) => s + (j.last_bytes ?? 0), 0);

  return (
    <div className="pane">
      {creating && (
        <BackupForm
          agentId={agentId}
          onClose={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            void refresh();
          }}
        />
      )}

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">⊞</span> BACKUP JOBS
            <span className="meta">
              {jobs.length} jobs{totalSize > 0 ? ` · ${fmtBytes(totalSize)} last run` : ''}
            </span>
          </div>
          <div className="panel-actions">
            <button
              className="btn primary"
              onClick={() => setCreating(true)}
              disabled={!canWrite}
              title={!canWrite ? 'viewer role: read-only' : undefined}
            >
              + job
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {loading && jobs.length === 0 ? (
            <div className="empty">
              <Loader2Icon className="w-4 h-4 animate-spin" />
            </div>
          ) : jobs.length === 0 ? (
            <div className="empty">No backup jobs configured for this host yet.</div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ width: 110 }}>STATUS</th>
                  <th>NAME</th>
                  <th>PATHS</th>
                  <th>DEST</th>
                  <th>CRON</th>
                  <th>LAST RUN</th>
                  <th style={{ width: 200 }} />
                </tr>
              </thead>
              <tbody>
                {jobs.map((j) => {
                  const cls =
                    j.last_status === 'success'
                      ? 'ok'
                      : j.last_status === 'failed'
                        ? 'err-c'
                        : j.last_status === 'running'
                          ? 'warn-c'
                          : 'muted';
                  return (
                    <tr key={j.id}>
                      <td>
                        <span className={`status ${cls}`}>
                          <span className="dot" />
                          {j.last_status ?? '—'}
                        </span>
                      </td>
                      <td className="mono" style={{ color: 'var(--fg)' }}>
                        {j.name}
                        {!j.enabled && (
                          <span className="chip muted" style={{ marginLeft: 8 }}>
                            disabled
                          </span>
                        )}
                      </td>
                      <td className="mono muted" title={j.paths.join(', ')}>
                        {j.paths.join(', ')}
                      </td>
                      <td className="mono">{j.dest}</td>
                      <td className="mono">{j.cron_expr || '—'}</td>
                      <td className="mono muted">
                        {fmtTs(j.last_run_at)}
                        {j.last_bytes != null && j.last_bytes > 0 && ` · ${fmtBytes(j.last_bytes)}`}
                      </td>
                      <td className="actions">
                        <button
                          className="btn sm"
                          onClick={() => runNow(j)}
                          disabled={running === j.id || !canWrite}
                          title={!canWrite ? 'viewer role: read-only' : undefined}
                        >
                          {running === j.id ? '…' : '▶ run'}
                        </button>
                        <button
                          className="btn sm"
                          onClick={() => setArchivesFor(j)}
                        >
                          archives
                        </button>
                        <button
                          className="btn sm icon danger"
                          onClick={() => remove(j)}
                          disabled={!canWrite}
                          title={!canWrite ? 'viewer role: read-only' : 'Delete job'}
                        >
                          ×
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </div>

      {archivesFor && (
        <ArchivesModal job={archivesFor} onClose={() => setArchivesFor(null)} />
      )}
    </div>
  );
}

// CanWrite is captured via the modals' own useCanWrite() calls below.
function BackupForm({
  agentId,
  onClose,
  onCreated,
}: {
  agentId: string;
  onClose: () => void;
  onCreated: () => void;
}) {
  const ui = useUi();
  const [name, setName] = useState('');
  const [paths, setPaths] = useState<string[]>(['/etc/shellfleet', '/etc/nginx']);
  const [dest, setDest] = useState('/var/backups/shellfleet');
  const [cronExpr, setCronExpr] = useState('');
  const [enabled, setEnabled] = useState(true);
  const [mode, setMode] = useState<'tar' | 'restic'>('tar');
  const [submitting, setSubmitting] = useState(false);

  // Common-paths quick-add chips. Click to append (deduped) — saves
  // typing the obvious system folders. Operator can still type any
  // path manually in the input rows.
  const COMMON_PATHS = [
    '/etc',
    '/etc/shellfleet',
    '/etc/nginx',
    '/etc/letsencrypt',
    '/root',
    '/home',
    '/var/lib/docker/volumes',
    '/opt',
    '/srv',
  ];
  const addPath = (p: string) => {
    setPaths((prev) => (prev.includes(p) ? prev : [...prev, p]));
  };
  const updatePath = (idx: number, value: string) => {
    setPaths((prev) => prev.map((v, i) => (i === idx ? value : v)));
  };
  const removePath = (idx: number) => {
    setPaths((prev) => prev.filter((_, i) => i !== idx));
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name) return;
    const pathList = paths.map((p) => p.trim()).filter((p) => p.length > 0);
    if (pathList.length === 0) {
      ui.toast('error', 'At least one path required');
      return;
    }
    setSubmitting(true);
    try {
      const res = await apiFetch('/api/backups', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          agent_id: agentId,
          name,
          paths: pathList,
          dest,
          cron_expr: cronExpr.trim() || null,
          enabled,
          mode,
        }),
      });
      if (!res.ok) {
        const t = await res.text();
        throw new Error(t || `HTTP ${res.status}`);
      }
      ui.toast('success', `Job "${name}" created`);
      onCreated();
    } catch (e) {
      ui.toast('error', `Create failed: ${(e as Error).message}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">+</span> NEW BACKUP JOB
        </div>
      </div>
      <form
        onSubmit={submit}
        className="panel-body"
        style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        <div className="grid-2">
          <div className="field">
            <label>name</label>
            <input
              className="input"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="etc-nginx"
              required
            />
          </div>
          <div className="field">
            <label>destination</label>
            <input
              className="input"
              type="text"
              value={dest}
              onChange={(e) => setDest(e.target.value)}
              placeholder="/var/backups/shellfleet  or  s3://bucket/prefix"
              required
            />
          </div>
        </div>

        <div className="grid-2">
          <div className="field">
            <label>mode</label>
            <select
              className="select"
              value={mode}
              onChange={(e) => setMode(e.target.value as 'tar' | 'restic')}
            >
              <option value="tar">tar (gzip)</option>
              <option value="restic">restic — not yet implemented</option>
            </select>
          </div>
          <div className="field">
            <label>options</label>
            <label
              className="row"
              style={{ gap: 6, fontSize: 12, height: 28, alignItems: 'center' }}
            >
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => setEnabled(e.target.checked)}
              />
              enabled
            </label>
          </div>
        </div>

        <div className="field">
          <label>paths to back up</label>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {paths.map((p, i) => (
              <div key={i} style={{ display: 'flex', gap: 4 }}>
                <input
                  className="input"
                  type="text"
                  value={p}
                  onChange={(e) => updatePath(i, e.target.value)}
                  placeholder="/path/to/folder"
                  spellCheck={false}
                  style={{ flex: 1, fontFamily: 'var(--mono)', fontSize: 12 }}
                />
                <button
                  type="button"
                  onClick={() => removePath(i)}
                  className="btn ghost sm"
                  title="remove this path"
                  style={{ height: 28, padding: '0 8px' }}
                  disabled={paths.length === 1}
                >
                  ×
                </button>
              </div>
            ))}
            <button
              type="button"
              onClick={() => setPaths((prev) => [...prev, ''])}
              className="btn sm"
              style={{ alignSelf: 'flex-start', height: 24, fontSize: 11, padding: '0 10px' }}
            >
              ＋ add path
            </button>
          </div>
          <div
            style={{
              display: 'flex',
              flexWrap: 'wrap',
              gap: 4,
              marginTop: 6,
              fontSize: 11,
              color: 'var(--fg-2)',
            }}
          >
            <span style={{ color: 'var(--fg-3)', alignSelf: 'center' }}>quick add:</span>
            {COMMON_PATHS.map((p) => (
              <button
                key={p}
                type="button"
                onClick={() => addPath(p)}
                disabled={paths.includes(p)}
                style={{
                  background: 'transparent',
                  border: '1px solid var(--line)',
                  borderRadius: 12,
                  color: paths.includes(p) ? 'var(--fg-3)' : 'var(--fg-2)',
                  cursor: paths.includes(p) ? 'default' : 'pointer',
                  fontFamily: 'var(--mono)',
                  fontSize: 10,
                  padding: '2px 8px',
                  opacity: paths.includes(p) ? 0.5 : 1,
                }}
                title={paths.includes(p) ? 'already added' : `add ${p}`}
              >
                {p}
              </button>
            ))}
          </div>
        </div>

        <div className="field">
          <label>cron expression (UTC, optional)</label>
          <input
            className="input"
            type="text"
            value={cronExpr}
            onChange={(e) => setCronExpr(e.target.value)}
            placeholder="0 0 3 * * Sun *"
            spellCheck={false}
          />
          <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
            {PRESETS.map((p) => (
              <button
                key={p.expr}
                type="button"
                className="btn sm"
                onClick={() => setCronExpr(p.expr)}
              >
                {p.label}
              </button>
            ))}
          </div>
        </div>

        <div className="row between">
          <button type="button" className="btn" onClick={onClose}>
            cancel
          </button>
          <button type="submit" className="btn primary" disabled={submitting}>
            {submitting ? '…' : '+ create'}
          </button>
        </div>
      </form>
    </div>
  );
}

function ArchivesModal({ job, onClose }: { job: BackupJob; onClose: () => void }) {
  const canWrite = useCanWrite();
  const ui = useUi();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [archives, setArchives] = useState<BackupArchive[] | null>(null);
  const [restoreFor, setRestoreFor] = useState<BackupArchive | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await apiFetch(`/api/backups/${job.id}/archives`, { method: 'POST' });
      if (!res.ok) {
        const t = await res.text();
        throw new Error(t || `HTTP ${res.status}`);
      }
      const data: BackupArchive[] = await res.json();
      setArchives(data);
    } catch (e) {
      setError((e as Error).message);
      ui.toast('error', `List failed: ${(e as Error).message}`);
    } finally {
      setLoading(false);
    }
  }, [job.id, ui]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div
        className="modal"
        style={{ width: 'min(720px, 95vw)', maxHeight: '85vh', display: 'flex', flexDirection: 'column' }}
      >
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">⌹</span> ARCHIVES
            <span className="meta">{job.name}</span>
          </div>
          <div className="panel-actions">
            <button className="btn sm" onClick={load} disabled={loading}>
              ↻
            </button>
            <button className="icon-btn" onClick={onClose}>
              ×
            </button>
          </div>
        </div>
        <div style={{ flex: 1, overflow: 'auto' }}>
          {loading ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : error ? (
            <div className="empty err-c">{error}</div>
          ) : !archives || archives.length === 0 ? (
            <div className="empty">No archives at this destination yet.</div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>NAME</th>
                  <th className="right">SIZE</th>
                  <th>CREATED</th>
                  <th style={{ width: 120 }} />
                </tr>
              </thead>
              <tbody>
                {archives.map((a) => (
                  <tr key={a.uri}>
                    <td className="mono" style={{ color: 'var(--fg)' }}>
                      {a.name}
                    </td>
                    <td className="right mono">{fmtBytes(a.bytes)}</td>
                    <td className="mono muted">{fmtTs(a.mtime)}</td>
                    <td className="actions">
                      <button
                        className="btn sm"
                        onClick={() => setRestoreFor(a)}
                        disabled={!canWrite}
                        title={!canWrite ? 'viewer role: read-only' : undefined}
                      >
                        restore
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        {restoreFor && (
          <RestoreModal
            job={job}
            archive={restoreFor}
            onClose={() => setRestoreFor(null)}
          />
        )}
      </div>
    </div>
  );
}

function RestoreModal({
  job,
  archive,
  onClose,
}: {
  job: BackupJob;
  archive: BackupArchive;
  onClose: () => void;
}) {
  const ui = useUi();
  const [destRoot, setDestRoot] = useState('/tmp/shellfleet-restore');
  const [submitting, setSubmitting] = useState(false);
  const [result, setResult] = useState<BackupRestoreResponse | null>(null);

  const submit = async () => {
    if (!destRoot.trim()) return;
    setSubmitting(true);
    setResult(null);
    try {
      const res = await apiFetch(`/api/backups/${job.id}/restore`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ archive_uri: archive.uri, dest_root: destRoot }),
      });
      if (!res.ok) {
        const t = await res.text();
        throw new Error(t || `HTTP ${res.status}`);
      }
      const data: BackupRestoreResponse = await res.json();
      setResult(data);
      if (data.success) {
        ui.toast('success', `Restored into ${destRoot}`);
      } else {
        ui.toast('error', data.error || 'Restore failed');
      }
    } catch (e) {
      ui.toast('error', `Restore failed: ${(e as Error).message}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">↺</span> RESTORE ARCHIVE
          </div>
          <button className="icon-btn" onClick={onClose}>
            ×
          </button>
        </div>
        <div
          className="panel-body"
          style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
        >
          <div className="muted mono" style={{ fontSize: 11, wordBreak: 'break-all' }}>
            {archive.uri}
          </div>
          <div className="field">
            <label>destination root on agent</label>
            <input
              className="input"
              type="text"
              value={destRoot}
              onChange={(e) => setDestRoot(e.target.value)}
            />
            <div className="muted" style={{ fontSize: 10.5 }}>
              the agent <code>tar -xzf</code>s into this dir; nothing is overwritten in place by default
            </div>
          </div>
          {result && (
            <div
              style={{
                padding: 8,
                background: result.success ? 'var(--accent-bg)' : 'var(--err-bg)',
                border: `1px solid ${result.success ? 'var(--accent-bd)' : 'var(--err-bd)'}`,
                borderRadius: 'var(--r)',
                color: result.success ? 'var(--accent)' : 'var(--err)',
                fontFamily: 'var(--mono)',
                fontSize: 11.5,
              }}
            >
              {result.success ? 'Restore succeeded.' : `Restore failed: ${result.error ?? 'unknown'}`}
              {result.log && (
                <pre className="code" style={{ marginTop: 6, fontSize: 10.5 }}>
                  {result.log}
                </pre>
              )}
            </div>
          )}
        </div>
        <div className="modal-foot">
          <button type="button" className="btn" onClick={onClose}>
            close
          </button>
          <button
            type="button"
            className="btn primary"
            onClick={submit}
            disabled={submitting}
          >
            {submitting ? '…' : '↺ restore'}
          </button>
        </div>
      </div>
    </div>
  );
}
