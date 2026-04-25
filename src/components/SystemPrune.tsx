'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import type { DockerSystemPrunePayload } from '@/lib/types';
import {
  Trash2Icon,
  ScissorsIcon,
  Loader2Icon,
  AlertCircleIcon,
  InfoIcon,
} from 'lucide-react';

function fmtBytes(n: number): string {
  if (!n) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export default function SystemPrune({ agentId }: { agentId: string }) {
  const ui = useUi();
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [pruneVolumes, setPruneVolumes] = useState(false);
  const [busy, setBusy] = useState<'preview' | 'apply' | null>(null);
  const [preview, setPreview] = useState<DockerSystemPrunePayload | null>(null);
  const [result, setResult] = useState<DockerSystemPrunePayload | null>(null);
  const expectedDryRunRef = useRef<boolean | null>(null);

  useEffect(() => {
    setPreview(null);
    setResult(null);
    setBusy(null);
    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type !== 'DockerSystemPruneResponse') return;
      const expected = expectedDryRunRef.current;
      // Defensive: only accept the response we're waiting for.
      if (expected !== null && msg.payload.dry_run !== expected) return;
      setBusy(null);
      expectedDryRunRef.current = null;
      if (msg.payload.dry_run) {
        setPreview(msg.payload);
      } else {
        setResult(msg.payload);
        if (msg.payload.success) {
          ui.toast(
            'success',
            `Reclaimed ${fmtBytes(msg.payload.reclaimed_bytes)}`,
          );
        } else {
          ui.toast('error', msg.payload.error ?? 'prune failed');
        }
      }
    });
    return unsub;
  }, [agentId, onAgentMessage, ui]);

  const runPreview = () => {
    setBusy('preview');
    setPreview(null);
    expectedDryRunRef.current = true;
    sendToAgent(agentId, {
      type: 'DockerSystemPruneRequest',
      payload: { dry_run: true, prune_volumes: pruneVolumes },
    });
  };

  const apply = async () => {
    if (!preview) return;
    const ok = await ui.confirm({
      title: 'Run docker system prune?',
      description: `This will remove ${preview.containers_removed.length} container(s), ${preview.images_removed.length} image(s), ${preview.networks_removed.length} network(s)${
        pruneVolumes ? `, ${preview.volumes_removed.length} volume(s)` : ''
      } and free roughly ${fmtBytes(preview.reclaimed_bytes)}. The actual amount may differ — preview is best-effort.`,
      destructive: true,
      confirmLabel: 'Prune now',
    });
    if (!ok) return;
    setBusy('apply');
    setResult(null);
    expectedDryRunRef.current = false;
    sendToAgent(agentId, {
      type: 'DockerSystemPruneRequest',
      payload: { dry_run: false, prune_volumes: pruneVolumes },
    });
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <ScissorsIcon className="w-5 h-5 text-slate-400" />
        <h2 className="text-base font-semibold">System prune</h2>
      </div>

      <div className="rounded-md border border-slate-800 bg-slate-900/40 p-3 text-xs text-slate-400 flex items-start gap-2">
        <InfoIcon className="w-4 h-4 mt-0.5 text-slate-500 shrink-0" />
        <div className="space-y-1">
          <p>
            Reclaims disk by removing stopped containers, dangling images,
            unused networks, and (optionally) unused volumes. The agent
            doesn&apos;t run prune in the background — nothing happens until
            you click below.
          </p>
          <p>
            Use <strong>Preview</strong> first; it runs <code>docker system df -v</code>{' '}
            and reports the upper bound on what would be deleted.
            <strong> Apply</strong> runs <code>docker system prune -af</code>.
          </p>
        </div>
      </div>

      <div className="rounded-md border border-slate-800 bg-slate-900/40 p-3 space-y-3">
        <label className="flex items-center gap-2 text-xs text-slate-300">
          <input
            type="checkbox"
            checked={pruneVolumes}
            onChange={(e) => setPruneVolumes(e.target.checked)}
            className="accent-blue-600"
          />
          Also prune unused volumes (passes <code>--volumes</code> — destructive,
          may delete anonymous volumes from removed one-shot containers)
        </label>
        <div className="flex gap-2 flex-wrap">
          <button
            type="button"
            onClick={runPreview}
            disabled={busy !== null}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 disabled:opacity-50 text-slate-200 rounded-md border border-slate-700"
          >
            {busy === 'preview' ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <InfoIcon className="w-3.5 h-3.5" />
            )}
            Preview (dry run)
          </button>
          <button
            type="button"
            onClick={apply}
            disabled={busy !== null || !preview}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-amber-600/20 hover:bg-amber-600/40 disabled:opacity-30 text-amber-200 rounded-md border border-amber-600/40"
          >
            {busy === 'apply' ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Trash2Icon className="w-3.5 h-3.5" />
            )}
            Apply
          </button>
        </div>
      </div>

      {preview && <PreviewCard payload={preview} title="Preview" />}
      {result && <PreviewCard payload={result} title="Result" />}
    </div>
  );
}

function PreviewCard({
  payload,
  title,
}: {
  payload: DockerSystemPrunePayload;
  title: string;
}) {
  const total =
    payload.containers_removed.length +
    payload.images_removed.length +
    payload.networks_removed.length +
    payload.volumes_removed.length;
  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40">
      <div className="px-3 py-2 border-b border-slate-800 flex items-center justify-between">
        <div className="text-sm font-semibold text-slate-100">
          {title} {payload.dry_run ? '(dry run)' : ''}
        </div>
        <div className="text-xs text-slate-400">
          {fmtBytes(payload.reclaimed_bytes)} · {total} item(s)
        </div>
      </div>
      {!payload.success && payload.error && (
        <div className="m-3 flex items-start gap-2 text-xs text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{payload.error}</span>
        </div>
      )}
      <div className="divide-y divide-slate-800 text-xs">
        <Section label="Containers" items={payload.containers_removed} />
        <Section label="Images" items={payload.images_removed} />
        <Section label="Networks" items={payload.networks_removed} />
        <Section label="Volumes" items={payload.volumes_removed} />
      </div>
      {payload.log && (
        <details className="border-t border-slate-800">
          <summary className="cursor-pointer px-3 py-1.5 text-xs text-slate-400 hover:text-slate-200">
            raw log
          </summary>
          <pre className="text-[11px] px-3 py-2 text-slate-300 whitespace-pre-wrap max-h-72 overflow-auto bg-slate-950">
            {payload.log}
          </pre>
        </details>
      )}
    </div>
  );
}

function Section({ label, items }: { label: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <details className="px-3 py-2">
      <summary className="cursor-pointer text-slate-400 hover:text-slate-200">
        {label} ({items.length})
      </summary>
      <ul className="mt-2 space-y-0.5 max-h-48 overflow-auto">
        {items.map((id, i) => (
          <li key={i} className="font-mono text-[11px] text-slate-500 break-all">
            {id}
          </li>
        ))}
      </ul>
    </details>
  );
}
