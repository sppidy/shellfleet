'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import UpdateWindowPanel from './UpdateWindowPanel';
import { AptStatusPayload, AptUpgradable } from '@/lib/types';
import {
  PackageIcon,
  RefreshCwIcon,
  AlertCircleIcon,
  Loader2Icon,
  ShieldAlertIcon,
  CheckCircleIcon,
} from 'lucide-react';

const STATUS_TIMEOUT_MS = 8_000;

export default function AptManager({ agentId }: { agentId: string }) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [status, setStatus] = useState<AptStatusPayload | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [upgrading, setUpgrading] = useState<string | 'all' | null>(null);
  const [log, setLog] = useState<string | null>(null);
  const [logKind, setLogKind] = useState<'success' | 'error'>('success');
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setStatus(null);
    setUnsupported(false);
    setRefreshing(false);
    setUpgrading(null);
    setLog(null);

    const unsub = onAgentMessage(agentId, (msg) => {
      if (msg.type === 'AptStatusResponse') {
        if (timeoutRef.current) {
          clearTimeout(timeoutRef.current);
          timeoutRef.current = null;
        }
        setUnsupported(false);
        setStatus(msg.payload);
      } else if (msg.type === 'AptRefreshResponse') {
        setRefreshing(false);
        setLogKind(msg.payload.success ? 'success' : 'error');
        setLog(msg.payload.log || (msg.payload.error ?? ''));
        // Refetch the upgradable list once the index is fresh.
        sendToAgent(agentId, { type: 'AptStatusRequest' });
      } else if (msg.type === 'AptUpgradeResponse') {
        setUpgrading(null);
        setLogKind(msg.payload.success ? 'success' : 'error');
        setLog(msg.payload.log || (msg.payload.error ?? ''));
        sendToAgent(agentId, { type: 'AptStatusRequest' });
      }
    });

    sendToAgent(agentId, { type: 'AptStatusRequest' });
    timeoutRef.current = setTimeout(() => setUnsupported(true), STATUS_TIMEOUT_MS);

    return () => {
      unsub();
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [agentId, sendToAgent, onAgentMessage]);

  const refresh = () => {
    setRefreshing(true);
    setLog(null);
    sendToAgent(agentId, { type: 'AptRefreshRequest' });
  };

  const upgradeOne = (pkg: string) => {
    setUpgrading(pkg);
    setLog(null);
    sendToAgent(agentId, {
      type: 'AptUpgradeRequest',
      payload: { package: pkg },
    });
  };

  const upgradeAll = () => {
    if (!confirm('Upgrade all upgradable packages on this host?')) return;
    setUpgrading('all');
    setLog(null);
    sendToAgent(agentId, {
      type: 'AptUpgradeRequest',
      payload: { package: null },
    });
  };

  const lastUpdated = status?.last_update_secs
    ? new Date(status.last_update_secs * 1000).toLocaleString()
    : 'never';

  if (unsupported && !status) {
    return (
      <div className="flex items-start gap-2 text-sm text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-md px-3 py-2">
        <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
        <span>
          This agent doesn&apos;t expose apt updates yet. Upgrade it via{' '}
          <code className="bg-amber-500/20 px-1 py-0.5 rounded">
            apt install --only-upgrade sys-manager-agent
          </code>
          .
        </span>
      </div>
    );
  }

  if (!status) {
    return (
      <div className="flex items-center justify-center py-12 text-slate-500">
        <Loader2Icon className="w-5 h-5 animate-spin" />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <UpdateWindowPanel agentId={agentId} />
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div>
          <div className="flex items-center gap-2">
            <PackageIcon className="w-5 h-5 text-slate-400" />
            <h2 className="text-base font-semibold">Updates</h2>
            <span className="text-xs text-slate-500">
              · {status.upgradable.length} upgradable
            </span>
          </div>
          <div className="text-xs text-slate-500 mt-0.5">
            Last <code className="text-slate-400">apt-get update</code>: {lastUpdated}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={refresh}
            disabled={refreshing || upgrading !== null}
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 disabled:opacity-50 text-slate-300 rounded-md transition-colors"
          >
            {refreshing ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <RefreshCwIcon className="w-3.5 h-3.5" />
            )}
            apt-get update
          </button>
          <button
            type="button"
            onClick={upgradeAll}
            disabled={
              refreshing || upgrading !== null || status.upgradable.length === 0
            }
            className="text-xs flex items-center gap-1.5 px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 disabled:cursor-not-allowed text-white font-medium rounded-md transition-colors"
          >
            {upgrading === 'all' ? (
              <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <ShieldAlertIcon className="w-3.5 h-3.5" />
            )}
            Upgrade all ({status.upgradable.length})
          </button>
        </div>
      </div>

      {status.error && (
        <div className="flex items-start gap-2 text-sm text-red-300 bg-red-500/10 border border-red-500/30 rounded-md px-3 py-2">
          <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
          <span>{status.error}</span>
        </div>
      )}

      {status.upgradable.length === 0 ? (
        <div className="border border-dashed border-slate-800 rounded-md px-4 py-8 text-center text-sm text-slate-500">
          <CheckCircleIcon className="w-5 h-5 mx-auto mb-2 text-emerald-500" />
          All packages on this host are up to date.
        </div>
      ) : (
        <ul className="divide-y divide-slate-800 border border-slate-800 rounded-md overflow-hidden">
          {status.upgradable.map((pkg) => (
            <PackageRow
              key={pkg.name}
              pkg={pkg}
              upgrading={upgrading === pkg.name}
              disabled={upgrading !== null || refreshing}
              onUpgrade={() => upgradeOne(pkg.name)}
            />
          ))}
        </ul>
      )}

      {log !== null && (
        <details
          open
          className={`rounded-md border ${
            logKind === 'success'
              ? 'border-emerald-500/30 bg-emerald-500/5'
              : 'border-red-500/30 bg-red-500/5'
          }`}
        >
          <summary
            className={`cursor-pointer px-3 py-2 text-xs font-medium ${
              logKind === 'success' ? 'text-emerald-300' : 'text-red-300'
            }`}
          >
            apt log {logKind === 'success' ? '· success' : '· failed'}
          </summary>
          <pre className="text-[11px] bg-slate-950 text-slate-300 px-3 py-2 overflow-x-auto whitespace-pre-wrap max-h-64 border-t border-slate-800">
            {log || '(empty)'}
          </pre>
        </details>
      )}
    </div>
  );
}

function PackageRow({
  pkg,
  upgrading,
  disabled,
  onUpgrade,
}: {
  pkg: AptUpgradable;
  upgrading: boolean;
  disabled: boolean;
  onUpgrade: () => void;
}) {
  return (
    <li className="px-3 py-2 bg-slate-900 flex items-center justify-between gap-3">
      <div className="min-w-0 flex-1">
        <div className="font-medium text-slate-100 text-sm truncate" title={pkg.name}>
          {pkg.name}
        </div>
        <div className="text-xs text-slate-500 truncate" title={pkg.source}>
          <span className="text-slate-400">{pkg.current_version}</span>{' '}
          <span className="text-slate-500">→</span>{' '}
          <span className="text-slate-200">{pkg.new_version}</span>
          {pkg.source && <span className="ml-2 text-slate-500">· {pkg.source}</span>}
        </div>
      </div>
      <button
        type="button"
        disabled={disabled}
        onClick={onUpgrade}
        className="text-xs flex items-center gap-1.5 px-2.5 py-1 rounded-md border border-slate-700 text-slate-300 hover:bg-slate-800 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
      >
        {upgrading ? <Loader2Icon className="w-3.5 h-3.5 animate-spin" /> : null}
        Upgrade
      </button>
    </li>
  );
}
