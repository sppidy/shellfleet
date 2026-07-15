'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import Terminal from '@/components/Terminal';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useSession, useCanWrite } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

/**
 * Standalone multi-host terminal multiplexer.
 *
 * Each tab owns one Terminal component (one PTY per tab). Tabs are
 * kept mounted across switches via display:none so scrollback +
 * shell history survive — closing a tab is the only way to drop the
 * PTY (which sends StopTerminalRequest down the WS).
 *
 * Multiple tabs per agent are allowed since v14: each tab's
 * session_id keys the agent's PTY map, so the operator can run
 * several concurrent shells against the same host.
 */

interface Tab {
  id: string;       // stable ui id; doubles as the wire-level session_id
  agentId: string;
}

export default function TerminalPage() {
  const router = useRouter();
  const { agents } = useWebSocket();
  const { status } = useSession();
  const canWrite = useCanWrite();
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);

  // Auth gating
  useEffect(() => {
    if (status === 'guest') router.replace('/login');
    if (status === 'pending_mfa') router.replace('/mfa');
  }, [status, router]);

  // Close the picker on outside click / ESC.
  useEffect(() => {
    if (!picking) return;
    const onClick = (e: MouseEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(e.target as Node)) {
        setPicking(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setPicking(false);
    };
    window.addEventListener('mousedown', onClick);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', onClick);
      window.removeEventListener('keydown', onKey);
    };
  }, [picking]);

  const addTab = useCallback((agentId: string) => {
    setTabs((prev) => {
      // Each tab gets its own UUID — agent keys host PTYs by it, so
      // multiple tabs against the same host run independent shells.
      const id = crypto.randomUUID();
      const next: Tab = { id, agentId };
      setActiveId(next.id);
      return [...prev, next];
    });
    setPicking(false);
  }, []);

  const closeTab = useCallback((id: string) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id);
      if (activeId === id) {
        setActiveId(next.length > 0 ? next[next.length - 1].id : null);
      }
      return next;
    });
  }, [activeId]);

  if (status !== 'authed') {
    return (
      <div className="center-screen">
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  if (!canWrite) {
    return (
      <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
        <main className="main">
          <div className="topbar">
            <div className="breadcrumb">
              <span className="prompt">$</span>
              <button
                type="button"
                className="nav-item"
                onClick={() => router.push('/')}
                style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}
              >
                ←&nbsp;back
              </button>
              <span className="sep">/</span>
              <span className="here">terminal</span>
            </div>
          </div>
          <div className="scroll">
            <div
              style={{
                padding: 32,
                color: 'var(--warn)',
                fontFamily: 'var(--mono)',
                fontSize: 13,
                textAlign: 'center',
              }}
            >
              viewer role: interactive shells are admin-only.
              <br />
              ask an admin to promote you at <code>/admin</code>.
            </div>
          </div>
        </main>
      </div>
    );
  }

  // All online agents are eligible; multiple tabs against the same
  // agent are now allowed.
  const availableAgents = agents;

  return (
    <div
      style={{
        height: '100vh',
        width: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: '#06090b',
      }}
    >
      {/* Header strip: breadcrumb + tab bar + add */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          background: 'var(--bg-1)',
          borderBottom: '1px solid var(--line)',
          padding: '0 8px',
          height: 36,
          flexShrink: 0,
          gap: 8,
        }}
      >
        <button
          type="button"
          className="btn ghost"
          onClick={() => router.push('/')}
          title="Back to dashboard"
          style={{ height: 24, padding: '0 8px', fontSize: 11 }}
        >
          ←&nbsp;back
        </button>
        <span className="muted mono" style={{ fontSize: 11 }}>
          terminal
        </span>
        <div style={{ width: 1, alignSelf: 'stretch', background: 'var(--line)', margin: '0 4px' }} />

        {/* Tab strip */}
        <div
          style={{
            display: 'flex',
            gap: 2,
            flex: 1,
            overflowX: 'auto',
            alignItems: 'center',
          }}
        >
          {tabs.map((t) => {
            const isActive = t.id === activeId;
            // When the same agent has multiple tabs, suffix each label
            // with a 1-based ordinal so the operator can tell them
            // apart at a glance.
            const sameAgent = tabs.filter((x) => x.agentId === t.agentId);
            const ordinal = sameAgent.length > 1
              ? sameAgent.findIndex((x) => x.id === t.id) + 1
              : 0;
            const base = t.agentId.replace(/-id$/, '');
            const label = ordinal > 0 ? `${base} #${ordinal}` : base;
            return (
              <div
                key={t.id}
                onClick={() => setActiveId(t.id)}
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 6,
                  height: 24,
                  padding: '0 8px',
                  background: isActive ? 'var(--bg)' : 'transparent',
                  borderTop: `2px solid ${isActive ? 'var(--accent)' : 'transparent'}`,
                  cursor: 'pointer',
                  fontFamily: 'var(--mono)',
                  fontSize: 11,
                  color: isActive ? 'var(--fg)' : 'var(--fg-2)',
                  flexShrink: 0,
                }}
                title={t.agentId}
              >
                <span>›_ {label}</span>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(t.id);
                  }}
                  title="Close tab"
                  style={{
                    background: 'transparent',
                    color: 'var(--fg-3)',
                    border: 0,
                    cursor: 'pointer',
                    padding: '0 2px',
                    fontSize: 12,
                    lineHeight: 1,
                  }}
                >
                  ×
                </button>
              </div>
            );
          })}
        </div>

        {/* Add-tab control */}
        <div ref={pickerRef} style={{ position: 'relative' }}>
          <button
            type="button"
            className="btn sm"
            onClick={() => setPicking((p) => !p)}
            disabled={availableAgents.length === 0}
            title={
              availableAgents.length === 0 ? 'no agents online' : 'add a tab'
            }
            style={{ height: 24, padding: '0 8px', fontSize: 11 }}
          >
            ＋ host
          </button>
          {picking && (
            <div
              style={{
                position: 'absolute',
                top: '100%',
                right: 0,
                marginTop: 4,
                minWidth: 200,
                background: 'var(--bg-1)',
                border: '1px solid var(--line)',
                borderRadius: 4,
                padding: 4,
                zIndex: 50,
                fontFamily: 'var(--mono)',
                fontSize: 12,
                maxHeight: 320,
                overflowY: 'auto',
              }}
            >
              {availableAgents.length === 0 ? (
                <div className="muted" style={{ padding: 8 }}>
                  no agents online
                </div>
              ) : (
                availableAgents.map((a) => (
                  <div
                    key={a}
                    onClick={() => addTab(a)}
                    style={{
                      padding: '6px 10px',
                      cursor: 'pointer',
                      borderRadius: 3,
                      color: 'var(--fg-1)',
                    }}
                    onMouseEnter={(e) => {
                      (e.currentTarget as HTMLDivElement).style.background = 'var(--bg-2)';
                    }}
                    onMouseLeave={(e) => {
                      (e.currentTarget as HTMLDivElement).style.background = 'transparent';
                    }}
                  >
                    {a.replace(/-id$/, '')}
                  </div>
                ))
              )}
            </div>
          )}
        </div>
      </div>

      {/* Terminal panes — all mounted, only the active one visible. */}
      <div style={{ flex: 1, position: 'relative', minHeight: 0 }}>
        {tabs.length === 0 ? (
          <div
            style={{
              position: 'absolute',
              inset: 0,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              flexDirection: 'column',
              gap: 12,
              fontFamily: 'var(--mono)',
              fontSize: 12,
              color: 'var(--fg-2)',
            }}
          >
            <pre
              style={{
                margin: 0,
                color: 'var(--fg-3)',
                fontSize: 11,
                lineHeight: 1.4,
              }}
            >
              {`┌────────────────────────────────────┐
│  no terminals open                 │
│                                    │
│  click '+ host' to start one;      │
│  multiple tabs per host are fine,  │
│  tabs preserve scrollback          │
└────────────────────────────────────┘`}
            </pre>
          </div>
        ) : (
          tabs.map((t) => (
            <div
              key={t.id}
              style={{
                position: 'absolute',
                inset: 0,
                display: t.id === activeId ? 'flex' : 'none',
              }}
            >
              <Terminal
                agentId={t.agentId}
                sessionId={t.id}
                visible={t.id === activeId}
                title={`shell · ${t.agentId.replace(/-id$/, '')}`}
              />
            </div>
          ))
        )}
      </div>
    </div>
  );
}
