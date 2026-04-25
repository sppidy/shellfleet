'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { ServiceInfo } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';
import JournalLogViewer from './JournalLogViewer';

type Action = 'start' | 'stop' | 'restart';
type Toast = { kind: 'success' | 'error'; text: string };

const REFRESH_INTERVAL_MS = 15_000;
const REQUEST_TIMEOUT_MS = 10_000;

export default function ServiceList({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage, isConnected } = useWebSocket();
  const [services, setServices] = useState<ServiceInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [stateFilter, setStateFilter] = useState<'all' | 'active' | 'failed' | 'inactive'>('all');
  const [pending, setPending] = useState<Record<string, Action>>({});
  const [toast, setToast] = useState<Toast | null>(null);
  const [logUnit, setLogUnit] = useState<string | null>(null);

  const requestTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const requestList = useCallback(() => {
    setError(null);
    sendToAgent(agentId, { type: 'ListServicesRequest' });
    if (requestTimer.current) clearTimeout(requestTimer.current);
    requestTimer.current = setTimeout(() => {
      setError('Agent did not respond in time. Retrying…');
      sendToAgent(agentId, { type: 'ListServicesRequest' });
    }, REQUEST_TIMEOUT_MS);
  }, [agentId, sendToAgent]);

  useEffect(() => {
    setServices(null);
    setError(null);
    setPending({});

    const unsubscribe = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'ListServicesResponse') {
        if (requestTimer.current) {
          clearTimeout(requestTimer.current);
          requestTimer.current = null;
        }
        setServices(msg.payload.services);
        setError(null);
      } else if (msg.type === 'ControlServiceResponse') {
        const { name, success, error: err } = msg.payload;
        setPending((prev) => {
          const next = { ...prev };
          delete next[name];
          return next;
        });
        if (success) {
          setToast({ kind: 'success', text: `${name}: ok` });
          requestList();
        } else {
          setToast({ kind: 'error', text: `${name}: ${err ?? 'failed'}` });
        }
      }
    });

    requestList();
    const interval = setInterval(requestList, REFRESH_INTERVAL_MS);

    return () => {
      unsubscribe();
      clearInterval(interval);
      if (requestTimer.current) {
        clearTimeout(requestTimer.current);
        requestTimer.current = null;
      }
    };
  }, [agentId, onAgentMessage, requestList]);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 4000);
    return () => clearTimeout(t);
  }, [toast]);

  const handleControl = (name: string, action: Action) => {
    setPending((prev) => ({ ...prev, [name]: action }));
    sendToAgent(agentId, {
      type: 'ControlServiceRequest',
      payload: { name, action },
    });
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
          <button className="btn sm" onClick={requestList} disabled={!isConnected}>
            ↻
          </button>
        </div>
      </div>

      {error && (
        <div
          style={{
            padding: 8,
            background: 'var(--warn-bg)',
            color: 'var(--warn)',
            fontSize: 11,
            fontFamily: 'var(--mono)',
            borderBottom: '1px solid var(--line)',
          }}
        >
          ⚠ {error}
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
                        title="Start"
                        disabled={!!p}
                        onClick={() => handleControl(s.name, 'start')}
                      >
                        {p === 'start' ? '…' : '▶'}
                      </button>
                      <button
                        className="btn sm icon"
                        title="Stop"
                        disabled={!!p}
                        onClick={() => handleControl(s.name, 'stop')}
                      >
                        {p === 'stop' ? '…' : '■'}
                      </button>
                      <button
                        className="btn sm icon"
                        title="Restart"
                        disabled={!!p}
                        onClick={() => handleControl(s.name, 'restart')}
                      >
                        {p === 'restart' ? '…' : '↻'}
                      </button>
                      <button
                        className="btn sm icon"
                        title="journalctl -fu"
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
