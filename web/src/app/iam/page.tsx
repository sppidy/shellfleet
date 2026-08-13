'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import type { IamPolicySummary } from '@/lib/iamPolicy';
import { Loader2Icon } from 'lucide-react';

export default function IamPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [policies, setPolicies] = useState<IamPolicySummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/ee/iam/policies');
      if (!res.ok) { setError(`HTTP ${res.status}`); setPolicies([]); return; }
      setPolicies(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setPolicies([]); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/iam requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  const formatDate = (ts: number) => {
    if (!ts || ts === 0) return '—';
    return new Date(ts * 1000).toISOString().slice(0, 10);
  };

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">iam policies</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={load}>↻ refresh</button>
            <button className="btn btn-accent" onClick={() => router.push('/iam/new')}>+ create policy</button>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="acl" label="IAM Policies">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">⊡</span> IAM POLICIES</div></div>
                <div className="panel-body flush">
                  {policies === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : policies.length === 0 ? <div className="empty">No policies.</div> : (
                    <table className="tbl"><thead><tr><th>NAME</th><th>DESCRIPTION</th><th>STATEMENTS</th><th>CREATED</th></tr></thead>
                      <tbody>{policies.map((p) => (
                        <tr key={p.id} className="clickable" onClick={() => router.push(`/iam/${p.id}`)} style={{ cursor: 'pointer' }}>
                          <td className="mono">
                            {p.managed && <span title="Managed — read-only" style={{ marginRight: 4 }}>⊠</span>}
                            {p.name}
                          </td>
                          <td className="mono muted">{p.description || '—'}</td>
                          <td className="mono">{p.statement_count}</td>
                          <td className="mono muted">{formatDate(p.created_at)}</td>
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
