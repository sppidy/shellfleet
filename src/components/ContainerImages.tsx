'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import type { DockerImage } from '@/lib/types';
import {
  ImageIcon,
  RefreshCwIcon,
  Trash2Icon,
  DownloadIcon,
  Loader2Icon,
  AlertCircleIcon,
} from 'lucide-react';

const REFRESH_MS = 15_000;
const REQ_TIMEOUT_MS = 60_000;

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

export default function ContainerImages({ agentId }: { agentId: string }) {
  const ui = useUi();
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [images, setImages] = useState<DockerImage[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pulling, setPulling] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const [pullRef, setPullRef] = useState('');
  const [pullLog, setPullLog] = useState<string | null>(null);
  const reqTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(() => {
    setError(null);
    sendToAgent(agentId, { type: 'DockerImageListRequest' });
    if (reqTimeoutRef.current) clearTimeout(reqTimeoutRef.current);
    reqTimeoutRef.current = setTimeout(() => {
      setError('agent didn\'t respond');
    }, 8_000);
  }, [agentId, sendToAgent]);

  useEffect(() => {
    setImages(null);
    setError(null);
    setPullLog(null);

    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'DockerImageListResponse') {
        if (reqTimeoutRef.current) {
          clearTimeout(reqTimeoutRef.current);
          reqTimeoutRef.current = null;
        }
        if (!msg.payload.available) {
          setError(msg.payload.error ?? 'docker not available');
          setImages([]);
          return;
        }
        setError(msg.payload.error);
        setImages(msg.payload.images);
      } else if (msg.type === 'DockerImagePullResponse') {
        setPulling(false);
        setPullLog(msg.payload.log || msg.payload.error || '');
        if (msg.payload.success) {
          ui.toast('success', `Pulled ${msg.payload.reference}`);
        } else {
          ui.toast('error', msg.payload.error ?? `Pull failed`);
        }
        refresh();
      } else if (msg.type === 'DockerImageRemoveResponse') {
        setRemoving(null);
        if (msg.payload.success) {
          ui.toast('success', `Removed ${msg.payload.id.slice(0, 12)}`);
        } else {
          ui.toast('error', msg.payload.error ?? `Remove failed`);
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

  const submitPull = (e: React.FormEvent) => {
    e.preventDefault();
    const ref = pullRef.trim();
    if (!ref) return;
    setPulling(true);
    setPullLog(null);
    sendToAgent(agentId, {
      type: 'DockerImagePullRequest',
      payload: { reference: ref },
    });
    setTimeout(() => {
      setPulling(false);
      setPullLog((curr) => curr ?? 'pull timed out (still running on the agent — refresh in a bit)');
    }, REQ_TIMEOUT_MS);
  };

  const remove = async (img: DockerImage, force: boolean) => {
    const label = img.repository === '<none>' ? img.id.slice(0, 12) : `${img.repository}:${img.tag}`;
    const ok = await ui.confirm({
      title: `Remove image ${label}?`,
      description: force
        ? 'This passes --force, which removes the image even if there are stopped containers using it.'
        : 'Will fail if any container references this image. Use Force remove if needed.',
      destructive: true,
      confirmLabel: force ? 'Force remove' : 'Remove',
    });
    if (!ok) return;
    setRemoving(img.id);
    sendToAgent(agentId, {
      type: 'DockerImageRemoveRequest',
      payload: { id: img.id, force },
    });
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <ImageIcon className="w-5 h-5 text-slate-400" />
          <h2 className="text-base font-semibold">Images</h2>
          <span className="text-xs text-slate-500">
            {images === null ? 'loading…' : `· ${images.length}`}
          </span>
        </div>
        <button
          type="button"
          onClick={refresh}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-md"
        >
          <RefreshCwIcon className="w-3.5 h-3.5" />
          Refresh
        </button>
      </div>

      <form
        onSubmit={submitPull}
        className="rounded-md border border-slate-800 bg-slate-900/40 p-3 flex items-center gap-2 flex-wrap"
      >
        <input
          type="text"
          value={pullRef}
          onChange={(e) => setPullRef(e.target.value)}
          placeholder="image reference (e.g. nginx:1.27, ghcr.io/owner/image@sha256:…)"
          className="flex-1 min-w-[20ch] bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100"
        />
        <button
          type="submit"
          disabled={pulling || !pullRef.trim()}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 text-white rounded-md"
        >
          {pulling ? (
            <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <DownloadIcon className="w-3.5 h-3.5" />
          )}
          Pull
        </button>
      </form>

      {pullLog !== null && (
        <details
          open
          className="rounded-md border border-slate-800 bg-slate-950"
        >
          <summary className="cursor-pointer px-2 py-1 text-xs text-slate-400 hover:text-slate-200">
            pull log
          </summary>
          <pre className="text-[11px] px-2 py-2 text-slate-300 whitespace-pre-wrap max-h-72 overflow-auto border-t border-slate-800">
            {pullLog || '(empty)'}
          </pre>
        </details>
      )}

      {error && (
        <div className="flex items-start gap-2 text-sm text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {images === null ? (
        <div className="flex items-center justify-center py-12 text-slate-500">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      ) : images.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          No images on this host.
        </div>
      ) : (
        <div className="rounded-md border border-slate-800 overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-slate-900/60 text-[11px] uppercase tracking-wide text-slate-500">
              <tr>
                <th className="text-left px-3 py-2 font-medium">Repository</th>
                <th className="text-left px-3 py-2 font-medium">Tag</th>
                <th className="text-left px-3 py-2 font-medium">ID</th>
                <th className="text-right px-3 py-2 font-medium">Size</th>
                <th className="text-left px-3 py-2 font-medium">Created</th>
                <th className="px-3 py-2"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-800">
              {images.map((img) => {
                const dangling = img.repository === '<none>';
                return (
                  <tr key={img.id} className="bg-slate-900/30">
                    <td className="px-3 py-2 font-mono text-slate-200 break-all">
                      {dangling ? (
                        <span className="text-slate-500 italic">&lt;none&gt;</span>
                      ) : (
                        img.repository
                      )}
                    </td>
                    <td className="px-3 py-2 font-mono text-slate-300">
                      {img.tag === '<none>' ? <span className="text-slate-500">—</span> : img.tag}
                    </td>
                    <td className="px-3 py-2 font-mono text-xs text-slate-500" title={img.id}>
                      {img.id.slice(0, 12)}
                    </td>
                    <td className="px-3 py-2 text-right text-slate-400">{fmtBytes(img.size_bytes)}</td>
                    <td className="px-3 py-2 text-xs text-slate-500">{img.created}</td>
                    <td className="px-3 py-2 text-right">
                      <div className="flex items-center justify-end gap-1">
                        <button
                          type="button"
                          onClick={() => remove(img, false)}
                          disabled={removing === img.id}
                          title="Remove"
                          className="p-1.5 rounded text-slate-400 hover:text-red-300 hover:bg-slate-800 disabled:opacity-50"
                        >
                          {removing === img.id ? (
                            <Loader2Icon className="w-4 h-4 animate-spin" />
                          ) : (
                            <Trash2Icon className="w-4 h-4" />
                          )}
                        </button>
                        <button
                          type="button"
                          onClick={() => remove(img, true)}
                          disabled={removing === img.id}
                          title="Force remove"
                          className="text-[10px] px-1.5 py-1 rounded text-red-300/80 hover:text-red-200 hover:bg-red-500/20 disabled:opacity-50"
                        >
                          force
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
