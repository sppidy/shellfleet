'use client';

import Terminal from './Terminal';

export default function ContainerExecModal({
  agentId,
  containerId,
  containerName,
  shell,
  onClose,
}: {
  agentId: string;
  containerId: string;
  containerName: string;
  shell?: string;
  onClose: () => void;
}) {
  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div
        className="modal"
        style={{
          width: 'min(1000px, 95vw)',
          height: '80vh',
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">›_</span> docker exec
            <span className="meta">
              {containerName} · {containerId.slice(0, 12)} · shell {shell ?? 'sh'}
            </span>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </div>
        <div
          style={{
            padding: '6px 12px',
            borderBottom: '1px solid var(--line)',
            background: 'var(--bg-2)',
            color: 'var(--fg-2)',
            fontSize: 11,
            fontFamily: 'var(--mono)',
          }}
        >
          ▾ One exec session per host at a time. Closing this modal kills the PTY on the agent —
          nothing keeps running in the background.
        </div>
        <div style={{ flex: 1, minHeight: 0, background: '#06090b' }}>
          <Terminal
            agentId={agentId}
            containerId={containerId}
            shell={shell}
            title={`exec ${containerName}`}
          />
        </div>
      </div>
    </div>
  );
}
