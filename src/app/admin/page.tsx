'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

interface UserRow {
  login: string;
  role: 'admin' | 'viewer';
  totp_enabled: number;
  created_at: number;
  last_login_at: number;
}

interface UsersResponse {
  users: UserRow[];
  seat_limit: number;
  seats_used: number;
}

const RELATIVE = (ts: number) => {
  if (!ts) return 'never';
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86_400) return `${Math.floor(delta / 3_600)}h ago`;
  return `${Math.floor(delta / 86_400)}d ago`;
};

export default function AdminPage() {
  const router = useRouter();
  const { user: currentUser, role, status } = useSession();
  const [data, setData] = useState<UsersResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);

  // EE RBAC state
  const [eeAvailable, setEeAvailable] = useState(false);

  // Invite state
  const [invites, setInvites] = useState<{ code: string; role: string; created_by: string; expires_at: number; used_by: string | null }[]>([]);

  // Tenancy state
  const [orgs, setOrgs] = useState<{ id: number; name: string; slug: string }[]>([]);
  const [newOrgName, setNewOrgName] = useState('');
  const [newOrgSlug, setNewOrgSlug] = useState('');
  const [selectedOrg, setSelectedOrg] = useState<{ id: number; name: string } | null>(null);
  const [orgMembers, setOrgMembers] = useState<{ login: string; role_in_org: string }[]>([]);
  const [orgAgents, setOrgAgents] = useState<string[]>([]);
  const [addMemberLogin, setAddMemberLogin] = useState('');
  const [addAgentId, setAddAgentId] = useState('');
  const [allAgents, setAllAgents] = useState<string[]>([]);
  const [inviteRole, setInviteRole] = useState('viewer');
  const [lastInviteUrl, setLastInviteUrl] = useState<string | null>(null);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
    if (status === 'pending_mfa') router.replace('/mfa');
  }, [status, router]);

  const fetchUsers = useCallback(async () => {
    setError(null);
    try {
      const res = await fetch('/api/users', { credentials: 'same-origin' });
      if (res.status === 401) { window.location.href = '/login'; return; }
      if (res.status === 403) { setError('admin access required'); return; }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setData((await res.json()) as UsersResponse);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  }, []);

  const checkEe = useCallback(async () => {
    try {
      const res = await apiFetch('/api/ee/acl/actions');
      if (res.ok) setEeAvailable(true);
    } catch { /* ignore */ }
  }, []);

  const fetchInvites = useCallback(async () => {
    try {
      const res = await apiFetch('/api/invites');
      if (res.ok) setInvites(await res.json());
    } catch { /* ignore */ }
  }, []);

  const createInvite = async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/invites', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ role: inviteRole, ttl_hours: 24 }),
      });
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      setLastInviteUrl(`${window.location.origin}${data.url}`);
      await fetchInvites();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const deleteInvite = async (code: string) => {
    try {
      await apiFetch(`/api/invites/${code}`, { method: 'DELETE' });
      await fetchInvites();
    } catch { /* ignore */ }
  };

  const fetchOrgs = useCallback(async () => {
    try {
      const res = await apiFetch('/api/ee/tenancy/orgs');
      if (res.ok) setOrgs(await res.json());
    } catch { /* ignore */ }
    try {
      const tRes = await apiFetch('/api/tokens');
      if (tRes.ok) {
        const tokens: { hostname?: string }[] = await tRes.json();
        setAllAgents(tokens.filter(t => t.hostname).map(t => `${t.hostname}-id`));
      }
    } catch { /* ignore */ }
  }, []);

  const fetchOrgDetails = useCallback(async (orgId: number) => {
    try {
      const [mRes, aRes] = await Promise.all([
        apiFetch(`/api/ee/tenancy/orgs/${orgId}/members`),
        apiFetch(`/api/ee/tenancy/orgs/${orgId}/agents`),
      ]);
      if (mRes.ok) setOrgMembers(await mRes.json());
      if (aRes.ok) setOrgAgents(await aRes.json());
    } catch { /* ignore */ }
  }, []);

  const createOrg = async () => {
    if (!newOrgName.trim() || !newOrgSlug.trim()) return;
    try {
      await apiFetch('/api/ee/tenancy/orgs', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ name: newOrgName.trim(), slug: newOrgSlug.trim() }),
      });
      setNewOrgName(''); setNewOrgSlug('');
      await fetchOrgs();
    } catch { /* ignore */ }
  };

  const addOrgMember = async () => {
    if (!selectedOrg || !addMemberLogin.trim()) return;
    await apiFetch(`/api/ee/tenancy/orgs/${selectedOrg.id}/members`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ login: addMemberLogin.trim() }),
    });
    setAddMemberLogin('');
    fetchOrgDetails(selectedOrg.id);
  };

  const addOrgAgent = async () => {
    if (!selectedOrg || !addAgentId.trim()) return;
    await apiFetch(`/api/ee/tenancy/orgs/${selectedOrg.id}/agents`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ agent_id: addAgentId.trim() }),
    });
    setAddAgentId('');
    fetchOrgDetails(selectedOrg.id);
  };

  useEffect(() => {
    if (status === 'authed') { fetchUsers(); checkEe(); fetchInvites(); fetchOrgs(); }
  }, [status, fetchUsers, checkEe, fetchInvites, fetchOrgs]);

  useEffect(() => {
    if (selectedOrg) fetchOrgDetails(selectedOrg.id);
  }, [selectedOrg, fetchOrgDetails]);

  const setRole = useCallback(async (login: string, newRole: 'admin' | 'viewer') => {
    setPending(login); setError(null);
    try {
      const res = await apiFetch(`/api/users/${encodeURIComponent(login)}`, {
        method: 'PUT', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ role: newRole }),
      });
      if (!res.ok) throw new Error(await res.text() || `HTTP ${res.status}`);
      await fetchUsers();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
    finally { setPending(null); }
  }, [fetchUsers]);

  const removeUser = useCallback(async (login: string) => {
    if (!confirm(`Remove seat for ${login}?`)) return;
    setPending(login); setError(null);
    try {
      const res = await apiFetch(`/api/users/${encodeURIComponent(login)}`, { method: 'DELETE' });
      if (!res.ok) throw new Error(await res.text() || `HTTP ${res.status}`);
      await fetchUsers();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
    finally { setPending(null); }
  }, [fetchUsers]);


  if (status !== 'authed') {
    return (<div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>);
  }
  if (role !== 'admin') {
    return (<div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/admin requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>);
  }

  const seats = data ? `${data.seats_used} / ${data.seat_limit}` : '—';
  const seatsFull = data ? data.seats_used >= data.seat_limit : false;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">admin</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={() => { fetchUsers(); fetchInvites(); fetchOrgs(); }}>↻ refresh</button>
          </div>
        </div>

        <div className="scroll">
          <div className="pane">
            {error && (
              <div className="panel" style={{ borderColor: 'var(--err-bd)' }}>
                <div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div>
              </div>
            )}

            {/* SEATS + USERS */}
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title">
                  <span className="ico">≡</span> USERS
                  <span className="meta" style={{ color: seatsFull ? 'var(--warn)' : 'var(--fg-2)' }}>
                    {seats} seats {seatsFull ? '· cap reached' : ''}
                  </span>
                </div>
              </div>
              <div className="panel-body flush">
                {data === null ? (
                  <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                ) : data.users.length === 0 ? (
                  <div className="empty">No users yet.</div>
                ) : (
                  <table className="tbl">
                    <thead>
                      <tr>
                        <th>LOGIN</th>
                        <th style={{ width: 100 }}>ROLE</th>
                        <th style={{ width: 80 }}>2FA</th>
                        <th style={{ width: 100 }}>LAST SEEN</th>
                        <th style={{ width: 260 }}>ACTIONS</th>
                      </tr>
                    </thead>
                    <tbody>
                      {data.users.map((u) => {
                        const isSelf = u.login === currentUser;
                        const isPending = pending === u.login;
                        return (
                          <tr key={u.login}>
                            <td className="mono" style={{ color: 'var(--fg)' }}>
                              {u.login}{isSelf ? <span className="muted"> (you)</span> : null}
                            </td>
                            <td className="mono" style={{ color: u.role === 'admin' ? 'var(--accent)' : 'var(--fg-2)' }}>
                              {u.role}
                            </td>
                            <td className="mono">{u.totp_enabled ? '✓' : '—'}</td>
                            <td className="mono muted">{RELATIVE(u.last_login_at)}</td>
                            <td>
                              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                                {u.role === 'viewer' ? (
                                  <button className="btn" disabled={isPending} onClick={() => setRole(u.login, 'admin')}>
                                    {isPending ? '…' : 'promote'}
                                  </button>
                                ) : (
                                  <button className="btn" disabled={isPending} onClick={() => setRole(u.login, 'viewer')}>
                                    {isPending ? '…' : 'demote'}
                                  </button>
                                )}
                                <button className="btn" style={{ borderColor: 'var(--err-bd)', color: 'var(--err)' }} disabled={isPending || isSelf} onClick={() => removeUser(u.login)}>
                                  remove
                                </button>
                              </div>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
              </div>
            </div>

            {/* INVITES (EE only) */}
            {eeAvailable && <div className="panel" style={{ marginTop: 12 }}>
              <div className="panel-head">
                <div className="panel-title"><span className="ico">✉</span> INVITE LINKS</div>
              </div>
              <div className="panel-body" style={{ padding: 12 }}>
                <div style={{ display: 'flex', gap: 6, marginBottom: 10, alignItems: 'center' }}>
                  <select className="input" value={inviteRole} onChange={(e) => setInviteRole(e.target.value)} style={{ width: 100 }}>
                    <option value="viewer">viewer</option>
                    <option value="admin">admin</option>
                  </select>
                  <button className="btn btn-accent" onClick={createInvite}>generate invite link</button>
                  {lastInviteUrl && (
                    <button className="btn" onClick={() => { navigator.clipboard.writeText(lastInviteUrl); }}>
                      copy link
                    </button>
                  )}
                </div>
                {lastInviteUrl && (
                  <div className="mono" style={{ fontSize: 12, color: 'var(--accent)', marginBottom: 10, wordBreak: 'break-all' }}>
                    {lastInviteUrl}
                  </div>
                )}
                {invites.length > 0 && (
                  <table className="tbl">
                    <thead><tr><th>CODE</th><th>ROLE</th><th>STATUS</th><th style={{ width: 40 }}></th></tr></thead>
                    <tbody>
                      {invites.map((inv) => {
                        const expired = Math.floor(Date.now() / 1000) > inv.expires_at;
                        const used = !!inv.used_by;
                        return (
                          <tr key={inv.code}>
                            <td className="mono" style={{ fontSize: 12 }}>{inv.code.slice(0, 8)}...</td>
                            <td className="mono">{inv.role}</td>
                            <td className="mono" style={{ color: used ? 'var(--fg-2)' : expired ? 'var(--err)' : 'var(--accent)' }}>
                              {used ? `used by ${inv.used_by}` : expired ? 'expired' : 'active'}
                            </td>
                            <td><button className="btn btn-sm" style={{ color: 'var(--err)', padding: '2px 6px' }} onClick={() => deleteInvite(inv.code)}>✕</button></td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
              </div>
            </div>}

            {/* EE: Orgs */}
            {eeAvailable && (<>

              {/* TENANCY */}
              <div className="panel" style={{ marginTop: 12 }}>
                <div className="panel-head">
                  <div className="panel-title"><span className="ico">◈</span> ORGANIZATIONS <span className="meta">EE</span></div>
                </div>
                <div className="panel-body" style={{ padding: 12 }}>
                  <div style={{ display: 'flex', gap: 6, marginBottom: 12 }}>
                    <input className="input" placeholder="Org name" value={newOrgName} onChange={(e) => setNewOrgName(e.target.value)} style={{ flex: 1 }} />
                    <input className="input" placeholder="slug" value={newOrgSlug} onChange={(e) => setNewOrgSlug(e.target.value)} style={{ width: 100 }} />
                    <button className="btn btn-accent" onClick={createOrg}>+</button>
                  </div>
                  <div style={{ display: 'grid', gridTemplateColumns: '200px 1fr', gap: 12 }}>
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                      {orgs.length === 0 ? (
                        <div className="mono muted" style={{ fontSize: 12 }}>No orgs yet.</div>
                      ) : orgs.map((o) => (
                        <div key={o.id} style={{ padding: '4px 8px', borderRadius: 4, cursor: 'pointer', background: selectedOrg?.id === o.id ? 'var(--bg-2)' : 'transparent' }} onClick={() => setSelectedOrg(o)}>
                          <span className="mono" style={{ fontSize: 13, color: 'var(--fg)' }}>{o.name}</span>
                          <span className="mono muted" style={{ fontSize: 11, marginLeft: 6 }}>{o.slug}</span>
                        </div>
                      ))}
                    </div>
                    <div>
                      {!selectedOrg ? (
                        <div className="mono muted" style={{ fontSize: 12 }}>Select an org.</div>
                      ) : (
                        <>
                          <div style={{ marginBottom: 10 }}>
                            <div className="mono muted" style={{ fontSize: 11, marginBottom: 4 }}>MEMBERS</div>
                            <div style={{ display: 'flex', gap: 6, marginBottom: 6 }}>
                              <select className="input" value={addMemberLogin} onChange={(e) => setAddMemberLogin(e.target.value)} style={{ flex: 1 }}>
                                <option value="">— select user —</option>
                                {data?.users.filter(u => !orgMembers.some(m => m.login === u.login)).map(u => (
                                  <option key={u.login} value={u.login}>{u.login}</option>
                                ))}
                              </select>
                              <button className="btn btn-accent" onClick={addOrgMember} disabled={!addMemberLogin}>add</button>
                            </div>
                            {orgMembers.map((m) => (
                              <div key={m.login} className="mono" style={{ fontSize: 12, color: 'var(--fg-1)' }}>{m.login} <span className="muted">({m.role_in_org})</span></div>
                            ))}
                          </div>
                          <div>
                            <div className="mono muted" style={{ fontSize: 11, marginBottom: 4 }}>AGENTS</div>
                            <div style={{ display: 'flex', gap: 6, marginBottom: 6 }}>
                              <select className="input" value={addAgentId} onChange={(e) => setAddAgentId(e.target.value)} style={{ flex: 1 }}>
                                <option value="">— select agent —</option>
                                {allAgents.filter(a => !orgAgents.includes(a)).map(a => (
                                  <option key={a} value={a}>{a.replace(/-id$/, '')}</option>
                                ))}
                              </select>
                              <button className="btn btn-accent" onClick={addOrgAgent} disabled={!addAgentId}>add</button>
                            </div>
                            {orgAgents.map((a) => (
                              <div key={a} className="mono" style={{ fontSize: 12, color: 'var(--fg-1)', display: 'flex', alignItems: 'center', gap: 6 }}>
                                {a.replace(/-id$/, '')}
                              </div>
                            ))}
                          </div>
                        </>
                      )}
                    </div>
                  </div>
                </div>
              </div>
            </>)}
          </div>
        </div>
      </main>
    </div>
  );
}
