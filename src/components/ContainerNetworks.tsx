'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import type { DockerNetwork } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

const REFRESH_MS = 15_000;

export default function ContainerNetworks({ agentId }: { agentId: string }) {
  const ui = useUi();
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [networks, setNetworks] = useState<DockerNetwork[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [inspectId, setInspectId] = useState<string | null>(null);
  const [inspectJson, setInspectJson] = useState<string | null>(null);
  const reqTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(() => {
    setError(null);
    sendToAgent(agentId, { type: 'DockerNetworkListRequest' });
    if (reqTimeoutRef.current) clearTimeout(reqTimeoutRef.current);
    reqTimeoutRef.current = setTimeout(() => setError("agent didn't respond"), 8_000);
  }, [agentId, sendToAgent]);

  useEffect(() => {
    setNetworks(null);
    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'DockerNetworkListResponse') {
        if (reqTimeoutRef.current) {
          clearTimeout(reqTimeoutRef.current);
          reqTimeoutRef.current = null;
        }
        if (!msg.payload.available) {
          setError(msg.payload.error ?? 'docker not available');
          setNetworks([]);
          return;
        }
        setError(msg.payload.error);
        setNetworks(msg.payload.networks);
      } else if (msg.type === 'DockerNetworkInspectResponse') {
        if (msg.payload.success) {
          setInspectJson(msg.payload.json);
        } else {
          ui.toast('error', msg.payload.error ?? 'inspect failed');
          setInspectId(null);
        }
      } else if (msg.type === 'DockerNetworkRemoveResponse') {
        setRemoving(null);
        if (msg.payload.success) {
          ui.toast('success', `Removed ${msg.payload.id.slice(0, 12)}`);
        } else {
          ui.toast('error', msg.payload.error ?? 'remove failed');
        }
        refresh();
      } else if (msg.type === 'DockerNetworkCreateResponse') {
        if (msg.payload.success) {
          ui.toast('success', `Created network ${msg.payload.name}`);
        } else {
          ui.toast('error', msg.payload.error ?? 'create failed');
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

  const remove = async (n: DockerNetwork) => {
    const ok = await ui.confirm({
      title: `Remove network "${n.name}"?`,
      description: 'Will fail if any container is connected to it.',
      destructive: true,
      confirmLabel: 'Remove',
    });
    if (!ok) return;
    setRemoving(n.id);
    sendToAgent(agentId, { type: 'DockerNetworkRemoveRequest', payload: { id: n.id } });
  };

  const inspect = (n: DockerNetwork) => {
    setInspectId(n.id);
    setInspectJson(null);
    sendToAgent(agentId, { type: 'DockerNetworkInspectRequest', payload: { id: n.id } });
  };

  return (
    <div className="pane">
      <CreateForm agentId={agentId} />

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
            <span className="ico">⊟</span> NETWORKS
            <span className="meta">
              {networks === null ? 'loading…' : `${networks.length} networks`}
            </span>
          </div>
          <div className="panel-actions">
            <button className="btn" onClick={refresh}>↻</button>
          </div>
        </div>
        <div className="panel-body flush">
          {networks === null ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : networks.length === 0 ? (
            <div className="empty">No networks.</div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>NAME</th>
                  <th>DRIVER</th>
                  <th>SCOPE</th>
                  <th>ID</th>
                  <th>FLAGS</th>
                  <th style={{ width: 140 }} />
                </tr>
              </thead>
              <tbody>
                {networks.map((n) => {
                  const flags: string[] = [];
                  if (n.attachable) flags.push('attachable');
                  if (n.internal) flags.push('internal');
                  if (n.ipv6) flags.push('ipv6');
                  return (
                    <tr key={n.id}>
                      <td className="mono" style={{ color: 'var(--fg)' }}>
                        {n.name}
                      </td>
                      <td className="mono">{n.driver}</td>
                      <td className={`mono ${n.scope === 'swarm' ? 'info-c' : ''}`}>
                        {n.scope}
                      </td>
                      <td className="mono muted">{n.id.slice(0, 12)}</td>
                      <td className="mono">{flags.join(', ') || '—'}</td>
                      <td className="actions">
                        <button className="btn sm" onClick={() => inspect(n)}>
                          inspect
                        </button>
                        <button
                          className="btn sm icon danger"
                          title="Remove"
                          disabled={removing === n.id}
                          onClick={() => remove(n)}
                        >
                          {removing === n.id ? '…' : '×'}
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

      {inspectId && (
        <InspectModal
          title={`Network ${inspectId.slice(0, 12)}`}
          json={inspectJson}
          onClose={() => {
            setInspectId(null);
            setInspectJson(null);
          }}
        />
      )}
    </div>
  );
}

function CreateForm({ agentId }: { agentId: string }) {
  const { sendToAgent } = useWebSocket();
  const [name, setName] = useState('');
  const [driver, setDriver] = useState('bridge');
  const [subnet, setSubnet] = useState('');
  const [attachable, setAttachable] = useState(true);
  const [internal, setInternal] = useState(false);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name) return;
    sendToAgent(agentId, {
      type: 'DockerNetworkCreateRequest',
      payload: {
        name,
        driver,
        subnet: subnet || null,
        attachable,
        internal,
      },
    });
    setName('');
    setSubnet('');
  };

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">⌑</span> CREATE NETWORK
        </div>
      </div>
      <div className="panel-body">
        <form onSubmit={submit}>
          <div className="grid-3" style={{ gap: 12 }}>
            <div className="field">
              <label>name</label>
              <input
                className="input"
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="proxy_net"
                required
              />
            </div>
            <div className="field">
              <label>driver</label>
              <select
                className="select"
                value={driver}
                onChange={(e) => setDriver(e.target.value)}
              >
                <option value="bridge">bridge</option>
                <option value="overlay">overlay (swarm manager)</option>
                <option value="macvlan">macvlan</option>
                <option value="host">host</option>
                <option value="none">none</option>
              </select>
            </div>
            <div className="field">
              <label>subnet (optional)</label>
              <input
                className="input"
                type="text"
                value={subnet}
                onChange={(e) => setSubnet(e.target.value)}
                placeholder="10.20.0.0/24"
              />
            </div>
          </div>
          <div className="row between" style={{ marginTop: 12 }}>
            <div className="row" style={{ gap: 18 }}>
              <label
                className="row"
                style={{ gap: 6, fontSize: 11.5, color: 'var(--fg-1)' }}
              >
                <input
                  type="checkbox"
                  checked={attachable}
                  onChange={(e) => setAttachable(e.target.checked)}
                />
                attachable
              </label>
              <label
                className="row"
                style={{ gap: 6, fontSize: 11.5, color: 'var(--fg-1)' }}
              >
                <input
                  type="checkbox"
                  checked={internal}
                  onChange={(e) => setInternal(e.target.checked)}
                />
                internal
              </label>
            </div>
            <button type="submit" className="btn primary">
              + create
            </button>
          </div>
        </form>
      </div>
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
            <pre className="code" style={{ maxHeight: 'none', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
              {json}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
