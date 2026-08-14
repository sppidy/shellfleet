'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useCoreFleet } from './providers/CoreFleetProvider';
import { useCanWrite } from './providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';
import JournalLogViewer from './JournalLogViewer';

type Action = 'start' | 'stop' | 'restart';
type Toast = { kind: 'success' | 'error'; text: string };
const CONTROL_TIMEOUT_MS = 15_000;

export default function ServiceList({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage, isConnected, liveAgents } = useWebSocket();
  const { snapshots, liveStatus, loading, refresh } = useCoreFleet();
  const canWrite = useCanWrite();
  const [filter, setFilter] = useState('');
  const [stateFilter, setStateFilter] = useState<'all' | 'active' | 'failed' | 'inactive'>('all');
  const [pending, setPending] = useState<Record<string, Action>>({});
  const [toast, setToast] = useState<Toast | null>(null);
  const [logUnit, setLogUnit] = useState<string | null>(null);
  const pendingTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  const snapshot = snapshots[agentId];
  const services = snapshot?.services ?? null;
  const hasDurableServices = services !== null;
  const isAgentOnline = snapshot?.status === 'online';
  const dataIsLive = isAgentOnline && liveStatus === 'live';
  const controlsAreLive = isConnected && liveAgents.includes(agentId);

  const requestList = useCallback(() => {
    refresh();
    if (controlsAreLive) {
      // Ask the agent for an immediate sample as an acceleration only. The
      // response is persisted by the server and delivered back through the
      // REST/SSE read plane; it is never held only in this component.
      sendToAgent(agentId, { type: 'ListServicesRequest' });
    }
  }, [agentId, controlsAreLive, refresh, sendToAgent]);

  useEffect(() => {
    const unsubscribe = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'ControlServiceResponse') {
        const { name, success, error: err } = msg.payload;
        const timer = pendingTimers.current.get(name);
        if (timer) clearTimeout(timer);
        pendingTimers.current.delete(name);
        setPending((prev) => {
          const next = { ...prev };
          delete next[name];
          return next;
        });
        if (success) {
          setToast({ kind: 'success', text: `${name}: ok` });
          // The server collector will publish a durable update within its next
          // cycle. A direct request makes successful controls visible sooner.
          sendToAgent(agentId, { type: 'ListServicesRequest' });
          refresh();
        } else {
          setToast({ kind: 'error', text: `${name}: ${err ?? 'failed'}` });
        }
      }
    });

    return unsubscribe;
  }, [agentId, onAgentMessage, refresh, sendToAgent]);

  useEffect(() => {
    const timers = pendingTimers.current;
    return () => {
      for (const timer of timers.values()) clearTimeout(timer);
      timers.clear();
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 4000);
    return () => clearTimeout(t);
  }, [toast]);

  const handleControl = (name: string, action: Action) => {
    if (!controlsAreLive) return;
    setPending((prev) => ({ ...prev, [name]: action }));
    sendToAgent(agentId, {
      type: 'ControlServiceRequest',
      payload: { name, action },
    });
    const previousTimer = pendingTimers.current.get(name);
    if (previousTimer) clearTimeout(previousTimer);
    pendingTimers.current.set(
      name,
      setTimeout(() => {
        pendingTimers.current.delete(name);
        setPending((prev) => {
          const next = { ...prev };
          delete next[name];
          return next;
        });
        setToast({ kind: 'error', text: `${name}: control timed out` });
      }, CONTROL_TIMEOUT_MS),
    );
  };

  const filtered = useMemo(() => {
    if (!services) return [];
    const q = filter.trim().toLowerCase();
    return services.filter((s) => {
      if (stateFilter === 'active' && s.active_state !== 'active') return false;
      if (stateFilter === 'failed' && s.active_state !== 'failed') return false;
      if (stateFilter === 'inactive' && s.active_state === 'active') return false;
      if (!q) return true;
      return s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q);
    });
  }, [services, filter, stateFilter]);

  const counts = useMemo(() => {
    const c = { total: 0, active: 0, failed: 0, inactive: 0 };
    if (!services) return c;
    for (const s of services) {
      c.total += 1;
      if (s.active_state === 'active') c.active += 1;
      else if (s.active_state === 'failed') c.failed += 1;
      else c.inactive += 1;
    }
    return c;
  }, [services]);

  return (
    <div
      className="panel"
      style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}
    >
      <div className="panel-head">
        <div className="panel-title">
          <span className="ico">≡</span> SERVICES
          {services && (
            <span className="meta">
              {counts.total} units · {counts.active} active
              {counts.failed > 0 ? ` · ${counts.failed} failed` : ''}
            </span>
          )}
        </div>
        <div className="panel-actions">
          <div className="search-input" style={{ width: 200, height: 24 }}>
            <span style={{ color: 'var(--accent)' }}>⌕</span>
            <input
              placeholder="filter…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
          </div>
          <div className="seg">
            {(['all', 'active', 'failed', 'inactive'] as const).map((k) => (
              <button
                key={k}
                className={stateFilter === k ? 'on' : ''}
                onClick={() => setStateFilter(k)}
              >
                {k}
              </button>
            ))}
          </div>
          <button className="btn sm" onClick={requestList} disabled={loading} title="Refresh service state">
            ↻
          </button>
        </div>
      </div>

      {(!dataIsLive || !controlsAreLive) && (
        <div className="live-data-note" role="status">
          {!isAgentOnline
            ? hasDurableServices
              ? 'This agent is offline. Showing its last durable service snapshot.'
              : 'This agent is offline. No durable service snapshot is available yet.'
            : !dataIsLive
              ? hasDurableServices
                ? 'Live service updates are reconnecting. Showing the latest durable snapshot.'
                : 'Live service updates are reconnecting. No durable snapshot is available yet.'
              : 'Service data is live. Interactive controls are reconnecting.'}
        </div>
      )}

      <div className="panel-body flush" style={{ flex: 1, overflow: 'auto', minHeight: 0 }}>
        {services === null ? (
          <div className="empty">
            <Loader2Icon className="w-5 h-5 animate-spin" />
          </div>
        ) : filtered.length === 0 ? (
          <div className="empty">
            {services.length === 0
              ? 'No services reported.'
              : 'No services match the current filter.'}
          </div>
        ) : (
          <table className="tbl">
            <tbody>
              {filtered.map((s) => {
                const cls =
                  s.active_state === 'active'
                    ? 'ok'
                    : s.active_state === 'failed'
                      ? 'err-c'
                      : 'muted';
                const dot = s.active_state === 'active' ? '●' : s.active_state === 'failed' ? '●' : '○';
                const p = pending[s.name];
                return (
                  <tr key={s.name}>
                    <td style={{ width: 24 }} className={`${cls} center`}>
                      {dot}
                    </td>
                    <td className="mono" style={{ color: 'var(--fg)' }}>
                      {s.name}
                    </td>
                    <td className="muted" title={s.description}>
                      {s.description}
                    </td>
                    <td className={`mono ${cls}`} style={{ width: 90 }}>
                      {s.active_state || '—'}
                    </td>
                    <td className="actions" style={{ width: 130 }}>
                      <button
                        className="btn sm icon"
                        title={!controlsAreLive ? 'Live controls are reconnecting' : !canWrite ? 'viewer role: read-only' : 'Start'}
                        disabled={!!p || !canWrite || !controlsAreLive}
                        onClick={() => handleControl(s.name, 'start')}
                      >
                        {p === 'start' ? '…' : '▶'}
                      </button>
                      <button
                        className="btn sm icon"
                        title={!controlsAreLive ? 'Live controls are reconnecting' : !canWrite ? 'viewer role: read-only' : 'Stop'}
                        disabled={!!p || !canWrite || !controlsAreLive}
                        onClick={() => handleControl(s.name, 'stop')}
                      >
                        {p === 'stop' ? '…' : '■'}
                      </button>
                      <button
                        className="btn sm icon"
                        title={!controlsAreLive ? 'Live controls are reconnecting' : !canWrite ? 'viewer role: read-only' : 'Restart'}
                        disabled={!!p || !canWrite || !controlsAreLive}
                        onClick={() => handleControl(s.name, 'restart')}
                      >
                        {p === 'restart' ? '…' : '↻'}
                      </button>
                      <button
                        className="btn sm icon"
                        title={controlsAreLive ? 'journalctl -fu' : 'Live controls are reconnecting'}
                        disabled={!controlsAreLive}
                        onClick={() => setLogUnit(s.name)}
                      >
                        ≡
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {toast && (
        <div
          style={{
            padding: '6px 12px',
            background: toast.kind === 'success' ? 'var(--accent-bg)' : 'var(--err-bg)',
            color: toast.kind === 'success' ? 'var(--accent)' : 'var(--err)',
            fontFamily: 'var(--mono)',
            fontSize: 11,
            borderTop: '1px solid var(--line)',
          }}
        >
          {toast.text}
        </div>
      )}

      {logUnit && (
        <JournalLogViewer
          agentId={agentId}
          unit={logUnit}
          onClose={() => setLogUnit(null)}
        />
      )}
    </div>
  );
}
