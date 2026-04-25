'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { DockerContainer, DockerListPayload, SwarmListPayload } from '@/lib/types';
import {
  AlertCircleIcon,
  BoxIcon,
  Loader2Icon,
  RefreshCwIcon,
  NetworkIcon,
} from 'lucide-react';

const REFRESH_MS = 10_000;
const TIMEOUT_MS = 8_000;

export default function Containers({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [docker, setDocker] = useState<DockerListPayload | null>(null);
  const [swarm, setSwarm] = useState<SwarmListPayload | null>(null);
  const [waiting, setWaiting] = useState(true);
  const [unsupported, setUnsupported] = useState(false);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setDocker(null);
    setSwarm(null);
    setWaiting(true);
    setUnsupported(false);

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
                    <li key={s.id} className="px-3 py-2 bg-slate-900">
                      <div className="flex items-center justify-between gap-3">
                        <div className="min-w-0">
                          <div className="font-medium text-slate-100 text-sm truncate">{s.name}</div>
                          <div className="text-xs text-slate-500 truncate" title={s.image}>
                            {s.image}
                          </div>
                        </div>
                        <div className="flex items-center gap-3 text-xs text-slate-400 shrink-0">
                          <span>{s.mode}</span>
                          <span className="font-mono text-slate-300">{s.replicas}</span>
                        </div>
                      </div>
                      {s.ports && (
                        <div className="text-[11px] text-slate-500 truncate mt-1" title={s.ports}>
                          {s.ports}
                        </div>
                      )}
                    </li>
                  ))}
                </ul>
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
