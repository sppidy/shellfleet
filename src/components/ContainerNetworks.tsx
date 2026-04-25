'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { useUi } from './providers/UiProvider';
import type { DockerNetwork } from '@/lib/types';
import {
  NetworkIcon,
  RefreshCwIcon,
  Trash2Icon,
  PlusIcon,
  Loader2Icon,
  AlertCircleIcon,
  EyeIcon,
} from 'lucide-react';

const REFRESH_MS = 15_000;

export default function ContainerNetworks({ agentId }: { agentId: string }) {
  const ui = useUi();
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [networks, setNetworks] = useState<DockerNetwork[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
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
          setCreating(false);
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
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <NetworkIcon className="w-5 h-5 text-slate-400" />
          <h2 className="text-base font-semibold">Networks</h2>
          <span className="text-xs text-slate-500">
            {networks === null ? 'loading…' : `· ${networks.length}`}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={refresh}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-md"
          >
            <RefreshCwIcon className="w-3.5 h-3.5" />
            Refresh
          </button>
          <button
            type="button"
            onClick={() => setCreating(true)}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-md"
          >
            <PlusIcon className="w-3.5 h-3.5" />
            Create
          </button>
        </div>
      </div>

      {creating && (
        <CreateForm
          agentId={agentId}
          onClose={() => setCreating(false)}
        />
      )}

      {error && (
        <div className="flex items-start gap-2 text-sm text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {networks === null ? (
        <div className="flex items-center justify-center py-12 text-slate-500">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      ) : networks.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          No networks.
        </div>
      ) : (
        <div className="rounded-md border border-slate-800 overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-slate-900/60 text-[11px] uppercase tracking-wide text-slate-500">
              <tr>
                <th className="text-left px-3 py-2 font-medium">Name</th>
                <th className="text-left px-3 py-2 font-medium">Driver</th>
                <th className="text-left px-3 py-2 font-medium">Scope</th>
                <th className="text-left px-3 py-2 font-medium">ID</th>
                <th className="text-left px-3 py-2 font-medium">Flags</th>
                <th className="px-3 py-2"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-800">
              {networks.map((n) => (
                <tr key={n.id} className="bg-slate-900/30">
                  <td className="px-3 py-2 font-medium text-slate-200">{n.name}</td>
                  <td className="px-3 py-2 text-slate-400">{n.driver}</td>
                  <td className="px-3 py-2 text-slate-400">{n.scope}</td>
                  <td className="px-3 py-2 font-mono text-xs text-slate-500" title={n.id}>
                    {n.id.slice(0, 12)}
                  </td>
                  <td className="px-3 py-2 text-xs text-slate-400 space-x-1">
                    {n.attachable && <span className="px-1 py-0.5 rounded bg-slate-800">attachable</span>}
                    {n.internal && <span className="px-1 py-0.5 rounded bg-slate-800">internal</span>}
                    {n.ipv6 && <span className="px-1 py-0.5 rounded bg-slate-800">ipv6</span>}
                  </td>
                  <td className="px-3 py-2 text-right">
                    <div className="flex items-center justify-end gap-1">
                      <button
                        type="button"
                        onClick={() => inspect(n)}
                        title="Inspect"
                        className="p-1.5 rounded text-slate-400 hover:text-slate-100 hover:bg-slate-800"
                      >
                        <EyeIcon className="w-4 h-4" />
                      </button>
                      <button
                        type="button"
                        onClick={() => remove(n)}
                        disabled={removing === n.id}
                        title="Remove"
                        className="p-1.5 rounded text-slate-400 hover:text-red-300 hover:bg-slate-800 disabled:opacity-50"
                      >
                        {removing === n.id ? (
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

function CreateForm({ agentId, onClose }: { agentId: string; onClose: () => void }) {
  const { sendToAgent } = useWebSocket();
  const [name, setName] = useState('');
  const [driver, setDriver] = useState('bridge');
  const [subnet, setSubnet] = useState('');
  const [attachable, setAttachable] = useState(false);
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
  };

  return (
    <form
      onSubmit={submit}
      className="rounded-md border border-slate-800 bg-slate-900/40 p-3 space-y-3"
    >
      <div className="grid grid-cols-3 gap-3">
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Name
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="my-network"
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100"
            required
          />
        </label>
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Driver
          <select
            value={driver}
            onChange={(e) => setDriver(e.target.value)}
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 text-sm text-slate-100"
          >
            <option value="bridge">bridge</option>
            <option value="overlay">overlay (swarm manager only)</option>
            <option value="macvlan">macvlan</option>
            <option value="host">host</option>
            <option value="none">none</option>
          </select>
        </label>
        <label className="text-xs text-slate-400 flex flex-col gap-1">
          Subnet (optional)
          <input
            type="text"
            value={subnet}
            onChange={(e) => setSubnet(e.target.value)}
            placeholder="172.30.0.0/16"
            className="bg-slate-950 border border-slate-700 rounded-md px-2 py-1.5 font-mono text-sm text-slate-100"
          />
        </label>
      </div>
      <div className="flex items-center gap-4 text-xs text-slate-300">
        <label className="flex items-center gap-1.5">
          <input
            type="checkbox"
            checked={attachable}
            onChange={(e) => setAttachable(e.target.checked)}
            className="accent-blue-600"
          />
          Attachable (overlay)
        </label>
        <label className="flex items-center gap-1.5">
          <input
            type="checkbox"
            checked={internal}
            onChange={(e) => setInternal(e.target.checked)}
            className="accent-blue-600"
          />
          Internal (no external connectivity)
        </label>
      </div>
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onClose}
          className="text-xs px-2.5 py-1.5 border border-slate-700 rounded-md text-slate-300 hover:bg-slate-800"
        >
          Cancel
        </button>
        <button
          type="submit"
          className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-md"
        >
          Create
        </button>
      </div>
    </form>
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
      className="fixed inset-0 z-50 bg-slate-950/70 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="bg-slate-900 border border-slate-800 rounded-lg shadow-2xl max-w-3xl w-full max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="p-4 border-b border-slate-800 flex items-center justify-between">
          <h3 className="text-base font-semibold text-slate-100">{title}</h3>
          <button
            type="button"
            onClick={onClose}
            className="text-slate-400 hover:text-slate-100"
          >
            ×
          </button>
        </div>
        <div className="flex-1 overflow-auto p-4">
          {json === null ? (
            <div className="flex items-center justify-center py-8 text-slate-500">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : (
            <pre className="text-[11px] whitespace-pre-wrap break-words text-slate-300 bg-slate-950 rounded p-3 border border-slate-800">
              {json}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
