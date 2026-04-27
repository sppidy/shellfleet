'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import { useCanWrite } from './providers/SessionProvider';
import type { DockerImage } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

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
  const canWrite = useCanWrite();
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [images, setImages] = useState<DockerImage[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pulling, setPulling] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const [pullRef, setPullRef] = useState('');
  const [pullLog, setPullLog] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const reqTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(() => {
    setError(null);
    sendToAgent(agentId, { type: 'DockerImageListRequest' });
    if (reqTimeoutRef.current) clearTimeout(reqTimeoutRef.current);
    reqTimeoutRef.current = setTimeout(() => {
      setError("agent didn't respond");
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

  const q = search.trim().toLowerCase();
  const filtered = (images ?? []).filter((img) => {
    if (!q) return true;
    return (
      img.repository.toLowerCase().includes(q) ||
      img.tag.toLowerCase().includes(q) ||
      img.id.toLowerCase().includes(q)
    );
  });
  const totalSize = (images ?? []).reduce((s, i) => s + i.size_bytes, 0);

  return (
    <div className="pane">
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">⊟</span> PULL IMAGE
          </div>
        </div>
        <div className="panel-body">
          <form
            onSubmit={submitPull}
            style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}
          >
            <div className="field" style={{ flex: 1 }}>
              <label>image reference</label>
              <input
                className="input"
                type="text"
                value={pullRef}
                onChange={(e) => setPullRef(e.target.value)}
                placeholder="ghcr.io/sppidy/api:1.0.0"
              />
            </div>
            <button
              type="submit"
              className="btn primary"
              disabled={pulling || !pullRef.trim() || !canWrite}
              title={!canWrite ? 'viewer role: read-only' : undefined}
            >
              {pulling ? '…' : '▼ pull'}
            </button>
          </form>
          {pullLog !== null && (
            <details open style={{ marginTop: 8 }}>
              <summary
                className="muted"
                style={{ cursor: 'pointer', fontSize: 11, fontFamily: 'var(--mono)' }}
              >
                pull log
              </summary>
              <pre className="code" style={{ marginTop: 4 }}>
                {pullLog || '(empty)'}
              </pre>
            </details>
          )}
        </div>
      </div>

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
            <span className="ico">⊠</span> IMAGES
            <span className="meta">
              {images === null ? 'loading…' : `${images.length} images · ${fmtBytes(totalSize)}`}
            </span>
          </div>
          <div className="panel-actions">
            <div className="search-input" style={{ width: 220 }}>
              <span style={{ color: 'var(--accent)' }}>⌕</span>
              <input
                placeholder="repo, tag…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <button className="btn" onClick={refresh}>
              ↻
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {images === null ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : filtered.length === 0 ? (
            <div className="empty">
              {images.length === 0 ? 'No images on this host.' : 'No images match.'}
            </div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>REPOSITORY</th>
                  <th>TAG</th>
                  <th>IMAGE ID</th>
                  <th className="right">SIZE</th>
                  <th>CREATED</th>
                  <th style={{ width: 140 }} />
                </tr>
              </thead>
              <tbody>
                {filtered.map((img) => {
                  const dangling = img.repository === '<none>';
                  return (
                    <tr key={img.id}>
                      <td className="mono" style={{ color: 'var(--fg)' }}>
                        {dangling ? <span className="muted">&lt;none&gt;</span> : img.repository}
                      </td>
                      <td className="mono" style={{ color: 'var(--accent)' }}>
                        {img.tag === '<none>' ? <span className="muted">—</span> : img.tag}
                      </td>
                      <td className="mono muted" title={img.id}>
                        {img.id.slice(0, 12)}
                      </td>
                      <td className="right mono">{fmtBytes(img.size_bytes)}</td>
                      <td className="mono muted">{img.created}</td>
                      <td className="actions">
                        <button
                          className="btn sm icon"
                          title={!canWrite ? 'viewer role: read-only' : 'Force remove'}
                          disabled={removing === img.id || !canWrite}
                          onClick={() => remove(img, true)}
                        >
                          !
                        </button>
                        <button
                          className="btn sm icon danger"
                          title={!canWrite ? 'viewer role: read-only' : 'Remove'}
                          disabled={removing === img.id || !canWrite}
                          onClick={() => remove(img, false)}
                        >
                          {removing === img.id ? '…' : '×'}
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
    </div>
  );
}
