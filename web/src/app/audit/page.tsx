'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface AuditRow { id: number; ts: number; actor: string | null; agent_id: string | null; kind: string; ok: number; detail: string | null }

const fmtTs = (t: number) => new Date(t * 1000).toLocaleString();

export default function AuditPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [rows, setRows] = useState<AuditRow[] | null>(null);
  const [limit, setLimit] = useState(200);
  const [q, setQ] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch(`/api/ee/audit?limit=${limit}`);
      if (!res.ok) { setError(`HTTP ${res.status}`); setRows([]); return; }
      setRows(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setRows([]); }
  }, [limit]);

  useEffect(() => { if (status === 'authed') load(); }, [status, load]);

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/audit requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  const ql = q.trim().toLowerCase();
  const shown = (rows ?? []).filter((r) =>
    !ql || r.kind.toLowerCase().includes(ql) || (r.actor || '').toLowerCase().includes(ql)
    || (r.agent_id || '').toLowerCase().includes(ql) || (r.detail || '').toLowerCase().includes(ql));

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">audit log</span>
          </div>
          <div className="topbar-actions">
            <input className="input" placeholder="filter (actor / kind / agent / detail)" value={q} onChange={(e) => setQ(e.target.value)} style={{ width: 240 }} />
            <select className="input" value={limit} onChange={(e) => setLimit(Number(e.target.value))} style={{ width: 90 }}>
              <option value={100}>100</option><option value={200}>200</option><option value={1000}>1000</option><option value={5000}>5000</option>
            </select>
            <button className="btn" onClick={load}>↻</button>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="audit-long" label="Audit Log">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">≣</span> AUDIT EVENTS <span className="meta">{shown.length} shown</span></div></div>
                <div className="panel-body flush">
                  {rows === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : shown.length === 0 ? <div className="empty">No matching audit events.</div> : (
                    <table className="tbl"><thead><tr><th style={{ width: 150 }}>WHEN</th><th>ACTOR</th><th>AGENT</th><th>KIND</th><th>OK</th><th>DETAIL</th></tr></thead>
                      <tbody>{shown.map((r) => (
                        <tr key={r.id}>
                          <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(r.ts)}</td>
                          <td className="mono">{r.actor || '—'}</td>
                          <td className="mono muted">{r.agent_id ? r.agent_id.replace(/-id$/, '') : '—'}</td>
                          <td className="mono">{r.kind}</td>
                          <td className="mono" style={{ color: r.ok ? 'var(--accent)' : 'var(--err)' }}>{r.ok ? '✓' : '✗'}</td>
                          <td className="mono muted" style={{ fontSize: 11, wordBreak: 'break-all' }}>{r.detail || '—'}</td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
