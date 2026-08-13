'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface Entry { id: number; login: string; actions: string; resources: string; reason: string; duration: number; status: string; approved_by: string | null; created_at: number; expires_at: number | null }
interface Scope { id: number; name: string; actions: string; resources: string; risk_level: string; max_duration: number; require_approval: number }

const fmtTs = (t: number) => new Date(t * 1000).toLocaleString();
const jl = (s: string) => { try { const v = JSON.parse(s); return Array.isArray(v) ? v.join(', ') : String(s); } catch { return s; } };
const jarr = (s: string): string[] => { try { const v = JSON.parse(s); return Array.isArray(v) ? v : []; } catch { return []; } };

export default function BreakGlassPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [pending, setPending] = useState<Entry[] | null>(null);
  const [active, setActive] = useState<Entry[]>([]);
  const [history, setHistory] = useState<Entry[]>([]);
  const [scopes, setScopes] = useState<Scope[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  // request form
  const [actions, setActions] = useState('');
  const [resources, setResources] = useState('');
  const [reason, setReason] = useState('');
  const [amount, setAmount] = useState(60);
  const [unit, setUnit] = useState(60); // seconds multiplier

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [p, a, h, s] = await Promise.all([
        apiFetch('/api/ee/break-glass/pending'),
        apiFetch('/api/ee/break-glass/active'),
        apiFetch('/api/ee/break-glass/history'),
        apiFetch('/api/ee/break-glass/scopes'),
      ]);
      if (p.ok) setPending(await p.json()); else { setError(`HTTP ${p.status}`); setPending([]); }
      if (a.ok) setActive(await a.json());
      if (h.ok) setHistory(await h.json());
      if (s.ok) setScopes(await s.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setPending([]); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  const applyScope = (id: string) => {
    const sc = scopes.find((x) => String(x.id) === id);
    if (!sc) return;
    setActions(jarr(sc.actions).join(', '));
    setResources(jarr(sc.resources).join(', '));
  };

  const submit = async () => {
    setMsg(null); setError(null);
    const acts = actions.split(',').map((s) => s.trim()).filter(Boolean);
    const res = resources.split(',').map((s) => s.trim()).filter(Boolean);
    if (acts.length === 0) { setError('specify at least one action'); return; }
    if (!reason.trim()) { setError('a reason is required'); return; }
    try {
      const r = await apiFetch('/api/ee/break-glass/request', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ actions: acts, resources: res.length ? res : ['*'], reason: reason.trim(), duration: amount * unit }),
      });
      if (!r.ok) { setError(await r.text() || `HTTP ${r.status}`); return; }
      setMsg('break-glass request submitted'); setActions(''); setResources(''); setReason(''); await load();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const act = async (id: number, kind: 'approve' | 'deny' | 'revoke') => {
    setMsg(null); setError(null);
    try {
      const opts: RequestInit = { method: 'POST', headers: { 'content-type': 'application/json' } };
      if (kind !== 'approve') { const reason = prompt(`${kind} reason (optional):`) ?? ''; opts.body = JSON.stringify({ reason }); }
      const r = await apiFetch(`/api/ee/break-glass/${id}/${kind}`, opts);
      if (!r.ok) { setError(await r.text() || `HTTP ${r.status}`); return; }
      setMsg(`#${id} ${kind}d`); await load();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const stColor = (s: string) => (s === 'active' || s === 'approved' ? 'var(--accent)' : s === 'denied' || s === 'revoked' ? 'var(--err)' : s === 'pending' ? 'var(--warn)' : 'var(--fg-2)');
  const riskColor = (r: string) => (r === 'high' ? 'var(--err)' : r === 'medium' ? 'var(--warn)' : 'var(--fg-2)');

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/break-glass requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">break-glass access</span>
          </div>
          <div className="topbar-actions"><button className="btn" onClick={load}>↻ refresh</button></div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="break-glass" label="Break-glass Access">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {msg && <div className="panel" style={{ borderColor: 'var(--accent-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--accent)' }}>{msg}</div></div>}

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">⊗</span> REQUEST EMERGENCY ACCESS</div></div>
                <div className="panel-body" style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
                  {scopes.length > 0 && (
                    <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                      <span className="mono muted" style={{ fontSize: 11, width: 60 }}>scope</span>
                      <select className="input" defaultValue="" onChange={(e) => applyScope(e.target.value)} style={{ width: 260 }}>
                        <option value="">— pick a predefined scope —</option>
                        {scopes.map((s) => <option key={s.id} value={s.id}>{s.name} ({s.risk_level})</option>)}
                      </select>
                    </div>
                  )}
                  <input className="input" placeholder="actions (comma) e.g. agent:Terminal, container:Exec" value={actions} onChange={(e) => setActions(e.target.value)} />
                  <input className="input" placeholder="resources (comma) default *" value={resources} onChange={(e) => setResources(e.target.value)} />
                  <input className="input" placeholder="reason (required — recorded for audit)" value={reason} onChange={(e) => setReason(e.target.value)} />
                  <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                    <span className="mono muted" style={{ fontSize: 11, width: 60 }}>duration</span>
                    <input className="input" type="number" min={1} value={amount} onChange={(e) => setAmount(Number(e.target.value) || 1)} style={{ width: 90 }} />
                    <select className="input" value={unit} onChange={(e) => setUnit(Number(e.target.value))} style={{ width: 110 }}>
                      <option value={60}>minutes</option>
                      <option value={3600}>hours</option>
                    </select>
                    <div style={{ flex: 1 }} />
                    <button className="btn btn-accent" onClick={submit}>request access</button>
                  </div>
                </div>
              </div>

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">◷</span> PENDING</div></div>
                <div className="panel-body flush">
                  {pending === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : pending.length === 0 ? <div className="empty">No pending requests.</div> : (
                    <table className="tbl"><thead><tr><th>USER</th><th>ACTIONS</th><th>REASON</th><th>DUR</th><th style={{ width: 150 }}>DECISION</th></tr></thead>
                      <tbody>{pending.map((e) => (
                        <tr key={e.id}>
                          <td className="mono">{e.login}</td>
                          <td className="mono">{jl(e.actions)}</td>
                          <td className="mono muted">{e.reason}</td>
                          <td className="mono muted">{Math.round(e.duration / 60)}m</td>
                          <td style={{ whiteSpace: 'nowrap' }}>
                            <button className="btn btn-sm btn-accent" onClick={() => act(e.id, 'approve')}>approve</button>{' '}
                            <button className="btn btn-sm" style={{ color: 'var(--err)' }} onClick={() => act(e.id, 'deny')}>deny</button>
                          </td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">●</span> ACTIVE GRANTS</div></div>
                <div className="panel-body flush">
                  {active.length === 0 ? <div className="empty">No active grants.</div> : (
                    <table className="tbl"><thead><tr><th>USER</th><th>ACTIONS</th><th>APPROVED BY</th><th>EXPIRES</th><th style={{ width: 80 }}></th></tr></thead>
                      <tbody>{active.map((e) => (
                        <tr key={e.id}>
                          <td className="mono">{e.login}</td>
                          <td className="mono">{jl(e.actions)}</td>
                          <td className="mono muted">{e.approved_by || '—'}</td>
                          <td className="mono" style={{ color: 'var(--warn)', fontSize: 11 }}>{e.expires_at ? fmtTs(e.expires_at) : '—'}</td>
                          <td><button className="btn btn-sm" style={{ color: 'var(--err)' }} onClick={() => act(e.id, 'revoke')}>revoke</button></td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">↻</span> HISTORY</div></div>
                <div className="panel-body flush">
                  {history.length === 0 ? <div className="empty">No history.</div> : (
                    <table className="tbl"><thead><tr><th>WHEN</th><th>USER</th><th>ACTIONS</th><th>STATUS</th></tr></thead>
                      <tbody>{history.map((e) => (
                        <tr key={e.id}>
                          <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(e.created_at)}</td>
                          <td className="mono">{e.login}</td>
                          <td className="mono muted">{jl(e.actions)}</td>
                          <td className="mono" style={{ color: stColor(e.status) }}>{e.status}</td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>

              {scopes.length > 0 && (
                <div className="panel">
                  <div className="panel-head"><div className="panel-title"><span className="ico">◇</span> SCOPES</div></div>
                  <div className="panel-body flush">
                    <table className="tbl"><thead><tr><th>NAME</th><th>RISK</th><th>ACTIONS</th><th>MAX DURATION</th><th>APPROVAL</th></tr></thead>
                      <tbody>{scopes.map((s) => (
                        <tr key={s.id}>
                          <td className="mono">{s.name}</td>
                          <td className="mono" style={{ color: riskColor(s.risk_level) }}>{s.risk_level}</td>
                          <td className="mono muted">{jl(s.actions)}</td>
                          <td className="mono muted">{Math.round(s.max_duration / 60)}m</td>
                          <td className="mono">{s.require_approval ? 'required' : 'auto'}</td>
                        </tr>
                      ))}</tbody></table>
                  </div>
                </div>
              )}
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
