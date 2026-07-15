'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface ApprovalRequest { id: number; requester: string; action: string; resource: string; agent_id: string | null; status: string; created_at: number; expires_at: number; decided_by: string | null; deny_reason: string | null }
interface ApprovalRule { id: number; actions: string; resources: string; approver_role: string; min_approvals: number; enabled: number }

const fmtTs = (t: number) => new Date(t * 1000).toLocaleString();
const jl = (s: string) => { try { const v = JSON.parse(s); return Array.isArray(v) ? v.join(', ') : String(s); } catch { return s; } };

export default function ApprovalsPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [pending, setPending] = useState<ApprovalRequest[] | null>(null);
  const [history, setHistory] = useState<ApprovalRequest[]>([]);
  const [rules, setRules] = useState<ApprovalRule[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  // new rule form
  const [rActions, setRActions] = useState('');
  const [rResources, setRResources] = useState('');
  const [rRole, setRRole] = useState('admin');
  const [rMin, setRMin] = useState(1);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [p, h, r] = await Promise.all([
        apiFetch('/api/ee/approvals/pending'),
        apiFetch('/api/ee/approvals/history'),
        apiFetch('/api/ee/approvals/rules'),
      ]);
      if (p.ok) setPending(await p.json()); else { setError(`HTTP ${p.status}`); setPending([]); }
      if (h.ok) setHistory(await h.json());
      if (r.ok) setRules(await r.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setPending([]); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  const decide = async (id: number, kind: 'approve' | 'deny') => {
    setMsg(null); setError(null);
    try {
      const opts: RequestInit = { method: 'POST', headers: { 'content-type': 'application/json' } };
      if (kind === 'deny') {
        const reason = prompt('Deny reason (optional):') ?? '';
        opts.body = JSON.stringify({ reason });
      }
      const res = await apiFetch(`/api/ee/approvals/${id}/${kind}`, opts);
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      setMsg(`request #${id} ${kind === 'approve' ? 'approved' : 'denied'}`);
      await load();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const addRule = async () => {
    const actions = rActions.split(',').map((s) => s.trim()).filter(Boolean);
    const resources = rResources.split(',').map((s) => s.trim()).filter(Boolean);
    if (actions.length === 0) { setError('rule needs at least one action'); return; }
    try {
      const res = await apiFetch('/api/ee/approvals/rules', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ actions, resources: resources.length ? resources : ['*'], approver_role: rRole, min_approvals: rMin }),
      });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      setRActions(''); setRResources(''); setMsg('rule created'); await load();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const stColor = (s: string) => (s === 'approved' ? 'var(--accent)' : s === 'denied' ? 'var(--err)' : s === 'pending' ? 'var(--warn)' : 'var(--fg-2)');

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/approvals requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">approval workflows</span>
          </div>
          <div className="topbar-actions"><button className="btn" onClick={load}>↻ refresh</button></div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="approvals" label="Approval Workflows">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {msg && <div className="panel" style={{ borderColor: 'var(--accent-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--accent)' }}>{msg}</div></div>}

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">◷</span> PENDING</div></div>
                <div className="panel-body flush">
                  {pending === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : pending.length === 0 ? <div className="empty">No pending requests.</div> : (
                    <table className="tbl"><thead><tr><th>REQUESTER</th><th>ACTION</th><th>RESOURCE</th><th>AGENT</th><th>EXPIRES</th><th style={{ width: 150 }}>DECISION</th></tr></thead>
                      <tbody>{pending.map((r) => (
                        <tr key={r.id}>
                          <td className="mono">{r.requester}</td>
                          <td className="mono">{r.action}</td>
                          <td className="mono muted">{r.resource}</td>
                          <td className="mono muted">{r.agent_id ? r.agent_id.replace(/-id$/, '') : '—'}</td>
                          <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(r.expires_at)}</td>
                          <td style={{ whiteSpace: 'nowrap' }}>
                            <button className="btn btn-sm btn-accent" onClick={() => decide(r.id, 'approve')}>approve</button>{' '}
                            <button className="btn btn-sm" style={{ color: 'var(--err)' }} onClick={() => decide(r.id, 'deny')}>deny</button>
                          </td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">⛨</span> APPROVAL RULES</div></div>
                <div className="panel-body" style={{ padding: 12 }}>
                  <div style={{ display: 'flex', gap: 6, marginBottom: 10, flexWrap: 'wrap', alignItems: 'end' }}>
                    <input className="input" placeholder="actions (comma) e.g. agent:Terminal" value={rActions} onChange={(e) => setRActions(e.target.value)} style={{ width: 220 }} />
                    <input className="input" placeholder="resources (comma) default *" value={rResources} onChange={(e) => setRResources(e.target.value)} style={{ width: 180 }} />
                    <select className="input" value={rRole} onChange={(e) => setRRole(e.target.value)} style={{ width: 110 }}><option value="admin">admin</option><option value="viewer">viewer</option></select>
                    <input className="input" type="number" min={1} value={rMin} onChange={(e) => setRMin(Number(e.target.value) || 1)} style={{ width: 70 }} title="min approvals" />
                    <button className="btn btn-accent" onClick={addRule}>add rule</button>
                  </div>
                  {rules.length === 0 ? <div className="mono muted" style={{ fontSize: 12 }}>No rules — actions are not gated by approval.</div> : (
                    <table className="tbl"><thead><tr><th>ACTIONS</th><th>RESOURCES</th><th>APPROVER</th><th>MIN</th></tr></thead>
                      <tbody>{rules.map((r) => (
                        <tr key={r.id}><td className="mono">{jl(r.actions)}</td><td className="mono muted">{jl(r.resources)}</td><td className="mono">{r.approver_role}</td><td className="mono">{r.min_approvals}</td></tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>

              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">↻</span> HISTORY</div></div>
                <div className="panel-body flush">
                  {history.length === 0 ? <div className="empty">No decisions yet.</div> : (
                    <table className="tbl"><thead><tr><th>WHEN</th><th>REQUESTER</th><th>ACTION</th><th>STATUS</th><th>BY</th></tr></thead>
                      <tbody>{history.map((r) => (
                        <tr key={r.id}>
                          <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(r.created_at)}</td>
                          <td className="mono">{r.requester}</td>
                          <td className="mono muted">{r.action}</td>
                          <td className="mono" style={{ color: stColor(r.status) }}>{r.status}</td>
                          <td className="mono muted">{r.decided_by || '—'}</td>
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
