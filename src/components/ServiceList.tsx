'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { ServiceInfo } from '@/lib/types';
import {
  PlayIcon,
  SquareIcon,
  RefreshCwIcon,
  AlertCircleIcon,
  Loader2Icon,
  SearchIcon,
  XIcon,
} from 'lucide-react';

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

  const requestTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const requestList = useCallback(() => {
    setError(null);
    sendToAgent(agentId, { type: 'ListServicesRequest' });
    if (requestTimer.current) clearTimeout(requestTimer.current);
    requestTimer.current = setTimeout(() => {
      // Fired only if the agent never replied. Surface the failure so the
      // dashboard doesn't sit on "Loading services…" forever.
      setError('Agent did not respond in time. Retrying…');
      sendToAgent(agentId, { type: 'ListServicesRequest' });
    }, REQUEST_TIMEOUT_MS);
  }, [agentId, sendToAgent]);

  // Subscribe to messages from this agent and refresh on mount + every 15s.
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
          // Refresh — the new state may differ from what we sent.
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

  // Auto-dismiss toast.
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
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between gap-3 mb-3">
        <div className="flex items-center gap-2">
          <h3 className="text-base font-semibold text-slate-800">Services</h3>
          {services && (
            <span className="text-xs text-slate-500">
              {counts.total} total · <span className="text-emerald-600">{counts.active} active</span>
              {counts.failed > 0 && <> · <span className="text-red-600">{counts.failed} failed</span></>}
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={requestList}
          disabled={!isConnected}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-100 hover:bg-slate-200 disabled:opacity-50 text-slate-700 rounded-md transition-colors"
          title="Refresh"
        >
          <RefreshCwIcon className="w-3.5 h-3.5" />
          Refresh
        </button>
      </div>

      <div className="flex items-center gap-2 mb-3">
        <div className="relative flex-1">
          <SearchIcon className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-400" />
          <input
            type="text"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter services…"
            className="w-full pl-8 pr-7 py-1.5 text-sm bg-white border border-slate-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
          />
          {filter && (
            <button
              type="button"
              onClick={() => setFilter('')}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-400 hover:text-slate-600"
              aria-label="Clear filter"
            >
              <XIcon className="w-4 h-4" />
            </button>
          )}
        </div>
        <div className="flex bg-slate-100 rounded-md p-0.5 text-xs">
          {(['all', 'active', 'failed', 'inactive'] as const).map((k) => (
            <button
              key={k}
              type="button"
              onClick={() => setStateFilter(k)}
              className={`px-2 py-1 rounded-md transition-colors ${
                stateFilter === k ? 'bg-white shadow text-slate-900' : 'text-slate-500 hover:text-slate-800'
              }`}
            >
              {k}
            </button>
          ))}
        </div>
      </div>

      {error && (
        <div className="mb-3 flex items-start gap-2 text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded-md p-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {services === null ? (
        <div className="flex-1 flex items-center justify-center text-slate-400">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      ) : filtered.length === 0 ? (
        <div className="flex-1 flex items-center justify-center text-sm text-slate-400">
          {services.length === 0 ? 'No services reported.' : 'No services match the current filter.'}
        </div>
      ) : (
        <ul className="flex-1 overflow-y-auto space-y-1.5 pr-1">
          {filtered.map((service) => (
            <ServiceRow
              key={service.name}
              service={service}
              pending={pending[service.name]}
              onControl={handleControl}
            />
          ))}
        </ul>
      )}

      {toast && (
        <div
          className={`absolute bottom-4 left-1/2 -translate-x-1/2 px-3 py-2 rounded-md shadow-lg text-sm border ${
            toast.kind === 'success'
              ? 'bg-emerald-50 border-emerald-200 text-emerald-800'
              : 'bg-red-50 border-red-200 text-red-800'
          }`}
        >
          {toast.text}
        </div>
      )}
    </div>
  );
}

function ServiceRow({
  service,
  pending,
  onControl,
}: {
  service: ServiceInfo;
  pending?: Action;
  onControl: (name: string, action: Action) => void;
}) {
  const stateClasses =
    service.active_state === 'active'
      ? 'bg-emerald-100 text-emerald-800'
      : service.active_state === 'failed'
        ? 'bg-red-100 text-red-800'
        : service.active_state === 'activating'
          ? 'bg-amber-100 text-amber-800'
          : 'bg-slate-100 text-slate-700';

  return (
    <li className="flex items-center justify-between gap-3 p-2.5 bg-white border border-slate-200 rounded-md hover:border-slate-300 transition-colors">
      <div className="overflow-hidden flex-1 min-w-0">
        <div className="font-medium text-slate-900 text-sm truncate" title={service.name}>
          {service.name}
        </div>
        {service.description && (
          <div className="text-xs text-slate-500 truncate mt-0.5" title={service.description}>
            {service.description}
          </div>
        )}
        <div className="flex items-center gap-1.5 mt-1">
          <span className={`inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium uppercase tracking-wide ${stateClasses}`}>
            {service.active_state || '—'}
          </span>
          {service.status && service.status !== service.active_state && (
            <span className="text-[10px] text-slate-400 uppercase tracking-wide">{service.status}</span>
          )}
        </div>
      </div>

      <div className="flex space-x-0.5 shrink-0">
        <ActionButton
          label="Start"
          icon={<PlayIcon className="w-3.5 h-3.5" />}
          color="emerald"
          loading={pending === 'start'}
          disabled={!!pending}
          onClick={() => onControl(service.name, 'start')}
        />
        <ActionButton
          label="Stop"
          icon={<SquareIcon className="w-3.5 h-3.5" />}
          color="red"
          loading={pending === 'stop'}
          disabled={!!pending}
          onClick={() => onControl(service.name, 'stop')}
        />
        <ActionButton
          label="Restart"
          icon={<RefreshCwIcon className="w-3.5 h-3.5" />}
          color="blue"
          loading={pending === 'restart'}
          disabled={!!pending}
          onClick={() => onControl(service.name, 'restart')}
        />
      </div>
    </li>
  );
}

function ActionButton({
  label,
  icon,
  color,
  loading,
  disabled,
  onClick,
}: {
  label: string;
  icon: React.ReactNode;
  color: 'emerald' | 'red' | 'blue';
  loading: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  const palette = {
    emerald: 'hover:text-emerald-600 hover:bg-emerald-50',
    red: 'hover:text-red-600 hover:bg-red-50',
    blue: 'hover:text-blue-600 hover:bg-blue-50',
  }[color];
  return (
    <button
      type="button"
      title={label}
      onClick={onClick}
      disabled={disabled}
      className={`p-1.5 text-slate-500 ${palette} rounded disabled:opacity-50 disabled:cursor-not-allowed transition-colors`}
    >
      {loading ? <Loader2Icon className="w-3.5 h-3.5 animate-spin" /> : icon}
    </button>
  );
}
