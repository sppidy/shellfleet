'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import { useCanWrite } from './providers/SessionProvider';
import type { DockerVolume } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

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
  const canWrite = useCanWrite();
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
        'Removes every volume not currently referenced by any container.',
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

  const totalSize = (volumes ?? []).reduce((s, v) => s + v.size_bytes, 0);

  return (
    <div className="pane">
      {error && (
        <div
          style={{
            padding: 10,
            background: 'var(--err-bg)',
            border: '1px solid var(--err-bd)',
            borderRadius: 'var(--r)',
            color: 'var(--err)',
            fontFamily: 'var(--mono)',
            fontSize: 11.5,
          }}
        >
          {error}
        </div>
      )}

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">⊠</span> VOLUMES
            <span className="meta">
              {volumes === null
                ? 'loading…'
                : `${volumes.length} volumes · ${fmtBytes(totalSize)}`}
            </span>
          </div>
          <div className="panel-actions">
            <button className="btn" onClick={refresh}>↻</button>
            <button
              className="btn warn"
              onClick={prune}
              disabled={pruning || !canWrite}
              title={!canWrite ? 'viewer role: read-only' : undefined}
            >
              {pruning ? '…' : '⚠ prune unused'}
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {volumes === null ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : volumes.length === 0 ? (
            <div className="empty">No volumes.</div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>NAME</th>
                  <th>DRIVER</th>
                  <th>MOUNTPOINT</th>
                  <th className="right">SIZE</th>
                  <th style={{ width: 200 }} />
                </tr>
              </thead>
              <tbody>
                {volumes.map((v) => (
                  <tr key={v.name}>
                    <td className="mono" style={{ color: 'var(--fg)' }}>
                      {v.name}
                    </td>
                    <td className="mono">{v.driver}</td>
                    <td className="mono muted" style={{ fontSize: 11 }}>
                      {v.mountpoint}
                    </td>
                    <td className="right mono">{fmtBytes(v.size_bytes)}</td>
                    <td className="actions">
                      <button className="btn sm" onClick={() => inspect(v)}>
                        inspect
                      </button>
                      <button
                        className="btn sm icon"
                        title={!canWrite ? 'viewer role: read-only' : 'Force remove'}
                        disabled={removing === v.name || !canWrite}
                        onClick={() => remove(v, true)}
                      >
                        !
                      </button>
                      <button
                        className="btn sm icon danger"
                        title={!canWrite ? 'viewer role: read-only' : 'Remove'}
                        disabled={removing === v.name || !canWrite}
                        onClick={() => remove(v, false)}
                      >
                        {removing === v.name ? '…' : '×'}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

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
      className="modal-overlay"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="modal" style={{ maxHeight: '80vh', display: 'flex', flexDirection: 'column' }}>
        <div className="panel-head">
          <div className="panel-title">{title}</div>
          <button className="icon-btn" onClick={onClose}>
            ×
          </button>
        </div>
        <div style={{ flex: 1, overflow: 'auto', padding: 12 }}>
          {json === null ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : (
            <pre
              className="code"
              style={{ maxHeight: 'none', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}
            >
              {json}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
