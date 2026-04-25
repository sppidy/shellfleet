'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useSession } from '@/components/providers/SessionProvider';
import { useUi } from '@/components/providers/UiProvider';
import { apiFetch } from '@/lib/api';
import type { Notification } from '@/lib/types';
import { Loader2Icon } from 'lucide-react';

const RELATIVE = (ts: number) => {
  if (!ts) return '—';
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86_400) return `${Math.floor(delta / 3_600)}h ago`;
  return `${Math.floor(delta / 86_400)}d ago`;
};

type LevelFilter = 'all' | 'unread' | 'error' | 'warn';

export default function NotificationsPage() {
  const router = useRouter();
  const ui = useUi();
  const { status } = useSession();
  const [rows, setRows] = useState<Notification[] | null>(null);
  const [filter, setFilter] = useState<LevelFilter>('all');

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  const refresh = useCallback(async () => {
    try {
      const url =
        filter === 'unread'
          ? '/api/notifications?unread=true&limit=200'
          : '/api/notifications?limit=200';
      const res = await apiFetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data: Notification[] = await res.json();
      setRows(data);
    } catch (e) {
      ui.toast('error', `Load failed: ${(e as Error).message}`);
    }
  }, [filter, ui]);

  useEffect(() => {
    void refresh();
    const t = setInterval(refresh, 5_000);
    return () => clearInterval(t);
  }, [refresh]);

  const markRead = async (id: number) => {
    const res = await apiFetch(`/api/notifications/${id}/read`, { method: 'POST' });
    if (res.ok) void refresh();
  };

  const markAll = async () => {
    const res = await apiFetch('/api/notifications/mark-all-read', { method: 'POST' });
    if (res.ok) {
      const j = await res.json();
      ui.toast('success', `Marked ${j.updated} as read`);
      void refresh();
    }
  };

  const remove = async (id: number) => {
    const res = await apiFetch(`/api/notifications/${id}`, { method: 'DELETE' });
    if (res.ok) void refresh();
  };

  if (status === 'loading' || status === 'guest') {
    return (
      <div
        className="app-shell"
        style={{ alignItems: 'center', justifyContent: 'center' }}
      >
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  const filtered = (rows ?? []).filter((n) => {
    if (filter === 'unread') return n.read_at == null;
    if (filter === 'error') return n.level === 'error';
    if (filter === 'warn') return n.level === 'warn';
    return true;
  });
  const unreadCount = (rows ?? []).filter((n) => n.read_at == null).length;

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
            <span className="here">notifications</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={refresh}>
              ↻ refresh
            </button>
          </div>
        </div>

        <div className="scroll">
          <div className="pane">
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">◇</span> NOTIFICATIONS
                  <span className="meta">{unreadCount} unread</span>
                </div>
                <div className="panel-actions">
                  <div className="seg">
                    <button
                      className={filter === 'all' ? 'on' : ''}
                      onClick={() => setFilter('all')}
                    >
                      all
                    </button>
                    <button
                      className={filter === 'unread' ? 'on' : ''}
                      onClick={() => setFilter('unread')}
                    >
                      unread
                    </button>
                    <button
                      className={filter === 'error' ? 'on' : ''}
                      onClick={() => setFilter('error')}
                    >
                      error
                    </button>
                    <button
                      className={filter === 'warn' ? 'on' : ''}
                      onClick={() => setFilter('warn')}
                    >
                      warn
                    </button>
                  </div>
                  <button className="btn" onClick={markAll}>
                    mark all read
                  </button>
                </div>
              </div>
              <div className="panel-body flush">
                {rows === null ? (
                  <div className="empty">
                    <Loader2Icon className="w-5 h-5 animate-spin" />
                  </div>
                ) : filtered.length === 0 ? (
                  <div className="empty">
                    {filter === 'unread' ? 'No unread notifications.' : 'Inbox is empty.'}
                  </div>
                ) : (
                  filtered.map((n) => {
                    const cls =
                      n.level === 'error'
                        ? 'err-c'
                        : n.level === 'warn'
                          ? 'warn-c'
                          : 'info-c';
                    const ico = n.level === 'error' ? '×' : n.level === 'warn' ? '!' : 'i';
                    const unread = n.read_at == null;
                    return (
                      <div
                        key={n.id}
                        style={{
                          display: 'grid',
                          gridTemplateColumns: '24px 1fr auto',
                          gap: 10,
                          padding: '10px 14px',
                          borderBottom: '1px solid var(--line)',
                          background: unread ? 'var(--accent-bg)' : 'transparent',
                        }}
                      >
                        <span
                          className={cls}
                          style={{
                            fontFamily: 'var(--mono)',
                            fontWeight: 700,
                            textAlign: 'center',
                          }}
                        >
                          {ico}
                        </span>
                        <div style={{ minWidth: 0 }}>
                          <div
                            style={{
                              color: 'var(--fg)',
                              fontFamily: 'var(--mono)',
                              fontSize: 12.5,
                            }}
                          >
                            {n.title}
                          </div>
                          {n.body && (
                            <pre
                              className="code"
                              style={{
                                fontSize: 11,
                                marginTop: 4,
                                maxHeight: 160,
                                whiteSpace: 'pre-wrap',
                                wordBreak: 'break-word',
                              }}
                            >
                              {n.body}
                            </pre>
                          )}
                          <div
                            className="muted"
                            style={{
                              fontSize: 11,
                              fontFamily: 'var(--mono)',
                              marginTop: 4,
                            }}
                          >
                            {n.kind}
                            {n.agent_id ? ` · ${n.agent_id.replace(/-id$/, '')}` : ''}
                            {' · '}
                            {RELATIVE(n.created_at)}
                          </div>
                        </div>
                        <div className="row" style={{ gap: 6, alignSelf: 'start' }}>
                          {unread && (
                            <button className="btn sm" onClick={() => markRead(n.id)}>
                              mark read
                            </button>
                          )}
                          <button
                            className="btn sm icon danger"
                            onClick={() => remove(n.id)}
                            title="Delete"
                          >
                            ×
                          </button>
                        </div>
                      </div>
                    );
                  })
                )}
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
