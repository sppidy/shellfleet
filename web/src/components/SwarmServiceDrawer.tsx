'use client';

import { useEffect, useState } from 'react';
import { useWebSocket } from './providers/WebSocketProvider';
import { SwarmServiceInspectPayload } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

export default function SwarmServiceDrawer({
  agentId,
  serviceName,
  onClose,
}: {
  agentId: string;
  serviceName: string;
  onClose: () => void;
}) {
  const { sendToAgent, onAgentMessage } = useWebSocket();
  const [data, setData] = useState<SwarmServiceInspectPayload | null>(null);
  const [unsupported, setUnsupported] = useState(false);

  useEffect(() => {
    setData(null);
    setUnsupported(false);

    const unsub = onAgentMessage(agentId, (msg) => {
      if (
        msg.type === 'SwarmServiceInspectResponse' &&
        msg.payload.name === serviceName
      ) {
        setData(msg.payload);
      }
    });

    sendToAgent(agentId, {
      type: 'SwarmServiceInspectRequest',
      payload: { name: serviceName },
    });

    const timeout = setTimeout(() => setUnsupported(true), 8000);
    return () => {
      unsub();
      clearTimeout(timeout);
    };
  }, [agentId, serviceName, sendToAgent, onAgentMessage]);

  return (
    <div
      className="modal-overlay"
      style={{ justifyContent: 'flex-end', padding: 0 }}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        style={{
          width: 'min(640px, 95vw)',
          height: '100vh',
          background: 'var(--bg-1)',
          borderLeft: '1px solid var(--line)',
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">⊞</span> SWARM SERVICE
            <span className="meta">{serviceName}</span>
          </div>
          <div className="panel-actions">
            <button
              className="btn sm"
              onClick={() =>
                sendToAgent(agentId, {
                  type: 'SwarmServiceInspectRequest',
                  payload: { name: serviceName },
                })
              }
            >
              ↻
            </button>
            <button className="icon-btn" onClick={onClose} title="Close">
              ×
            </button>
          </div>
        </div>

        <div style={{ flex: 1, overflow: 'auto', padding: 16 }}>
          {!data ? (
            unsupported ? (
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
                ⚠ Inspect not supported on this agent. Upgrade to the latest shellfleet-agent.
              </div>
            ) : (
              <div className="empty">
                <Loader2Icon className="w-5 h-5 animate-spin" />
              </div>
            )
          ) : !data.success ? (
            <div
              style={{
                padding: 12,
                background: 'var(--err-bg)',
                border: '1px solid var(--err-bd)',
                borderRadius: 'var(--r)',
                color: 'var(--err)',
                fontFamily: 'var(--mono)',
                fontSize: 12,
              }}
            >
              {data.error ?? 'inspect failed'}
            </div>
          ) : (
            <>
              {data.spec && <SpecBlock spec={data.spec} />}
              <div style={{ height: 16 }} />
              <TasksBlock tasks={data.tasks} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function SpecBlock({
  spec,
}: {
  spec: NonNullable<SwarmServiceInspectPayload['spec']>;
}) {
  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">SPEC</div>
      </div>
      <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Row label="image" value={spec.image} mono />
        {spec.image_digest && <Row label="digest" value={spec.image_digest} mono />}
        <Row
          label="mode"
          value={
            spec.mode === 'replicated' && spec.replicas !== null
              ? `replicated · ${spec.replicas}`
              : spec.mode
          }
        />
        {spec.created_at && <Row label="created" value={spec.created_at} />}
        {spec.updated_at && <Row label="updated" value={spec.updated_at} />}
        {spec.published_ports.length > 0 && (
          <ListRow label="published ports" items={spec.published_ports} />
        )}
        {spec.networks.length > 0 && (
          <ListRow label="networks" items={spec.networks} />
        )}
        {spec.constraints.length > 0 && (
          <ListRow label="constraints" items={spec.constraints} />
        )}
        {spec.mounts.length > 0 && <ListRow label="mounts" items={spec.mounts} />}
        {spec.env.length > 0 && <ListRow label="environment" items={spec.env} />}
      </div>
    </div>
  );
}

function TasksBlock({
  tasks,
}: {
  tasks: SwarmServiceInspectPayload['tasks'];
}) {
  return (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">
          TASKS<span className="meta">{tasks.length}</span>
        </div>
      </div>
      <div className="panel-body flush">
        {tasks.length === 0 ? (
          <div className="empty">No task records.</div>
        ) : (
          <table className="tbl">
            <thead>
              <tr>
                <th>ID</th>
                <th>NAME</th>
                <th>NODE</th>
                <th>DESIRED</th>
                <th>CURRENT</th>
              </tr>
            </thead>
            <tbody>
              {tasks.map((t) => (
                <tr key={t.id}>
                  <td className="mono muted">{t.id.slice(0, 12)}</td>
                  <td className="mono" style={{ color: 'var(--fg)' }}>
                    {t.name}
                  </td>
                  <td className="mono">{t.node || '—'}</td>
                  <td className="mono">{t.desired_state}</td>
                  <td
                    className={`mono ${
                      t.current_state.startsWith('Running')
                        ? 'ok'
                        : t.current_state.startsWith('Failed') ||
                            t.current_state.startsWith('Rejected')
                          ? 'err-c'
                          : 'muted'
                    }`}
                    title={t.error || undefined}
                  >
                    {t.current_state}
                    {t.error && <div className="err-c" style={{ fontSize: 10.5 }}>{t.error}</div>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

function Row({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '120px 1fr', gap: 8, alignItems: 'baseline' }}>
      <span
        style={{
          fontSize: 10.5,
          color: 'var(--fg-3)',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          fontFamily: 'var(--mono)',
        }}
      >
        {label}
      </span>
      <span
        className={mono ? 'mono' : ''}
        style={{ color: 'var(--fg)', fontSize: 12, wordBreak: 'break-word' }}
      >
        {value}
      </span>
    </div>
  );
}

function ListRow({ label, items }: { label: string; items: string[] }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '120px 1fr', gap: 8 }}>
      <span
        style={{
          fontSize: 10.5,
          color: 'var(--fg-3)',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          fontFamily: 'var(--mono)',
        }}
      >
        {label}
      </span>
      <ul style={{ margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 3 }}>
        {items.map((it, i) => (
          <li
            key={`${label}-${i}`}
            className="mono"
            style={{
              background: 'var(--bg)',
              border: '1px solid var(--line)',
              borderRadius: 'var(--r)',
              padding: '3px 6px',
              fontSize: 11,
              color: 'var(--fg-1)',
              wordBreak: 'break-word',
            }}
          >
            {it}
          </li>
        ))}
      </ul>
    </div>
  );
}
