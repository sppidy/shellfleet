'use client';

import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';
import { useUi } from './providers/UiProvider';
import type { BackupJob, BackupArchive, BackupRestoreResponse } from '@/lib/types';
import {
  ArchiveIcon,
  PlusIcon,
  Loader2Icon,
  PlayIcon,
  Trash2Icon,
  CheckCircleIcon,
  AlertCircleIcon,
  CircleDashedIcon,
  ClockIcon,
  FolderDownIcon,
  RotateCcwIcon,
} from 'lucide-react';

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
  const [loading, setLoading] = useState(true);
  const [jobs, setJobs] = useState<BackupJob[]>([]);
  const [creating, setCreating] = useState(false);
  const [running, setRunning] = useState<number | null>(null);

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

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <ArchiveIcon className="w-5 h-5 text-slate-400" />
          <h2 className="text-base font-semibold">Backups</h2>
          <span className="text-xs text-slate-500">· {jobs.length}</span>
        </div>
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-md"
        >
          <PlusIcon className="w-3.5 h-3.5" />
          New backup job
        </button>
      </div>

      <p className="text-xs text-slate-500">
        Destinations: a local path on the agent host (e.g.{' '}
        <code className="text-slate-400">/var/backups/sys-manager</code>) or
        an <code className="text-slate-400">s3://bucket/prefix</code> URI.
        S3 uploads use the agent host's <code>aws</code> CLI — install it
        and configure credentials (env vars,{' '}
        <code className="text-slate-400">~/.aws/credentials</code>, or{' '}
        <code className="text-slate-400">AWS_ENDPOINT_URL</code> for
        S3-compatible backends).
      </p>

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

      {loading && jobs.length === 0 ? (
        <div className="flex items-center justify-center py-8 text-slate-500">
          <Loader2Icon className="w-4 h-4 animate-spin" />
        </div>
      ) : jobs.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          No backup jobs configured for this host yet.
        </div>
      ) : (
        <ul className="space-y-2">
          {jobs.map((j) => (
            <li
              key={j.id}
              className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-3"
            >
              <div className="flex items-start gap-3">
                <StatusIcon status={j.last_status} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="font-medium text-slate-100 truncate">{j.name}</span>
                    {!j.enabled && (
                      <span className="text-[10px] uppercase tracking-wide px-1 py-0.5 rounded bg-slate-800 text-slate-500">
                        disabled
                      </span>
                    )}
                    {j.cron_expr && (
                      <span className="inline-flex items-center gap-1 text-[10px] uppercase tracking-wide px-1 py-0.5 rounded bg-slate-800 text-slate-300">
                        <ClockIcon className="w-2.5 h-2.5" />
                        cron
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-slate-500 mt-0.5 truncate" title={j.dest}>
                    dest: <code className="text-slate-400">{j.dest}</code>
                  </div>
                  <div className="text-xs text-slate-500 mt-0.5 truncate">
                    paths:{' '}
                    {j.paths.map((p, i) => (
                      <code key={i} className="text-slate-400 mr-1">
                        {p}
                      </code>
                    ))}
                  </div>
                  {j.cron_expr && (
                    <div className="text-[11px] text-slate-500 mt-0.5">
                      cron <code className="text-slate-400">{j.cron_expr}</code>
                      {j.next_run_at && (
                        <> · next {fmtTs(j.next_run_at)}</>
                      )}
                    </div>
                  )}
                  <div className="text-[11px] text-slate-500 mt-1">
                    last run {fmtTs(j.last_run_at)}
                    {j.last_bytes != null && j.last_bytes > 0 && (
                      <> · {fmtBytes(j.last_bytes)}</>
                    )}
                    {j.last_archive_path && (
                      <> · <code className="text-slate-400">{j.last_archive_path}</code></>
                    )}
                  </div>
                  {j.last_log && (
                    <details className="mt-2 rounded border border-slate-800 bg-slate-950">
                      <summary className="cursor-pointer px-2 py-1 text-xs text-slate-400 hover:text-slate-200">
                        last tar log
                      </summary>
                      <pre className="text-[11px] px-2 py-2 text-slate-300 whitespace-pre-wrap max-h-64 overflow-auto border-t border-slate-800">
                        {j.last_log}
                      </pre>
                    </details>
                  )}
                  <ArchivesPanel job={j} />
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  <button
                    type="button"
                    onClick={() => runNow(j)}
                    disabled={running === j.id}
                    title="Run now"
                    className="text-xs flex items-center gap-1 px-2 py-1 rounded border border-slate-700 text-slate-300 hover:bg-slate-800 disabled:opacity-50"
                  >
                    {running === j.id ? (
                      <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
                    ) : (
                      <PlayIcon className="w-3.5 h-3.5" />
                    )}
                    Run
                  </button>
                  <button
                    type="button"
                    onClick={() => remove(j)}
                    title="Delete job"
                    className="p-1.5 rounded text-slate-400 hover:text-red-400 hover:bg-slate-800"
                  >
                    <Trash2Icon className="w-4 h-4" />
                  </button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function StatusIcon({ status }: { status: string | null }) {
  if (status === 'success') {
    return <CheckCircleIcon className="w-4 h-4 mt-0.5 text-emerald-400 shrink-0" />;
  }
  if (status === 'failed') {
    return <AlertCircleIcon className="w-4 h-4 mt-0.5 text-red-400 shrink-0" />;
  }
  if (status === 'running') {
    return <Loader2Icon className="w-4 h-4 mt-0.5 animate-spin text-amber-400 shrink-0" />;
  }
  return <CircleDashedIcon className="w-4 h-4 mt-0.5 text-slate-500 shrink-0" />;
}

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
  const [paths, setPaths] = useState('/etc/sys-manager\n/etc/nginx');
  const [dest, setDest] = useState('/var/backups/sys-manager');
  const [cronExpr, setCronExpr] = useState('');
  const [enabled, setEnabled] = useState(true);
  const [mode, setMode] = useState<'tar' | 'restic'>('tar');
  const [submitting, setSubmitting] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name) return;
    const pathList = paths
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
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
    <form
      onSubmit={submit}
      className="rounded-md border border-slate-800 bg-slate-900/40 p-3 space-y-3"
    >
      <div className="grid grid-cols-2 gap-3">
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Name
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="etc-nginx"
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
            required
          />
        </label>
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Destination
          <input
            type="text"
            value={dest}
            onChange={(e) => setDest(e.target.value)}
            placeholder="/var/backups/sys-manager  or  s3://bucket/prefix"
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100"
            required
          />
        </label>
      </div>
      <label className="text-xs text-slate-400 flex flex-col gap-1 max-w-xs">
        Mode
        <select
          value={mode}
          onChange={(e) => setMode(e.target.value as 'tar' | 'restic')}
          className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
        >
          <option value="tar">tar (gzip)</option>
          <option value="restic">restic — not yet implemented</option>
        </select>
      </label>
      <label className="text-xs text-slate-400 flex flex-col gap-1">
        Paths to back up (one per line)
        <textarea
          value={paths}
          onChange={(e) => setPaths(e.target.value)}
          rows={4}
          className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100"
          required
        />
      </label>
      <div>
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Cron expression (UTC, optional — leave blank for run-now-only)
          <input
            type="text"
            value={cronExpr}
            onChange={(e) => setCronExpr(e.target.value)}
            placeholder="0 0 3 * * Sun *"
            spellCheck={false}
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100"
          />
        </label>
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
      <label className="flex items-center gap-2 text-xs text-slate-300">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => setEnabled(e.target.checked)}
          className="accent-blue-600"
        />
        Enabled
      </label>
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onClose}
          className="text-xs px-2.5 py-1.5 border border-slate-700 rounded-md text-slate-300 hover:bg-slate-800"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={submitting}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 text-white rounded-md"
        >
          {submitting && <Loader2Icon className="w-3.5 h-3.5 animate-spin" />}
          Create
        </button>
      </div>
    </form>
  );
}

function ArchivesPanel({ job }: { job: BackupJob }) {
  const ui = useUi();
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [archives, setArchives] = useState<BackupArchive[] | null>(null);
  const [restoreFor, setRestoreFor] = useState<BackupArchive | null>(null);

  const load = async () => {
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
  };

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (next && archives === null) {
      await load();
    }
  };

  return (
    <details
      open={open}
      onToggle={(e) => {
        const wasOpen = open;
        const nowOpen = (e.currentTarget as HTMLDetailsElement).open;
        if (nowOpen !== wasOpen) {
          if (nowOpen && archives === null) void load();
          setOpen(nowOpen);
        }
      }}
      className="mt-2 rounded border border-slate-800 bg-slate-950"
    >
      <summary
        className="cursor-pointer px-2 py-1 text-xs text-slate-400 hover:text-slate-200 flex items-center gap-2"
        onClick={(e) => {
          // We let the <details> toggle naturally; only log the request.
          if (!archives && !loading) {
            // First open will trigger load via onToggle.
          }
          // also call our state toggle for consistency
        }}
      >
        <FolderDownIcon className="w-3.5 h-3.5" />
        Archives
        {archives && <span className="text-slate-500">· {archives.length}</span>}
        {loading && <Loader2Icon className="w-3 h-3 animate-spin" />}
      </summary>
      <div className="border-t border-slate-800 p-2 space-y-1">
        {error ? (
          <div className="text-xs text-red-300">{error}</div>
        ) : !archives ? (
          <div className="text-xs text-slate-500">Loading…</div>
        ) : archives.length === 0 ? (
          <div className="text-xs text-slate-500 italic">No archives at this destination yet.</div>
        ) : (
          <ul className="space-y-1">
            {archives.map((a) => (
              <li
                key={a.uri}
                className="flex items-center justify-between gap-2 text-[11px] bg-slate-900 rounded px-2 py-1"
              >
                <div className="min-w-0 flex-1">
                  <div className="text-slate-200 truncate" title={a.name}>{a.name}</div>
                  <div className="text-slate-500 truncate" title={a.uri}>
                    {fmtBytes(a.bytes)} · {fmtTs(a.mtime)}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => setRestoreFor(a)}
                  className="text-xs flex items-center gap-1 px-2 py-1 rounded border border-slate-700 text-slate-300 hover:bg-slate-800"
                >
                  <RotateCcwIcon className="w-3 h-3" />
                  Restore
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex justify-end pt-1">
          <button
            type="button"
            onClick={() => void load()}
            disabled={loading}
            className="text-[11px] px-2 py-0.5 text-slate-400 hover:text-slate-100 disabled:opacity-50"
          >
            ↻ refresh
          </button>
        </div>
      </div>
      {restoreFor && (
        <RestoreModal
          job={job}
          archive={restoreFor}
          onClose={() => setRestoreFor(null)}
        />
      )}
    </details>
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
  const [destRoot, setDestRoot] = useState('/tmp/sys-manager-restore');
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
    <div
      className="fixed inset-0 z-50 bg-slate-950/70 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="bg-slate-900 border border-slate-800 rounded-lg shadow-2xl max-w-lg w-full"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="p-4 space-y-3">
          <div>
            <h3 className="text-base font-semibold text-slate-100">Restore archive</h3>
            <p className="text-xs text-slate-400 mt-1 font-mono break-all">{archive.uri}</p>
          </div>
          <label className="text-xs text-slate-400 flex flex-col gap-1">
            Destination root on agent (the agent <code>tar -xzf</code>s into this dir; nothing is overwritten in place by default)
            <input
              type="text"
              value={destRoot}
              onChange={(e) => setDestRoot(e.target.value)}
              className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100"
            />
          </label>
          {result && (
            <div
              className={`text-xs rounded-md border px-3 py-2 ${
                result.success
                  ? 'border-emerald-500/30 bg-emerald-500/5 text-emerald-200'
                  : 'border-red-500/30 bg-red-500/5 text-red-200'
              }`}
            >
              {result.success ? 'Restore succeeded.' : `Restore failed: ${result.error ?? 'unknown'}`}
              {result.log && (
                <pre className="mt-2 whitespace-pre-wrap break-words max-h-48 overflow-auto text-slate-300 bg-slate-950/50 px-2 py-1 rounded">
                  {result.log}
                </pre>
              )}
            </div>
          )}
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-slate-800 bg-slate-900/50">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded-md text-sm border border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            Close
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={submitting}
            className="px-3 py-1.5 rounded-md text-sm font-medium bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 text-white"
          >
            {submitting && <Loader2Icon className="w-3.5 h-3.5 animate-spin inline mr-1" />}
            Restore
          </button>
        </div>
      </div>
    </div>
  );
}
