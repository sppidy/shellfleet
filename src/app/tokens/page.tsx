'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Loader2Icon } from 'lucide-react';
import { apiFetch } from '@/lib/api';
import { useCanWrite } from '@/components/providers/SessionProvider';

type TokenRow = {
  token_preview: string;
  hostname: string | null;
  created_at: number;
  last_seen: number;
};

const formatRelative = (unixSeconds: number) => {
  if (!unixSeconds) return 'never';
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
};

export default function TokensPage() {
  const router = useRouter();
  const canWrite = useCanWrite();
  const [rows, setRows] = useState<TokenRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/tokens');
      if (res.status === 401) {
        window.location.href = '/auth/login';
        return;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setRows((await res.json()) as TokenRow[]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load tokens');
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleRevoke = async (row: TokenRow) => {
    let body: Record<string, string>;
    if (row.hostname) {
      const ok = window.confirm(
        `Revoke pairing for ${row.hostname}? The agent will fail its next reconnect and need to be re-paired through /device.`,
      );
      if (!ok) return;
      body = { hostname: row.hostname };
    } else {
      const fullToken = window.prompt(
        `This token has never connected, so we can't match it by hostname. Paste the full token value to revoke (or cancel).`,
      );
      if (!fullToken) return;
      body = { token: fullToken.trim() };
    }

    setRevoking(row.token_preview);
    try {
      const res = await apiFetch('/api/tokens/revoke', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `HTTP ${res.status}`);
      }
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to revoke');
    } finally {
      setRevoking(null);
    }
  };

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
            <span className="here">tokens</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={refresh}>
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
                  <span className="ico">⚿</span> AGENT TOKENS
                  <span className="meta">{rows?.length ?? 0} active</span>
                </div>
              </div>
              <div className="panel-body flush">
                {rows === null ? (
                  <div className="empty">
                    <Loader2Icon className="w-5 h-5 animate-spin" />
                  </div>
                ) : rows.length === 0 ? (
                  <div className="empty">
                    No agents paired yet. Use{' '}
                    <span style={{ color: 'var(--accent)' }}>Connect agent</span> to add one.
                  </div>
                ) : (
                  <table className="tbl">
                    <thead>
                      <tr>
                        <th>HOSTNAME</th>
                        <th>TOKEN</th>
                        <th>ISSUED</th>
                        <th>LAST SEEN</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {rows.map((row) => (
                        <tr key={`${row.token_preview}-${row.created_at}`}>
                          <td className="mono" style={{ color: 'var(--fg)' }}>
                            {row.hostname ?? (
                              <span className="muted">(never connected)</span>
                            )}
                          </td>
                          <td className="mono muted">{row.token_preview}</td>
                          <td className="mono">{formatRelative(row.created_at)}</td>
                          <td className="mono muted">{formatRelative(row.last_seen)}</td>
                          <td className="actions">
                            <button
                              className="btn sm danger"
                              disabled={!canWrite || revoking === row.token_preview}
                              title={!canWrite ? 'viewer role: read-only' : undefined}
                              onClick={() => handleRevoke(row)}
                            >
                              {revoking === row.token_preview ? '…' : 'revoke'}
                            </button>
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
