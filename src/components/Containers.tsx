'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import {
  DockerContainer,
  DockerContainerAction,
  DockerListPayload,
  SwarmAction,
  SwarmListPayload,
  SwarmService,
} from '@/lib/types';
import { Loader2Icon } from 'lucide-react';
import LogViewer from './LogViewer';
import SwarmServiceDrawer from './SwarmServiceDrawer';
import ContainerExecModal from './ContainerExecModal';
import { useUi } from './providers/UiProvider';
import { useCanWrite } from './providers/SessionProvider';

const REFRESH_MS = 10_000;
const TIMEOUT_MS = 8_000;

type SwarmActionState = null | { name: string; action: 'scale' | 'update' | 'remove' };
type ContainerActionState = { id: string; action: DockerContainerAction };

export default function Containers({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const canWrite = useCanWrite();
  const [docker, setDocker] = useState<DockerListPayload | null>(null);
  const [swarm, setSwarm] = useState<SwarmListPayload | null>(null);
  const [waiting, setWaiting] = useState(true);
  const [unsupported, setUnsupported] = useState(false);
  const [pendingAction, setPendingAction] = useState<SwarmActionState>(null);
  const [actionLog, setActionLog] = useState<{
    name: string;
    success: boolean;
    text: string;
  } | null>(null);
  const [containerAction, setContainerAction] = useState<ContainerActionState | null>(null);
  const [containerActionLog, setContainerActionLog] = useState<{
    id: string;
    success: boolean;
    text: string;
  } | null>(null);
  const [logViewer, setLogViewer] = useState<{ id: string; name: string } | null>(null);
  const [serviceDrawer, setServiceDrawer] = useState<string | null>(null);
  const [execModal, setExecModal] = useState<{ id: string; name: string; shell: string } | null>(null);
  const [filter, setFilter] = useState<'all' | 'running' | 'stopped'>('all');
  const [search, setSearch] = useState('');
  const { confirm } = useUi();
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setDocker(null);
    setSwarm(null);
    setWaiting(true);
    setUnsupported(false);
    setPendingAction(null);
    setActionLog(null);

    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'DockerListResponse') {
        if (timeoutRef.current) {
          clearTimeout(timeoutRef.current);
          timeoutRef.current = null;
        }
        setUnsupported(false);
        setWaiting(false);
        setDocker(msg.payload);
        if (msg.payload.swarm_role === 'manager') {
          sendToAgent(agentId, { type: 'SwarmListRequest' });
        } else {
          setSwarm(null);
        }
      } else if (msg.type === 'SwarmListResponse') {
        setSwarm(msg.payload);
      } else if (msg.type === 'SwarmServiceActionResponse') {
        setPendingAction(null);
        setActionLog({
          name: msg.payload.name,
          success: msg.payload.success,
          text: msg.payload.log || (msg.payload.error ?? ''),
        });
        sendToAgent(agentId, { type: 'SwarmListRequest' });
      } else if (msg.type === 'DockerContainerActionResponse') {
        setContainerAction(null);
        setContainerActionLog({
          id: msg.payload.id,
          success: msg.payload.success,
          text: msg.payload.log || (msg.payload.error ?? ''),
        });
        sendToAgent(agentId, { type: 'DockerListRequest' });
      }
    });

    const request = () => sendToAgent(agentId, { type: 'DockerListRequest' });
    request();
    timeoutRef.current = setTimeout(() => {
      if (waiting) setUnsupported(true);
    }, TIMEOUT_MS);
    const interval = setInterval(request, REFRESH_MS);

    return () => {
      unsub();
      clearInterval(interval);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, sendToAgent, onAgentMessage]);

  if (unsupported && !docker) {
    return (
      <div className="pane">
        <div
          style={{
            padding: 12,
            background: 'var(--warn-bg)',
            border: '1px solid var(--warn-bd)',
            borderRadius: 'var(--r)',
            color: 'var(--warn)',
            fontFamily: 'var(--mono)',
            fontSize: 12,
          }}
        >
          ⚠ This agent doesn&apos;t expose Docker info. Upgrade with{' '}
          <code style={{ background: 'rgba(0,0,0,0.2)', padding: '0 4px', borderRadius: 2 }}>
            apt install --only-upgrade sys-manager-agent
          </code>
          .
        </div>
      </div>
    );
  }

  if (!docker) {
    return (
      <div className="pane">
        <div className="empty">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      </div>
    );
  }

  if (!docker.available) {
    return (
      <div className="pane">
        <div className="panel">
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">▢</span> CONTAINERS
            </div>
          </div>
          <div className="panel-body">
            <div style={{ color: 'var(--fg-1)', marginBottom: 4 }}>
              Docker unavailable on this host
            </div>
            {docker.error && (
              <div className="muted" style={{ fontSize: 11 }}>
                {docker.error}
              </div>
            )}
          </div>
        </div>
      </div>
    );
  }

  const q = search.trim().toLowerCase();
  const containers = docker.containers.filter((c) => {
    if (filter === 'running' && c.state !== 'running') return false;
    if (filter === 'stopped' && c.state === 'running') return false;
    if (q) {
      return (
        c.names.toLowerCase().includes(q) ||
        c.image.toLowerCase().includes(q) ||
        c.id.toLowerCase().includes(q)
      );
    }
    return true;
  });

  const runningCount = docker.containers.filter((c) => c.state === 'running').length;

  return (
    <div className="pane">
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▢</span> CONTAINERS
            <span className="meta">
              {docker.containers.length} total · {runningCount} running
            </span>
          </div>
          <div className="panel-actions">
            <div className="search-input" style={{ width: 220 }}>
              <span style={{ color: 'var(--accent)' }}>⌕</span>
              <input
                placeholder="name, image, id…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <div className="seg">
              <button className={filter === 'all' ? 'on' : ''} onClick={() => setFilter('all')}>
                all
              </button>
              <button
                className={filter === 'running' ? 'on' : ''}
                onClick={() => setFilter('running')}
              >
                running
              </button>
              <button
                className={filter === 'stopped' ? 'on' : ''}
                onClick={() => setFilter('stopped')}
              >
                stopped
              </button>
            </div>
            <button
              className="btn"
              onClick={() => sendToAgent(agentId, { type: 'DockerListRequest' })}
            >
              ↻
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {containers.length === 0 ? (
            <div className="empty">No containers match.</div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ width: 110 }}>STATE</th>
                  <th>NAME</th>
                  <th>IMAGE</th>
                  <th style={{ width: 110 }}>ID</th>
                  <th>PORTS</th>
                  <th style={{ width: 220 }}>STATUS</th>
                  <th style={{ width: 200 }} />
                </tr>
              </thead>
              <tbody>
                {containers.map((c) => (
                  <ContainerRow
                    key={c.id}
                    container={c}
                    canWrite={canWrite}
                    pending={containerAction?.id === c.id ? containerAction.action : null}
                    disabled={containerAction !== null && containerAction.id !== c.id}
                    onAction={async (action) => {
                      if (action === 'remove') {
                        const ok = await confirm({
                          title: `Remove container ${c.names || c.id.slice(0, 12)}?`,
                          description: 'Running containers will be force-killed.',
                          confirmLabel: 'Remove',
                          destructive: true,
                        });
                        if (!ok) return;
                      }
                      setContainerAction({ id: c.id, action });
                      setContainerActionLog(null);
                      sendToAgent(agentId, {
                        type: 'DockerContainerActionRequest',
                        payload: { id: c.id, action },
                      });
                    }}
                    onShowLogs={() =>
                      setLogViewer({ id: c.id, name: c.names || c.id.slice(0, 12) })
                    }
                    onShowShell={() =>
                      setExecModal({
                        id: c.id,
                        name: c.names || c.id.slice(0, 12),
                        shell: 'sh',
                      })
                    }
                  />
                ))}
              </tbody>
            </table>
          )}
          {containerActionLog && (
            <div
              style={{
                padding: '6px 12px',
                borderTop: '1px solid var(--line)',
                background: containerActionLog.success ? 'var(--accent-bg)' : 'var(--err-bg)',
                color: containerActionLog.success ? 'var(--accent)' : 'var(--err)',
                fontFamily: 'var(--mono)',
                fontSize: 11,
              }}
            >
              {containerActionLog.id.slice(0, 12)} ·{' '}
              {containerActionLog.success ? 'success' : 'failed'}
              {containerActionLog.text && (
                <pre className="code" style={{ marginTop: 6, fontSize: 10.5, maxHeight: 160 }}>
                  {containerActionLog.text}
                </pre>
              )}
            </div>
          )}
        </div>
      </div>

      {logViewer && (
        <LogViewer
          agentId={agentId}
          containerId={logViewer.id}
          containerName={logViewer.name}
          onClose={() => setLogViewer(null)}
        />
      )}

      {serviceDrawer && (
        <SwarmServiceDrawer
          agentId={agentId}
          serviceName={serviceDrawer}
          onClose={() => setServiceDrawer(null)}
        />
      )}

      {execModal && (
        <ContainerExecModal
          agentId={agentId}
          containerId={execModal.id}
          containerName={execModal.name}
          shell={execModal.shell}
          onClose={() => setExecModal(null)}
        />
      )}

      {docker.swarm_role === 'manager' && (
        <div className="panel">
          <div className="panel-head">
            <div className="panel-title">
              <span className="ico">⊞</span> SWARM SERVICES
              <span className="meta">
                {swarm
                  ? `${swarm.services.length} services · ${swarm.nodes.length} nodes`
                  : 'loading…'}
              </span>
            </div>
            <div className="panel-actions">
              <button
                className="btn"
                onClick={() => sendToAgent(agentId, { type: 'SwarmListRequest' })}
              >
                ↻
              </button>
            </div>
          </div>
          <div className="panel-body flush">
            {!swarm ? (
              <div className="empty">
                <Loader2Icon className="w-4 h-4 animate-spin" />
              </div>
            ) : swarm.services.length === 0 ? (
              <div className="empty">No swarm services running.</div>
            ) : (
              <table className="tbl">
                <thead>
                  <tr>
                    <th style={{ width: 100 }}>REPLICAS</th>
                    <th>NAME</th>
                    <th>IMAGE</th>
                    <th style={{ width: 110 }}>ID</th>
                    <th style={{ width: 100 }}>MODE</th>
                    <th>PORTS</th>
                    <th style={{ width: 220 }} />
                  </tr>
                </thead>
                <tbody>
                  {swarm.services.map((s) => (
                    <SwarmServiceRow
                      key={s.id}
                      service={s}
                      canWrite={canWrite}
                      pending={pendingAction?.name === s.name ? pendingAction.action : null}
                      disabled={pendingAction !== null && pendingAction.name !== s.name}
                      onShowDetail={() => setServiceDrawer(s.name)}
                      onAction={async (action) => {
                        let payload: SwarmAction;
                        let kindLabel: 'scale' | 'update' | 'remove';
                        if (action === 'forceupdate') {
                          payload = { kind: 'ForceUpdate' };
                          kindLabel = 'update';
                          const ok = await confirm({
                            title: `Force-update ${s.name}?`,
                            description: "This rolls every task even if the spec hasn't changed.",
                            confirmLabel: 'Force update',
                          });
                          if (!ok) return;
                        } else if (action === 'remove') {
                          payload = { kind: 'Remove' };
                          kindLabel = 'remove';
                          const ok = await confirm({
                            title: `Remove ${s.name}?`,
                            description: 'Stops every replica and deletes the service spec.',
                            confirmLabel: 'Remove',
                            destructive: true,
                          });
                          if (!ok) return;
                        } else {
                          const replicas = window.prompt(
                            `Scale ${s.name} to how many replicas?`,
                            s.replicas.split('/')[1] ?? '1',
                          );
                          if (!replicas) return;
                          const n = Number.parseInt(replicas, 10);
                          if (!Number.isFinite(n) || n < 0) return;
                          payload = { kind: 'Scale', value: n };
                          kindLabel = 'scale';
                        }
                        setPendingAction({ name: s.name, action: kindLabel });
                        setActionLog(null);
                        sendToAgent(agentId, {
                          type: 'SwarmServiceActionRequest',
                          payload: { name: s.name, action: payload },
                        });
                      }}
                    />
                  ))}
                </tbody>
              </table>
            )}
            {actionLog && (
              <div
                style={{
                  padding: '6px 12px',
                  borderTop: '1px solid var(--line)',
                  background: actionLog.success ? 'var(--accent-bg)' : 'var(--err-bg)',
                  color: actionLog.success ? 'var(--accent)' : 'var(--err)',
                  fontFamily: 'var(--mono)',
                  fontSize: 11,
                }}
              >
                {actionLog.name} · {actionLog.success ? 'success' : 'failed'}
                {actionLog.text && (
                  <pre className="code" style={{ marginTop: 6, fontSize: 10.5, maxHeight: 160 }}>
                    {actionLog.text}
                  </pre>
                )}
              </div>
            )}
            {swarm?.nodes && swarm.nodes.length > 0 && (
              <table className="tbl" style={{ borderTop: '1px solid var(--line)' }}>
                <thead>
                  <tr>
                    <th colSpan={4}>NODES</th>
                  </tr>
                </thead>
                <tbody>
                  {swarm.nodes.map((n) => (
                    <tr key={n.id}>
                      <td className="mono" style={{ color: 'var(--fg)' }}>
                        {n.hostname}
                        {n.manager_status && (
                          <span
                            className="chip"
                            style={{
                              marginLeft: 8,
                              color: 'var(--info)',
                              borderColor: 'var(--info-bd)',
                            }}
                          >
                            {n.manager_status}
                          </span>
                        )}
                      </td>
                      <td className="mono muted">engine {n.engine_version}</td>
                      <td>
                        <span className={`status ${n.status === 'Ready' ? 'ok' : 'err-c'}`}>
                          <span className="dot" />
                          {n.status}
                        </span>
                      </td>
                      <td className="mono">{n.availability}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            {swarm?.error && (
              <div
                style={{
                  padding: '6px 12px',
                  borderTop: '1px solid var(--line)',
                  background: 'var(--err-bg)',
                  color: 'var(--err)',
                  fontFamily: 'var(--mono)',
                  fontSize: 11,
                }}
              >
                {swarm.error}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function ContainerRow({
  container,
  canWrite,
  pending,
  disabled,
  onAction,
  onShowLogs,
  onShowShell,
}: {
  container: DockerContainer;
  canWrite: boolean;
  pending: DockerContainerAction | null;
  disabled: boolean;
  onAction: (action: DockerContainerAction) => void;
  onShowLogs: () => void;
  onShowShell: () => void;
}) {
  const isRunning = container.state === 'running';
  const cls =
    isRunning
      ? 'ok'
      : container.state === 'restarting'
        ? 'warn-c'
        : container.state === 'exited' || container.state === 'dead'
          ? 'err-c'
          : 'muted';
  return (
    <tr>
      <td>
        <span className={`status ${cls}`}>
          <span className="dot" />
          {container.state}
        </span>
      </td>
      <td>
        <span className="name-cell">
          <span className="icn">▢</span>
          <span className="mono" style={{ color: 'var(--fg)' }}>
            {container.names || container.id}
          </span>
        </span>
      </td>
      <td className="mono muted">{container.image}</td>
      <td className="mono muted">{container.id.slice(0, 12)}</td>
      <td className="mono">{container.ports || '—'}</td>
      <td className="mono muted">{container.status}</td>
      <td className="actions">
        <button
          className="btn sm icon"
          title="Logs"
          onClick={onShowLogs}
          disabled={disabled}
        >
          ≡
        </button>
        <button
          className="btn sm icon"
          title={!canWrite ? 'viewer role: read-only' : 'Shell'}
          onClick={onShowShell}
          disabled={disabled || !isRunning || !canWrite}
        >
          ›_
        </button>
        <button
          className="btn sm icon"
          title={!canWrite ? 'viewer role: read-only' : isRunning ? 'Stop' : 'Start'}
          disabled={disabled || !canWrite}
          onClick={() => onAction(isRunning ? 'stop' : 'start')}
        >
          {pending === 'start' || pending === 'stop' ? '…' : isRunning ? '■' : '▶'}
        </button>
        <button
          className="btn sm icon"
          title={!canWrite ? 'viewer role: read-only' : 'Restart'}
          disabled={disabled || !canWrite}
          onClick={() => onAction('restart')}
        >
          {pending === 'restart' ? '…' : '↻'}
        </button>
        <button
          className="btn sm icon danger"
          title={!canWrite ? 'viewer role: read-only' : 'Remove'}
          disabled={disabled || !canWrite}
          onClick={() => onAction('remove')}
        >
          {pending === 'remove' ? '…' : '×'}
        </button>
      </td>
    </tr>
  );
}

function SwarmServiceRow({
  service,
  canWrite,
  pending,
  disabled,
  onAction,
  onShowDetail,
}: {
  service: SwarmService;
  canWrite: boolean;
  pending: 'scale' | 'update' | 'remove' | null;
  disabled: boolean;
  onAction: (action: 'scale' | 'forceupdate' | 'remove') => void;
  onShowDetail: () => void;
}) {
  const [ok, want] = service.replicas.split('/').map((n) => parseInt(n, 10));
  const cls = ok === want ? 'ok' : 'warn-c';
  return (
    <tr>
      <td>
        <span className={`status ${cls}`}>
          <span className="dot" />
          {service.replicas}
        </span>
      </td>
      <td className="mono" style={{ color: 'var(--fg)', cursor: 'pointer' }}>
        <button
          type="button"
          onClick={onShowDetail}
          style={{
            background: 'transparent',
            border: 0,
            color: 'inherit',
            font: 'inherit',
            padding: 0,
            cursor: 'pointer',
            textAlign: 'left',
          }}
        >
          {service.name}
        </button>
      </td>
      <td className="mono muted">{service.image}</td>
      <td className="mono muted">{service.id.slice(0, 12)}</td>
      <td className="mono">{service.mode}</td>
      <td className="mono">{service.ports || '—'}</td>
      <td className="actions">
        <button
          className="btn sm"
          disabled={disabled || !canWrite}
          title={!canWrite ? 'viewer role: read-only' : undefined}
          onClick={() => onAction('scale')}
        >
          {pending === 'scale' ? '…' : 'scale'}
        </button>
        <button
          className="btn sm icon"
          title={!canWrite ? 'viewer role: read-only' : 'Force update'}
          disabled={disabled || !canWrite}
          onClick={() => onAction('forceupdate')}
        >
          {pending === 'update' ? '…' : '↻'}
        </button>
        <button
          className="btn sm icon danger"
          title={!canWrite ? 'viewer role: read-only' : 'Remove'}
          disabled={disabled || !canWrite}
          onClick={() => onAction('remove')}
        >
          {pending === 'remove' ? '…' : '×'}
        </button>
      </td>
    </tr>
  );
}
