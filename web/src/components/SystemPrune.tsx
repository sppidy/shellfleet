'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import { useCanWrite } from './providers/SessionProvider';
import type { DockerSystemPrunePayload } from '@/lib/types';

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
  const canWrite = useCanWrite();
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
      if (expected !== null && msg.payload.dry_run !== expected) return;
      setBusy(null);
      expectedDryRunRef.current = null;
      if (msg.payload.dry_run) {
        setPreview(msg.payload);
      } else {
        setResult(msg.payload);
        if (msg.payload.success) {
          ui.toast('success', `Reclaimed ${fmtBytes(msg.payload.reclaimed_bytes)}`);
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
      } and free roughly ${fmtBytes(preview.reclaimed_bytes)}.`,
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
    <div className="pane">
      <div
        className="panel"
        style={{ background: 'var(--bg-2)', borderColor: 'var(--accent-bd)' }}
      >
        <div className="panel-body" style={{ fontSize: 11.5, color: 'var(--fg-1)' }}>
          <div style={{ marginBottom: 4, color: 'var(--accent)' }}>▾ Cost banner</div>
          The agent doesn&apos;t run prune in the background — nothing happens
          until you click below. Preview runs <code>docker system df -v</code>{' '}
          (fast, read-only). Apply runs <code>docker system prune -af</code>.
        </div>
      </div>

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">✂</span> SYSTEM PRUNE
          </div>
        </div>
        <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <label
            className="row"
            style={{ gap: 6, fontSize: 11.5, color: 'var(--fg-1)' }}
          >
            <input
              type="checkbox"
              checked={pruneVolumes}
              onChange={(e) => setPruneVolumes(e.target.checked)}
            />
            also prune unused volumes (passes <code>--volumes</code> — destructive)
          </label>
          <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
            <button
              className="btn"
              onClick={runPreview}
              disabled={busy !== null || !canWrite}
              title={!canWrite ? 'viewer role: read-only' : undefined}
            >
              {busy === 'preview' ? '…' : 'i preview (dry run)'}
            </button>
            <button
              className="btn warn"
              onClick={apply}
              disabled={busy !== null || !preview || !canWrite}
              title={!canWrite ? 'viewer role: read-only' : undefined}
            >
              {busy === 'apply' ? '…' : '⚠ apply'}
            </button>
          </div>
        </div>
      </div>

      {preview && <PreviewCard payload={preview} title="PREVIEW" />}
      {result && <PreviewCard payload={result} title="RESULT" />}
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
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">▤</span> {title}
          <span className="meta">
            {payload.dry_run ? 'dry run · ' : ''}
            {fmtBytes(payload.reclaimed_bytes)} · {total} item(s)
          </span>
        </div>
      </div>
      {!payload.success && payload.error && (
        <div
          style={{
            padding: 10,
            background: 'var(--err-bg)',
            color: 'var(--err)',
            fontSize: 11.5,
            fontFamily: 'var(--mono)',
            borderBottom: '1px solid var(--line)',
          }}
        >
          {payload.error}
        </div>
      )}
      <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <Section label="Containers" items={payload.containers_removed} />
        <Section label="Images" items={payload.images_removed} />
        <Section label="Networks" items={payload.networks_removed} />
        <Section label="Volumes" items={payload.volumes_removed} />
        {payload.log && (
          <details>
            <summary
              className="muted"
              style={{ cursor: 'pointer', fontSize: 11, fontFamily: 'var(--mono)' }}
            >
              raw log
            </summary>
            <pre className="code" style={{ marginTop: 4, fontSize: 10.5 }}>
              {payload.log}
            </pre>
          </details>
        )}
      </div>
    </div>
  );
}

function Section({ label, items }: { label: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <details>
      <summary
        className="muted"
        style={{ cursor: 'pointer', fontSize: 11, fontFamily: 'var(--mono)' }}
      >
        {label} ({items.length})
      </summary>
      <ul style={{ margin: '4px 0 0', padding: 0, listStyle: 'none', maxHeight: 200, overflow: 'auto' }}>
        {items.map((id, i) => (
          <li
            key={i}
            className="mono muted"
            style={{ fontSize: 10.5, padding: '2px 0', wordBreak: 'break-all' }}
          >
            {id}
          </li>
        ))}
      </ul>
    </details>
  );
}
