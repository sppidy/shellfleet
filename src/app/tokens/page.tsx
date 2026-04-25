'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import {
  KeyIcon,
  ArrowLeftIcon,
  RefreshCwIcon,
  Trash2Icon,
  Loader2Icon,
  ServerIcon,
  AlertCircleIcon,
} from 'lucide-react';
import { apiFetch } from '@/lib/api';

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
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      setRows((await res.json()) as TokenRow[]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load tokens');
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleRevoke = async (row: TokenRow) => {
    // Prefer revoking by hostname — the operator only sees the preview here
    // and shouldn't have to SSH + cat the agent-token file just to revoke.
    // Fall back to a token prompt only when the row has never connected
    // (no hostname recorded yet).
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
    <div className="min-h-screen bg-slate-950 text-slate-100 flex flex-col">
      <header className="border-b border-slate-800 px-6 py-4 flex items-center justify-between">
        <button
          type="button"
          onClick={() => router.push('/')}
          className="inline-flex items-center text-sm text-slate-400 hover:text-slate-100"
        >
          <ArrowLeftIcon className="w-4 h-4 mr-1.5" />
          Back to dashboard
        </button>
        <button
          type="button"
          onClick={refresh}
          className="inline-flex items-center gap-1.5 text-sm text-slate-400 hover:text-slate-100"
        >
          <RefreshCwIcon className="w-3.5 h-3.5" />
          Refresh
        </button>
      </header>

      <main className="flex-1 px-6 py-8">
        <div className="max-w-3xl mx-auto">
          <div className="flex items-center gap-3 mb-1">
            <KeyIcon className="w-5 h-5 text-blue-400" />
            <h1 className="text-2xl font-semibold">Approved agent tokens</h1>
          </div>
          <p className="text-sm text-slate-400 mb-6">
            One row per token issued through the device-auth flow. Revoke a
            row to immediately invalidate that pairing — the agent will fail
            its next reconnect and have to be re-paired through{' '}
            <code className="text-slate-200">/device</code>.
          </p>

          {error && (
            <div className="mb-4 flex items-start gap-2 text-sm text-red-400 bg-red-500/5 border border-red-500/20 rounded-md p-3">
              <AlertCircleIcon className="w-4 h-4 mt-0.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          {rows === null ? (
            <div className="flex items-center justify-center py-12 text-slate-500">
              <Loader2Icon className="w-5 h-5 animate-spin" />
            </div>
          ) : rows.length === 0 ? (
            <div className="text-center text-slate-500 py-12 border border-dashed border-slate-800 rounded-lg">
              <ServerIcon className="w-6 h-6 mx-auto mb-2 text-slate-600" />
              No agents are paired yet.
            </div>
          ) : (
            <ul className="divide-y divide-slate-800 border border-slate-800 rounded-lg overflow-hidden">
              {rows.map((row) => (
                <li
                  key={`${row.token_preview}-${row.created_at}`}
                  className="flex items-center justify-between gap-4 px-4 py-3 bg-slate-900"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2 text-sm">
                      <ServerIcon className="w-4 h-4 text-slate-500 shrink-0" />
                      <span className="font-medium truncate">
                        {row.hostname ?? <em className="text-slate-500">(never connected)</em>}
                      </span>
                    </div>
                    <div className="text-xs text-slate-500 mt-1 flex flex-wrap gap-3">
                      <span>
                        Token <code className="text-slate-300">{row.token_preview}</code>
                      </span>
                      <span>Created {formatRelative(row.created_at)}</span>
                      <span>Last seen {formatRelative(row.last_seen)}</span>
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={() => handleRevoke(row)}
                    disabled={revoking === row.token_preview}
                    className="inline-flex items-center gap-1.5 text-xs font-medium py-1.5 px-2.5 rounded-md border border-red-500/30 text-red-400 hover:bg-red-500/10 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                  >
                    {revoking === row.token_preview ? (
                      <Loader2Icon className="w-3.5 h-3.5 animate-spin" />
                    ) : (
                      <Trash2Icon className="w-3.5 h-3.5" />
                    )}
                    Revoke
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </main>
    </div>
  );
}
