'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import {
  DockerContainer,
  DockerListPayload,
  SwarmAction,
  SwarmListPayload,
  SwarmService,
} from '@/lib/types';
import {
  AlertCircleIcon,
  BoxIcon,
  Loader2Icon,
  RefreshCwIcon,
  NetworkIcon,
  Trash2Icon,
  PlayIcon,
} from 'lucide-react';

const REFRESH_MS = 10_000;
const TIMEOUT_MS = 8_000;

type SwarmActionState =
  | null
  | { name: string; action: 'scale' | 'update' | 'remove' };

export default function Containers({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
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
      }
    });

    const request = () => {
      sendToAgent(agentId, { type: 'DockerListRequest' });
    };
    request();
    timeoutRef.current = setTimeout(() => {
      if (waiting) {
        setUnsupported(true);
      }
    }, TIMEOUT_MS);
    const interval = setInterval(request, REFRESH_MS);

    return () => {
      unsub();
      clearInterval(interval);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
    // We deliberately exclude `waiting` from deps — the timeout reads it
    // through the closure but only as a late-firing nudge, and including
    // it would re-create the subscription every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, sendToAgent, onAgentMessage]);

  if (unsupported && !docker) {
    return (
      <div className="flex items-start gap-2 text-sm text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-md px-3 py-2">
        <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
        <span>
          This agent doesn&apos;t expose Docker info. Upgrade with{' '}
          <code className="bg-amber-500/20 px-1 py-0.5 rounded">
            apt install --only-upgrade sys-manager-agent
          </code>
          .
        </span>
      </div>
    );
  }

  if (!docker) {
    return (
      <div className="flex items-center justify-center py-12 text-slate-500">
        <Loader2Icon className="w-5 h-5 animate-spin" />
      </div>
    );
  }

  if (!docker.available) {
    return (
      <div className="flex items-start gap-2 text-sm text-slate-400 bg-slate-900 border border-slate-800 rounded-md px-3 py-3">
        <BoxIcon className="w-4 h-4 mt-0.5 shrink-0" />
        <div>
          <div className="font-medium text-slate-200">Docker unavailable on this host</div>
          {docker.error && <div className="text-xs mt-1 text-slate-500">{docker.error}</div>}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <section>
        <SectionHeader
          icon={<BoxIcon className="w-4 h-4" />}
          title="Containers"
          subtitle={`${docker.containers.length} total · swarm role: ${docker.swarm_role}`}
          onRefresh={() => sendToAgent(agentId, { type: 'DockerListRequest' })}
        />
        {docker.containers.length === 0 ? (
          <Empty>No containers.</Empty>
        ) : (
          <ul className="divide-y divide-slate-800 border border-slate-800 rounded-md overflow-hidden">
            {docker.containers.map((c) => (
              <ContainerRow key={c.id} container={c} />
            ))}
          </ul>
        )}
      </section>

      {docker.swarm_role === 'manager' && (
        <section>
          <SectionHeader
            icon={<NetworkIcon className="w-4 h-4" />}
            title="Swarm services"
            subtitle={
              swarm
                ? `${swarm.services.length} services · ${swarm.nodes.length} nodes`
                : 'Loading…'
            }
            onRefresh={() => sendToAgent(agentId, { type: 'SwarmListRequest' })}
          />
          {!swarm ? (
            <div className="flex items-center justify-center py-6 text-slate-500">
              <Loader2Icon className="w-4 h-4 animate-spin" />
            </div>
          ) : (
            <div className="space-y-4">
              {swarm.services.length === 0 ? (
                <Empty>No swarm services running.</Empty>
              ) : (
                <ul className="divide-y divide-slate-800 border border-slate-800 rounded-md overflow-hidden">
                  {swarm.services.map((s) => (
                    <SwarmServiceRow
                      key={s.id}
                      service={s}
                      pending={pendingAction?.name === s.name ? pendingAction.action : null}
                      disabled={pendingAction !== null && pendingAction.name !== s.name}
                      onAction={(action) => {
                        let payload: SwarmAction;
                        let kindLabel: 'scale' | 'update' | 'remove';
                        if (action === 'forceupdate') {
                          payload = { kind: 'ForceUpdate' };
                          kindLabel = 'update';
                          if (!confirm(`Force-update ${s.name}? This rolls every task even if the spec hasn't changed.`)) {
                            return;
                          }
                        } else if (action === 'remove') {
                          payload = { kind: 'Remove' };
                          kindLabel = 'remove';
                          if (!confirm(`Remove swarm service ${s.name}? This stops every replica and deletes the spec.`)) {
                            return;
                          }
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
                </ul>
              )}
              {actionLog && (
                <details
                  open
                  className={`mt-2 rounded-md border ${
                    actionLog.success
                      ? 'border-emerald-500/30 bg-emerald-500/5'
                      : 'border-red-500/30 bg-red-500/5'
                  }`}
                >
                  <summary
                    className={`cursor-pointer px-3 py-1.5 text-xs font-medium ${
                      actionLog.success ? 'text-emerald-300' : 'text-red-300'
                    }`}
                  >
                    {actionLog.name} · {actionLog.success ? 'success' : 'failed'}
                  </summary>
                  <pre className="text-[11px] bg-slate-950 text-slate-300 px-3 py-2 overflow-x-auto whitespace-pre-wrap max-h-48 border-t border-slate-800">
                    {actionLog.text || '(empty)'}
                  </pre>
                </details>
              )}
              {swarm.nodes.length > 0 && (
                <div className="border border-slate-800 rounded-md overflow-hidden">
                  <div className="px-3 py-1.5 bg-slate-900 border-b border-slate-800 text-xs uppercase tracking-wide text-slate-400">
                    Nodes
                  </div>
                  <ul className="divide-y divide-slate-800">
                    {swarm.nodes.map((n) => (
                      <li
                        key={n.id}
                        className="px-3 py-2 bg-slate-900 flex items-center justify-between gap-3 text-sm"
                      >
                        <div className="min-w-0">
                          <div className="font-medium text-slate-100 truncate">
                            {n.hostname}
                            {n.manager_status && (
                              <span className="ml-2 text-[10px] uppercase tracking-wide text-blue-400">
                                {n.manager_status}
                              </span>
                            )}
                          </div>
                          <div className="text-xs text-slate-500">
                            engine {n.engine_version}
                          </div>
                        </div>
                        <div className="flex items-center gap-3 text-xs shrink-0">
                          <span
                            className={`px-1.5 py-0.5 rounded ${
                              n.status === 'Ready'
                                ? 'bg-emerald-500/20 text-emerald-300'
                                : 'bg-red-500/20 text-red-300'
                            }`}
                          >
                            {n.status}
                          </span>
                          <span className="text-slate-400">{n.availability}</span>
                        </div>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {swarm.error && (
                <div className="flex items-start gap-2 text-xs text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
                  <AlertCircleIcon className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                  <span>{swarm.error}</span>
                </div>
              )}
            </div>
          )}
        </section>
      )}
    </div>
  );
}

function SectionHeader({
  icon,
  title,
  subtitle,
  onRefresh,
}: {
  icon: React.ReactNode;
  title: string;
  subtitle: string;
  onRefresh: () => void;
}) {
  return (
    <div className="flex items-center justify-between mb-2">
      <div className="flex items-center gap-2">
        <span className="text-slate-400">{icon}</span>
        <h3 className="text-sm font-semibold text-slate-100">{title}</h3>
        <span className="text-xs text-slate-500">· {subtitle}</span>
      </div>
      <button
        type="button"
        onClick={onRefresh}
        className="text-xs flex items-center gap-1 px-2 py-1 rounded-md text-slate-400 hover:text-slate-100 hover:bg-slate-800 transition-colors"
      >
        <RefreshCwIcon className="w-3.5 h-3.5" />
        Refresh
      </button>
    </div>
  );
}

function SwarmServiceRow({
  service,
  pending,
  disabled,
  onAction,
}: {
  service: SwarmService;
  pending: 'scale' | 'update' | 'remove' | null;
  disabled: boolean;
  onAction: (action: 'scale' | 'forceupdate' | 'remove') => void;
}) {
  return (
    <li className="px-3 py-2 bg-slate-900">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="font-medium text-slate-100 text-sm truncate">{service.name}</div>
          <div className="text-xs text-slate-500 truncate" title={service.image}>
            {service.image}
          </div>
        </div>
        <div className="flex items-center gap-3 text-xs text-slate-400 shrink-0">
          <span>{service.mode}</span>
          <span className="font-mono text-slate-200">{service.replicas}</span>
          <button
            type="button"
            disabled={disabled}
            onClick={() => onAction('scale')}
            className="px-2 py-0.5 rounded border border-slate-700 hover:bg-slate-800 disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center gap-1"
            title="Scale replicas"
          >
            {pending === 'scale' ? <Loader2Icon className="w-3 h-3 animate-spin" /> : 'Scale'}
          </button>
          <button
            type="button"
            disabled={disabled}
            onClick={() => onAction('forceupdate')}
            className="p-1 rounded text-slate-400 hover:text-blue-300 hover:bg-blue-500/10 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            title="Force update"
          >
            {pending === 'update' ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <PlayIcon className="w-3.5 h-3.5" />
            )}
          </button>
          <button
            type="button"
            disabled={disabled}
            onClick={() => onAction('remove')}
            className="p-1 rounded text-slate-400 hover:text-red-300 hover:bg-red-500/10 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            title="Remove service"
          >
            {pending === 'remove' ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Trash2Icon className="w-3.5 h-3.5" />
            )}
          </button>
        </div>
      </div>
      {service.ports && (
        <div className="text-[11px] text-slate-500 truncate mt-1" title={service.ports}>
          {service.ports}
        </div>
      )}
    </li>
  );
}

function ContainerRow({ container }: { container: DockerContainer }) {
  const stateClasses =
    container.state === 'running'
      ? 'bg-emerald-500/20 text-emerald-300'
      : container.state === 'exited' || container.state === 'dead'
        ? 'bg-red-500/20 text-red-300'
        : 'bg-slate-800 text-slate-300';
  return (
    <li className="px-3 py-2 bg-slate-900 flex items-center justify-between gap-3">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="font-medium text-slate-100 text-sm truncate" title={container.names}>
            {container.names || container.id}
          </span>
          <code className="text-[11px] text-slate-500">{container.id.slice(0, 8)}</code>
        </div>
        <div className="text-xs text-slate-500 truncate" title={container.image}>
          {container.image}
        </div>
        {container.ports && (
          <div className="text-[11px] text-slate-500 truncate mt-0.5" title={container.ports}>
            {container.ports}
          </div>
        )}
      </div>
      <div className="flex flex-col items-end gap-1 text-xs shrink-0">
        <span className={`px-1.5 py-0.5 rounded uppercase tracking-wide font-medium text-[10px] ${stateClasses}`}>
          {container.state || '—'}
        </span>
        <span className="text-slate-500 text-[11px] truncate max-w-[14rem]" title={container.status}>
          {container.status}
        </span>
      </div>
    </li>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-sm text-slate-500 border border-dashed border-slate-800 rounded-md px-3 py-6 text-center">
      {children}
    </div>
  );
}
