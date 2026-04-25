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
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between gap-3 mb-3">
        <div className="flex items-center gap-2">
          <h3 className="text-base font-semibold text-slate-100">Services</h3>
          {services && (
            <span className="text-xs text-slate-500">
              {counts.total} total · <span className="text-emerald-400">{counts.active} active</span>
              {counts.failed > 0 && <> · <span className="text-red-400">{counts.failed} failed</span></>}
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={requestList}
          disabled={!isConnected}
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 disabled:opacity-50 text-slate-300 rounded-md transition-colors"
          title="Refresh"
        >
          <RefreshCwIcon className="w-3.5 h-3.5" />
          Refresh
        </button>
      </div>

      <div className="flex items-center gap-2 mb-3">
        <div className="relative flex-1">
          <SearchIcon className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-500" />
          <input
            type="text"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter services…"
            className="w-full pl-8 pr-7 py-1.5 text-sm bg-slate-900 border border-slate-700 rounded-md focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500 text-slate-100 placeholder:text-slate-500"
          />
          {filter && (
            <button
              type="button"
              onClick={() => setFilter('')}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-200"
              aria-label="Clear filter"
            >
              <XIcon className="w-4 h-4" />
            </button>
          )}
        </div>
        <div className="flex bg-slate-900 border border-slate-800 rounded-md p-0.5 text-xs">
          {(['all', 'active', 'failed', 'inactive'] as const).map((k) => (
            <button
              key={k}
              type="button"
              onClick={() => setStateFilter(k)}
              className={`px-2 py-1 rounded-md transition-colors ${
                stateFilter === k
                  ? 'bg-slate-700 text-slate-100'
                  : 'text-slate-400 hover:text-slate-100'
              }`}
            >
              {k}
            </button>
          ))}
        </div>
      </div>

      {error && (
        <div className="mb-3 flex items-start gap-2 text-xs text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-md p-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {services === null ? (
        <div className="flex-1 flex items-center justify-center text-slate-500">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      ) : filtered.length === 0 ? (
        <div className="flex-1 flex items-center justify-center text-sm text-slate-500">
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
          className={`absolute bottom-4 left-1/2 -translate-x-1/2 px-3 py-2 rounded-md shadow-xl text-sm border ${
            toast.kind === 'success'
              ? 'bg-emerald-500/15 border-emerald-500/40 text-emerald-200'
              : 'bg-red-500/15 border-red-500/40 text-red-200'
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
      ? 'bg-emerald-500/20 text-emerald-300'
      : service.active_state === 'failed'
        ? 'bg-red-500/20 text-red-300'
        : service.active_state === 'activating'
          ? 'bg-amber-500/20 text-amber-300'
          : 'bg-slate-800 text-slate-400';

  return (
    <li className="flex items-center justify-between gap-3 p-2.5 bg-slate-900 border border-slate-800 rounded-md hover:border-slate-700 transition-colors">
      <div className="overflow-hidden flex-1 min-w-0">
        <div className="font-medium text-slate-100 text-sm truncate" title={service.name}>
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
            <span className="text-[10px] text-slate-500 uppercase tracking-wide">{service.status}</span>
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
    emerald: 'hover:text-emerald-300 hover:bg-emerald-500/10',
    red: 'hover:text-red-300 hover:bg-red-500/10',
    blue: 'hover:text-blue-300 hover:bg-blue-500/10',
  }[color];
  return (
    <button
      type="button"
      title={label}
      onClick={onClick}
      disabled={disabled}
      className={`p-1.5 text-slate-400 ${palette} rounded disabled:opacity-50 disabled:cursor-not-allowed transition-colors`}
    >
      {loading ? <Loader2Icon className="w-3.5 h-3.5 animate-spin" /> : icon}
    </button>
  );
}
