'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter, useParams } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import type { IamPolicyDetail } from '@/lib/iamPolicy';
import { Loader2Icon } from 'lucide-react';

export default function IamDetailPage() {
  const router = useRouter();
  const params = useParams();
  const id = params.id as string;
  const { role, status } = useSession();
  const [policy, setPolicy] = useState<IamPolicyDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch(`/api/ee/iam/policies/${id}`);
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      setPolicy(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  }, [id]);

  useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  const doDelete = async () => {
    if (!confirm('Delete this policy? This cannot be undone.')) return;
    setDeleting(true);
    setError(null);
    try {
      const res = await apiFetch(`/api/ee/iam/policies/${id}`, { method: 'DELETE' });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); setDeleting(false); return; }
      router.push('/iam');
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setDeleting(false); }
  };

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
            <button type="button" className="nav-item" onClick={() => router.push('/iam')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>iam</button>
            <span className="sep">/</span>
            <span className="here">{policy?.name || id}</span>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="acl" label="IAM Policies">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {policy === null && !error ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div> : policy && (
                <>
                  <div className="panel" style={{ marginBottom: 12 }}>
                    <div className="panel-head">
                      <div className="panel-title">
                        {policy.managed && <span title="Managed — read-only" style={{ marginRight: 6 }}>⊠</span>}
                        {policy.name}
                        {policy.managed && <span className="meta" style={{ color: 'var(--warn)', marginLeft: 8 }}>Managed — read-only</span>}
                        {!policy.managed && policy.resource_bound && <span className="meta" style={{ color: 'var(--warn)', marginLeft: 8 }}>Resource-scoped — read-only in v1</span>}
                      </div>
                      <div className="panel-actions">
                        {!policy.managed && !policy.resource_bound && (
                          <button className="btn btn-sm" onClick={() => router.push(`/iam/${id}/edit`)}>edit</button>
                        )}
                        {!policy.managed && (
                          <button className="btn btn-sm" style={{ color: 'var(--err)' }} onClick={doDelete} disabled={deleting}>{deleting ? 'deleting…' : 'delete'}</button>
                        )}
                      </div>
                    </div>
                    <div className="panel-body">
                      <div className="field" style={{ marginBottom: 8 }}>
                        <span className="mono muted" style={{ fontSize: 11 }}>Description</span>
                        <div className="mono">{policy.description || '—'}</div>
                      </div>
                      <div className="field">
                        <span className="mono muted" style={{ fontSize: 11 }}>Created</span>
                        <div className="mono">{formatDate(policy.created_at)}</div>
                      </div>
                    </div>
                  </div>
                  <div className="panel">
                    <div className="panel-head"><div className="panel-title">Statements ({policy.statement_count})</div></div>
                    <div className="panel-body flush">
                      {policy.statements.length === 0 ? <div className="empty">No statements.</div> : (
                        <table className="tbl"><thead><tr><th style={{ width: 80 }}>EFFECT</th><th>ACTIONS</th><th>RESOURCES</th></tr></thead>
                          <tbody>{policy.statements.map((s) => (
                            <tr key={s.id}>
                              <td className="mono" style={{ color: s.effect === 'Allow' ? 'var(--accent)' : 'var(--err)' }}>{s.effect}</td>
                              <td className="mono" style={{ fontSize: 11 }}>{s.actions.join(', ')}</td>
                              <td className="mono muted" style={{ fontSize: 11 }}>{s.resources.join(', ')}</td>
                            </tr>
                          ))}</tbody></table>
                      )}
                    </div>
                  </div>
                </>
              )}
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
