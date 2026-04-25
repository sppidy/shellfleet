'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import type { DockerVolume } from '@/lib/types';
import {
  HardDriveIcon,
  RefreshCwIcon,
  Trash2Icon,
  Loader2Icon,
  AlertCircleIcon,
  EyeIcon,
  ScissorsIcon,
} from 'lucide-react';

const REFRESH_MS = 15_000;

function fmtBytes(n: number): string {
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

export default function ContainerVolumes({ agentId }: { agentId: string }) {
  const ui = useUi();
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [volumes, setVolumes] = useState<DockerVolume[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [pruning, setPruning] = useState(false);
  const [inspectName, setInspectName] = useState<string | null>(null);
  const [inspectJson, setInspectJson] = useState<string | null>(null);
  const reqTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(() => {
    setError(null);
    sendToAgent(agentId, { type: 'DockerVolumeListRequest' });
    if (reqTimeoutRef.current) clearTimeout(reqTimeoutRef.current);
    reqTimeoutRef.current = setTimeout(() => setError("agent didn't respond"), 8_000);
  }, [agentId, sendToAgent]);

  useEffect(() => {
    setVolumes(null);
    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'DockerVolumeListResponse') {
        if (reqTimeoutRef.current) {
          clearTimeout(reqTimeoutRef.current);
          reqTimeoutRef.current = null;
        }
        if (!msg.payload.available) {
          setError(msg.payload.error ?? 'docker not available');
          setVolumes([]);
          return;
        }
        setError(msg.payload.error);
        setVolumes(msg.payload.volumes);
      } else if (msg.type === 'DockerVolumeInspectResponse') {
        if (msg.payload.success) {
          setInspectJson(msg.payload.json);
        } else {
          ui.toast('error', msg.payload.error ?? 'inspect failed');
          setInspectName(null);
        }
      } else if (msg.type === 'DockerVolumeRemoveResponse') {
        setRemoving(null);
        if (msg.payload.success) {
          ui.toast('success', `Removed ${msg.payload.name}`);
        } else {
          ui.toast('error', msg.payload.error ?? 'remove failed');
        }
        refresh();
      } else if (msg.type === 'DockerVolumePruneResponse') {
        setPruning(false);
        if (msg.payload.success) {
          ui.toast(
            'success',
            `Pruned ${msg.payload.removed.length} volume(s) — reclaimed ${fmtBytes(
              msg.payload.space_reclaimed_bytes,
            )}`,
          );
        } else {
          ui.toast('error', msg.payload.error ?? 'prune failed');
        }
        refresh();
      }
    });
    refresh();
    const t = setInterval(refresh, REFRESH_MS);
    return () => {
      unsub();
      clearInterval(t);
      if (reqTimeoutRef.current) clearTimeout(reqTimeoutRef.current);
    };
  }, [agentId, onAgentMessage, refresh, ui]);

  const remove = async (v: DockerVolume, force: boolean) => {
    const ok = await ui.confirm({
      title: `Remove volume "${v.name}"?`,
      description: force
        ? 'Force removes even if in use by stopped containers.'
        : 'Will fail if any container references it.',
      destructive: true,
      confirmLabel: force ? 'Force remove' : 'Remove',
    });
    if (!ok) return;
    setRemoving(v.name);
    sendToAgent(agentId, {
      type: 'DockerVolumeRemoveRequest',
      payload: { name: v.name, force },
    });
  };

  const prune = async () => {
    const ok = await ui.confirm({
      title: 'Prune unused volumes?',
      description:
        'Removes every volume not currently referenced by any container. This frees disk but is destructive — anonymous volumes from stopped one-shot containers will also disappear.',
      destructive: true,
      confirmLabel: 'Prune',
    });
    if (!ok) return;
    setPruning(true);
    sendToAgent(agentId, { type: 'DockerVolumePruneRequest' });
  };

  const inspect = (v: DockerVolume) => {
    setInspectName(v.name);
    setInspectJson(null);
    sendToAgent(agentId, { type: 'DockerVolumeInspectRequest', payload: { name: v.name } });
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <HardDriveIcon className="w-5 h-5 text-slate-400" />
          <h2 className="text-base font-semibold">Volumes</h2>
          <span className="text-xs text-slate-500">
            {volumes === null ? 'loading…' : `· ${volumes.length}`}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={refresh}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-md"
          >
            <RefreshCwIcon className="w-3.5 h-3.5" />
            Refresh
          </button>
          <button
            type="button"
            onClick={prune}
            disabled={pruning}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-amber-600/20 hover:bg-amber-600/40 disabled:opacity-50 text-amber-200 rounded-md border border-amber-600/40"
          >
            {pruning ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <ScissorsIcon className="w-3.5 h-3.5" />
            )}
            Prune unused
          </button>
        </div>
      </div>

      {error && (
        <div className="flex items-start gap-2 text-sm text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {volumes === null ? (
        <div className="flex items-center justify-center py-12 text-slate-500">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      ) : volumes.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          No volumes.
        </div>
      ) : (
        <div className="rounded-md border border-slate-800 overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-slate-900/60 text-[11px] uppercase tracking-wide text-slate-500">
              <tr>
                <th className="text-left px-3 py-2 font-medium">Name</th>
                <th className="text-left px-3 py-2 font-medium">Driver</th>
                <th className="text-left px-3 py-2 font-medium">Mountpoint</th>
                <th className="text-right px-3 py-2 font-medium">Size</th>
                <th className="px-3 py-2"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-800">
              {volumes.map((v) => (
                <tr key={v.name} className="bg-slate-900/30">
                  <td className="px-3 py-2 font-mono text-slate-200 break-all">{v.name}</td>
                  <td className="px-3 py-2 text-slate-400">{v.driver}</td>
                  <td className="px-3 py-2 font-mono text-xs text-slate-500 break-all">
                    {v.mountpoint}
                  </td>
                  <td className="px-3 py-2 text-right text-slate-400">{fmtBytes(v.size_bytes)}</td>
                  <td className="px-3 py-2 text-right">
                    <div className="flex items-center justify-end gap-1">
                      <button
                        type="button"
                        onClick={() => inspect(v)}
                        title="Inspect"
                        className="p-1.5 rounded text-slate-400 hover:text-slate-100 hover:bg-slate-800"
                      >
                        <EyeIcon className="w-4 h-4" />
                      </button>
                      <button
                        type="button"
                        onClick={() => remove(v, false)}
                        disabled={removing === v.name}
                        title="Remove"
                        className="p-1.5 rounded text-slate-400 hover:text-red-300 hover:bg-slate-800 disabled:opacity-50"
                      >
                        {removing === v.name ? (
                          <Loader2Icon className="w-4 h-4 animate-spin" />
                        ) : (
                          <Trash2Icon className="w-4 h-4" />
                        )}
                      </button>
                      <button
                        type="button"
                        onClick={() => remove(v, true)}
                        disabled={removing === v.name}
                        title="Force remove"
                        className="text-[10px] px-1.5 py-1 rounded text-red-300/80 hover:text-red-200 hover:bg-red-500/20 disabled:opacity-50"
                      >
                        force
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {inspectName && (
        <InspectModal
          title={`Volume ${inspectName}`}
          json={inspectJson}
          onClose={() => {
            setInspectName(null);
            setInspectJson(null);
          }}
        />
      )}
    </div>
  );
}

function InspectModal({
  title,
  json,
  onClose,
}: {
  title: string;
  json: string | null;
  onClose: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 bg-slate-950/70 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="bg-slate-900 border border-slate-800 rounded-lg shadow-2xl max-w-3xl w-full max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="p-4 border-b border-slate-800 flex items-center justify-between">
          <h3 className="text-base font-semibold text-slate-100 break-all">{title}</h3>
          <button type="button" onClick={onClose} className="text-slate-400 hover:text-slate-100">
            ×
          </button>
        </div>
        <div className="flex-1 overflow-auto p-4">
          {json === null ? (
            <div className="flex items-center justify-center py-8 text-slate-500">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : (
            <pre className="text-[11px] whitespace-pre-wrap break-words text-slate-300 bg-slate-950 rounded p-3 border border-slate-800">
              {json}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
