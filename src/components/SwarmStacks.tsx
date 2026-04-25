'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import type { SwarmStackRow, SwarmService, SwarmTask } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

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
          ⚠ This host isn&apos;t a swarm manager. Stack management is manager-only.
        </div>
      </div>
    );
  }

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
            <span className="ico">⊞</span> SWARM STACKS
            <span className="meta">
              {stacks === null ? 'loading…' : `manager · ${stacks.length} stacks`}
            </span>
          </div>
          <div className="panel-actions">
            <button className="btn" onClick={refresh}>↻</button>
          </div>
        </div>
        <div className="panel-body flush">
          {stacks === null ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : stacks.length === 0 ? (
            <div className="empty">No stacks deployed.</div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>NAME</th>
                  <th className="right">SERVICES</th>
                  <th>ORCHESTRATOR</th>
                  <th style={{ width: 200 }} />
                </tr>
              </thead>
              <tbody>
                {stacks.map((s) => (
                  <tr key={s.name}>
                    <td className="mono" style={{ color: 'var(--fg)' }}>
                      {s.name}
                    </td>
                    <td className="right mono">{s.services}</td>
                    <td className="mono">{s.orchestrator}</td>
                    <td className="actions">
                      <button className="btn sm" onClick={() => openInspect(s)}>
                        inspect
                      </button>
                      <button
                        className="btn sm icon danger"
                        title="Remove"
                        disabled={removing === s.name}
                        onClick={() => remove(s)}
                      >
                        {removing === s.name ? '…' : '×'}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

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
      className="modal-overlay"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        className="modal"
        style={{ width: 'min(900px, 95vw)', maxHeight: '85vh', display: 'flex', flexDirection: 'column' }}
      >
        <div className="panel-head">
          <div className="panel-title">Stack {inspect.name}</div>
          <button className="icon-btn" onClick={onClose}>
            ×
          </button>
        </div>
        <div style={{ flex: 1, overflow: 'auto', padding: 12, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {inspect.error && (
            <div
              style={{
                padding: 8,
                background: 'var(--err-bg)',
                border: '1px solid var(--err-bd)',
                borderRadius: 'var(--r)',
                color: 'var(--err)',
                fontSize: 11,
              }}
            >
              {inspect.error}
            </div>
          )}
          {inspect.services === null ? (
            <div className="empty">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : (
            <>
              <div className="panel">
                <div className="panel-head">
                  <div className="panel-title">SERVICES</div>
                </div>
                <div className="panel-body flush">
                  {inspect.services.length === 0 ? (
                    <div className="empty">none</div>
                  ) : (
                    <table className="tbl">
                      <thead>
                        <tr>
                          <th>NAME</th>
                          <th>MODE</th>
                          <th>REPLICAS</th>
                          <th>IMAGE</th>
                        </tr>
                      </thead>
                      <tbody>
                        {inspect.services.map((s) => (
                          <tr key={s.id}>
                            <td className="mono" style={{ color: 'var(--fg)' }}>
                              {s.name}
                            </td>
                            <td className="mono">{s.mode}</td>
                            <td className="mono">{s.replicas}</td>
                            <td className="mono muted">{s.image}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>
              </div>

              <div className="panel">
                <div className="panel-head">
                  <div className="panel-title">TASKS</div>
                </div>
                <div className="panel-body flush">
                  {!inspect.tasks || inspect.tasks.length === 0 ? (
                    <div className="empty">none</div>
                  ) : (
                    <table className="tbl">
                      <thead>
                        <tr>
                          <th>NAME</th>
                          <th>NODE</th>
                          <th>DESIRED</th>
                          <th>CURRENT</th>
                          <th>ERROR</th>
                        </tr>
                      </thead>
                      <tbody>
                        {inspect.tasks.map((t) => (
                          <tr key={t.id}>
                            <td className="mono" style={{ color: 'var(--fg)' }}>
                              {t.name}
                            </td>
                            <td className="mono">{t.node}</td>
                            <td className="mono">{t.desired_state}</td>
                            <td
                              className={`mono ${
                                t.current_state.includes('Failed')
                                  ? 'err-c'
                                  : t.current_state.includes('Running')
                                    ? 'ok'
                                    : 'muted'
                              }`}
                            >
                              {t.current_state}
                            </td>
                            <td className="mono err-c">{t.error}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
