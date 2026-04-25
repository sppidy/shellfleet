'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import type { SwarmStackRow, SwarmService, SwarmTask } from '@/lib/types';
import {
  LayersIcon,
  RefreshCwIcon,
  Trash2Icon,
  Loader2Icon,
  AlertCircleIcon,
  EyeIcon,
} from 'lucide-react';

const REFRESH_MS = 15_000;

export default function SwarmStacks({ agentId }: { agentId: string }) {
  const ui = useUi();
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [stacks, setStacks] = useState<SwarmStackRow[] | null>(null);
  const [isManager, setIsManager] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [inspect, setInspect] = useState<{
    name: string;
    services: SwarmService[] | null;
    tasks: SwarmTask[] | null;
    error: string | null;
  } | null>(null);
  const reqTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(() => {
    setError(null);
    sendToAgent(agentId, { type: 'SwarmStackListRequest' });
    if (reqTimeoutRef.current) clearTimeout(reqTimeoutRef.current);
    reqTimeoutRef.current = setTimeout(() => setError("agent didn't respond"), 8_000);
  }, [agentId, sendToAgent]);

  useEffect(() => {
    setStacks(null);
    setIsManager(null);
    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'SwarmStackListResponse') {
        if (reqTimeoutRef.current) {
          clearTimeout(reqTimeoutRef.current);
          reqTimeoutRef.current = null;
        }
        setIsManager(msg.payload.is_manager);
        if (!msg.payload.is_manager) {
          setStacks([]);
          setError(null);
          return;
        }
        if (!msg.payload.available) {
          setError(msg.payload.error ?? 'docker not available');
          setStacks([]);
          return;
        }
        setError(msg.payload.error);
        setStacks(msg.payload.stacks);
      } else if (msg.type === 'SwarmStackInspectResponse') {
        if (msg.payload.success) {
          setInspect({
            name: msg.payload.name,
            services: msg.payload.services,
            tasks: msg.payload.tasks,
            error: null,
          });
        } else {
          setInspect({
            name: msg.payload.name,
            services: [],
            tasks: [],
            error: msg.payload.error ?? 'inspect failed',
          });
        }
      } else if (msg.type === 'SwarmStackRemoveResponse') {
        setRemoving(null);
        if (msg.payload.success) {
          ui.toast('success', `Removed stack ${msg.payload.name}`);
        } else {
          ui.toast('error', msg.payload.error ?? 'remove failed');
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

  const remove = async (s: SwarmStackRow) => {
    const ok = await ui.confirm({
      title: `Remove stack "${s.name}"?`,
      description:
        'Tears down every service, task, and stack-managed network. Volumes referenced by services are kept.',
      destructive: true,
      confirmLabel: 'Remove',
    });
    if (!ok) return;
    setRemoving(s.name);
    sendToAgent(agentId, { type: 'SwarmStackRemoveRequest', payload: { name: s.name } });
  };

  const openInspect = (s: SwarmStackRow) => {
    setInspect({ name: s.name, services: null, tasks: null, error: null });
    sendToAgent(agentId, { type: 'SwarmStackInspectRequest', payload: { name: s.name } });
  };

  if (isManager === false) {
    return (
      <div className="flex items-start gap-2 text-sm text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-md px-3 py-2">
        <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
        <span>This host isn't a swarm manager. Stack management is manager-only.</span>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <LayersIcon className="w-5 h-5 text-slate-400" />
          <h2 className="text-base font-semibold">Stacks</h2>
          <span className="text-xs text-slate-500">
            {stacks === null ? 'loading…' : `· ${stacks.length}`}
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

      {error && (
        <div className="flex items-start gap-2 text-sm text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {stacks === null ? (
        <div className="flex items-center justify-center py-12 text-slate-500">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      ) : stacks.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          No stacks deployed.
        </div>
      ) : (
        <div className="rounded-md border border-slate-800 overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-slate-900/60 text-[11px] uppercase tracking-wide text-slate-500">
              <tr>
                <th className="text-left px-3 py-2 font-medium">Name</th>
                <th className="text-right px-3 py-2 font-medium">Services</th>
                <th className="text-left px-3 py-2 font-medium">Orchestrator</th>
                <th className="px-3 py-2"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-800">
              {stacks.map((s) => (
                <tr key={s.name} className="bg-slate-900/30">
                  <td className="px-3 py-2 font-medium text-slate-200">{s.name}</td>
                  <td className="px-3 py-2 text-right text-slate-400">{s.services}</td>
                  <td className="px-3 py-2 text-slate-400">{s.orchestrator}</td>
                  <td className="px-3 py-2 text-right">
                    <div className="flex items-center justify-end gap-1">
                      <button
                        type="button"
                        onClick={() => openInspect(s)}
                        title="Inspect"
                        className="p-1.5 rounded text-slate-400 hover:text-slate-100 hover:bg-slate-800"
                      >
                        <EyeIcon className="w-4 h-4" />
                      </button>
                      <button
                        type="button"
                        onClick={() => remove(s)}
                        disabled={removing === s.name}
                        title="Remove"
                        className="p-1.5 rounded text-slate-400 hover:text-red-300 hover:bg-slate-800 disabled:opacity-50"
                      >
                        {removing === s.name ? (
                          <Loader2Icon className="w-4 h-4 animate-spin" />
                        ) : (
                          <Trash2Icon className="w-4 h-4" />
                        )}
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {inspect && <InspectModal inspect={inspect} onClose={() => setInspect(null)} />}
    </div>
  );
}

function InspectModal({
  inspect,
  onClose,
}: {
  inspect: {
    name: string;
    services: SwarmService[] | null;
    tasks: SwarmTask[] | null;
    error: string | null;
  };
  onClose: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 bg-slate-950/70 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="bg-slate-900 border border-slate-800 rounded-lg shadow-2xl max-w-4xl w-full max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="p-4 border-b border-slate-800 flex items-center justify-between">
          <h3 className="text-base font-semibold text-slate-100 break-all">
            Stack {inspect.name}
          </h3>
          <button type="button" onClick={onClose} className="text-slate-400 hover:text-slate-100">
            ×
          </button>
        </div>
        <div className="flex-1 overflow-auto p-4 space-y-4">
          {inspect.error && (
            <div className="flex items-start gap-2 text-sm text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
              <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
              <span>{inspect.error}</span>
            </div>
          )}
          {inspect.services === null ? (
            <div className="flex items-center justify-center py-8 text-slate-500">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : (
            <>
              <div>
                <h4 className="text-xs uppercase tracking-wide text-slate-500 mb-2">Services</h4>
                {inspect.services.length === 0 ? (
                  <p className="text-xs text-slate-500 italic">none</p>
                ) : (
                  <div className="rounded-md border border-slate-800 overflow-hidden">
                    <table className="w-full text-xs">
                      <thead className="bg-slate-900/60 text-slate-500">
                        <tr>
                          <th className="text-left px-2 py-1">Name</th>
                          <th className="text-left px-2 py-1">Mode</th>
                          <th className="text-left px-2 py-1">Replicas</th>
                          <th className="text-left px-2 py-1">Image</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-slate-800">
                        {inspect.services.map((s) => (
                          <tr key={s.id} className="bg-slate-900/30">
                            <td className="px-2 py-1 text-slate-200">{s.name}</td>
                            <td className="px-2 py-1 text-slate-400">{s.mode}</td>
                            <td className="px-2 py-1 text-slate-400">{s.replicas}</td>
                            <td className="px-2 py-1 font-mono text-slate-500 break-all">
                              {s.image}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
              <div>
                <h4 className="text-xs uppercase tracking-wide text-slate-500 mb-2">Tasks</h4>
                {!inspect.tasks || inspect.tasks.length === 0 ? (
                  <p className="text-xs text-slate-500 italic">none</p>
                ) : (
                  <div className="rounded-md border border-slate-800 overflow-hidden">
                    <table className="w-full text-xs">
                      <thead className="bg-slate-900/60 text-slate-500">
                        <tr>
                          <th className="text-left px-2 py-1">Name</th>
                          <th className="text-left px-2 py-1">Node</th>
                          <th className="text-left px-2 py-1">Desired</th>
                          <th className="text-left px-2 py-1">Current</th>
                          <th className="text-left px-2 py-1">Error</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-slate-800">
                        {inspect.tasks.map((t) => (
                          <tr key={t.id} className="bg-slate-900/30">
                            <td className="px-2 py-1 text-slate-200 break-all">{t.name}</td>
                            <td className="px-2 py-1 text-slate-400">{t.node}</td>
                            <td className="px-2 py-1 text-slate-400">{t.desired_state}</td>
                            <td
                              className={`px-2 py-1 ${
                                t.current_state.includes('Failed')
                                  ? 'text-red-300'
                                  : t.current_state.includes('Running')
                                    ? 'text-emerald-300'
                                    : 'text-slate-400'
                              }`}
                            >
                              {t.current_state}
                            </td>
                            <td className="px-2 py-1 text-red-300 break-all">{t.error}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
