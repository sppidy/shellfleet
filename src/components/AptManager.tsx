'use client';

import { useEffect, useRef, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import UpdateWindowPanel from './UpdateWindowPanel';
import { AptStatusPayload, AptUpgradable } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

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
    sendToAgent(agentId, { type: 'AptUpgradeRequest', payload: { package: pkg } });
  };

  const upgradeAll = () => {
    if (!confirm('Upgrade all upgradable packages on this host?')) return;
    setUpgrading('all');
    setLog(null);
    sendToAgent(agentId, { type: 'AptUpgradeRequest', payload: { package: null } });
  };

  const lastUpdated = status?.last_update_secs
    ? new Date(status.last_update_secs * 1000).toLocaleString()
    : 'never';

  if (unsupported && !status) {
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
          ⚠ This agent doesn&apos;t expose apt updates yet. Upgrade with{' '}
          <code style={{ background: 'rgba(0,0,0,0.2)', padding: '0 4px' }}>
            apt install --only-upgrade sys-manager-agent
          </code>
          .
        </div>
      </div>
    );
  }

  if (!status) {
    return (
      <div className="pane">
        <div className="empty">
          <Loader2Icon className="w-5 h-5 animate-spin" />
        </div>
      </div>
    );
  }

  return (
    <div className="pane">
      <UpdateWindowPanel agentId={agentId} />

      {status.error && (
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
          {status.error}
        </div>
      )}

      <div className="panel">
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">▤</span> UPGRADABLE
            <span className="meta">
              {status.upgradable.length} packages · last apt-get update {lastUpdated}
            </span>
          </div>
          <div className="panel-actions">
            <button
              className="btn"
              onClick={refresh}
              disabled={refreshing || upgrading !== null}
            >
              {refreshing ? '…' : '↻ apt-get update'}
            </button>
            <button
              className="btn primary"
              onClick={upgradeAll}
              disabled={refreshing || upgrading !== null || status.upgradable.length === 0}
            >
              {upgrading === 'all' ? '…' : `▲ upgrade all (${status.upgradable.length})`}
            </button>
          </div>
        </div>
        <div className="panel-body flush">
          {status.upgradable.length === 0 ? (
            <div className="empty ok">
              ✓ All packages on this host are up to date.
            </div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>PACKAGE</th>
                  <th>CURRENT</th>
                  <th>NEW</th>
                  <th>SOURCE</th>
                  <th style={{ width: 140 }} />
                </tr>
              </thead>
              <tbody>
                {status.upgradable.map((pkg) => (
                  <PackageRow
                    key={pkg.name}
                    pkg={pkg}
                    upgrading={upgrading === pkg.name}
                    disabled={upgrading !== null || refreshing}
                    onUpgrade={() => upgradeOne(pkg.name)}
                  />
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

      {log !== null && (
        <details
          open
          className="panel"
          style={{
            borderColor: logKind === 'success' ? 'var(--accent-bd)' : 'var(--err-bd)',
          }}
        >
          <summary
            className="panel-head"
            style={{
              cursor: 'pointer',
              color: logKind === 'success' ? 'var(--accent)' : 'var(--err)',
              listStyle: 'none',
            }}
          >
            <div className="panel-title">
              apt log {logKind === 'success' ? '· success' : '· failed'}
            </div>
          </summary>
          <pre className="code" style={{ margin: 0, borderRadius: 0, border: 0, fontSize: 10.5 }}>
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
  const isSecurity = pkg.source.toLowerCase().includes('security');
  return (
    <tr>
      <td className="mono" style={{ color: 'var(--fg)' }}>
        {pkg.name}
      </td>
      <td className="mono muted">{pkg.current_version}</td>
      <td className="mono" style={{ color: 'var(--accent)' }}>
        → {pkg.new_version}
      </td>
      <td className={`mono ${isSecurity ? 'err-c' : ''}`}>{pkg.source || '—'}</td>
      <td className="actions">
        <button
          className="btn sm primary"
          disabled={disabled}
          onClick={onUpgrade}
        >
          {upgrading ? '…' : '▲ upgrade'}
        </button>
      </td>
    </tr>
  );
}
