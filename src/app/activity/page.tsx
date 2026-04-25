'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { AuditRow } from '@/lib/types';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

const RELATIVE = (ts: number) => {
  if (!ts) return '—';
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86_400) return `${Math.floor(delta / 3_600)}h ago`;
  return `${Math.floor(delta / 86_400)}d ago`;
};

export default function ActivityPage() {
  const router = useRouter();
  const { status } = useSession();
  const [rows, setRows] = useState<AuditRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<'all' | 'user' | 'system' | 'cron'>('all');
  const [search, setSearch] = useState('');

  const fetchRows = useCallback(async () => {
    setError(null);
    try {
      const res = await fetch('/api/audit?limit=200', { credentials: 'same-origin' });
      if (res.status === 401) {
        window.location.href = '/auth/login';
        return;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setRows((await res.json()) as AuditRow[]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load activity');
    }
  }, []);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  useEffect(() => {
    if (status === 'authed') fetchRows();
  }, [status, fetchRows]);

  if (status !== 'authed') {
    return (
      <div
        className="app-shell"
        style={{ alignItems: 'center', justifyContent: 'center' }}
      >
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  const filtered = (rows ?? []).filter((r) => {
    if (filter !== 'all') {
      const actor = r.actor ?? '';
      if (filter === 'user' && (actor === 'system' || actor === 'cron' || !actor)) return false;
      if (filter === 'system' && actor !== 'system') return false;
      if (filter === 'cron' && actor !== 'cron') return false;
    }
    if (search) {
      const q = search.toLowerCase();
      return (
        r.kind.toLowerCase().includes(q) ||
        (r.actor ?? '').toLowerCase().includes(q) ||
        (r.agent_id ?? '').toLowerCase().includes(q) ||
        (r.detail ?? '').toLowerCase().includes(q)
      );
    }
    return true;
  });

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
            <span className="here">activity</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={fetchRows} title="Refresh">
              ↻ refresh
            </button>
          </div>
        </div>

        <div className="scroll">
          <div className="pane">
            {error && (
              <div className="panel" style={{ borderColor: 'var(--err-bd)' }}>
                <div className="panel-body" style={{ color: 'var(--err)' }}>
                  {error}
                </div>
              </div>
            )}

            <div className="panel">
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">≡</span> ACTIVITY
                  <span className="meta">audit log · last 200 events</span>
                </div>
                <div className="panel-actions">
                  <div className="search-input" style={{ width: 240 }}>
                    <span style={{ color: 'var(--accent)' }}>⌕</span>
                    <input
                      placeholder="actor, kind, target…"
                      value={search}
                      onChange={(e) => setSearch(e.target.value)}
                    />
                  </div>
                  <div className="seg">
                    <button
                      className={filter === 'all' ? 'on' : ''}
                      onClick={() => setFilter('all')}
                    >
                      all
                    </button>
                    <button
                      className={filter === 'user' ? 'on' : ''}
                      onClick={() => setFilter('user')}
                    >
                      user
                    </button>
                    <button
                      className={filter === 'system' ? 'on' : ''}
                      onClick={() => setFilter('system')}
                    >
                      system
                    </button>
                    <button
                      className={filter === 'cron' ? 'on' : ''}
                      onClick={() => setFilter('cron')}
                    >
                      cron
                    </button>
                  </div>
                </div>
              </div>
              <div className="panel-body flush">
                {rows === null ? (
                  <div className="empty">
                    <Loader2Icon className="w-5 h-5 animate-spin" />
                  </div>
                ) : filtered.length === 0 ? (
                  <div className="empty">No activity matches.</div>
                ) : (
                  <table className="tbl">
                    <thead>
                      <tr>
                        <th style={{ width: 100 }}>TS</th>
                        <th style={{ width: 100 }}>ACTOR</th>
                        <th style={{ width: 200 }}>KIND</th>
                        <th>TARGET</th>
                        <th style={{ width: 80 }}>OK</th>
                      </tr>
                    </thead>
                    <tbody>
                      {filtered.map((r) => (
                        <tr key={r.id}>
                          <td
                            className="mono muted"
                            title={new Date(r.ts * 1000).toLocaleString()}
                          >
                            {RELATIVE(r.ts)}
                          </td>
                          <td
                            className="mono"
                            style={{
                              color:
                                r.actor === 'system'
                                  ? 'var(--info)'
                                  : r.actor === 'cron'
                                    ? 'var(--warn)'
                                    : 'var(--accent)',
                            }}
                          >
                            {r.actor ?? '—'}
                          </td>
                          <td className="mono" style={{ color: 'var(--fg)' }}>
                            {r.kind}
                          </td>
                          <td className="mono">
                            {(r.agent_id ?? '').replace(/-id$/, '')}
                            {r.detail ? (
                              <span className="muted"> · {r.detail}</span>
                            ) : null}
                          </td>
                          <td className={`mono ${r.ok ? 'ok' : 'err-c'}`}>
                            {r.ok ? '✓ ok' : '× fail'}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
