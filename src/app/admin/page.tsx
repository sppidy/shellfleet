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

interface EeRole {
  id: number;
  name: string;
  description: string | null;
}

interface EePermission {
  id: number;
  role_id: number;
  resource_type: string;
  resource_pattern: string;
  action: string;
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
  const [roles, setRoles] = useState<EeRole[]>([]);
  const [selectedRole, setSelectedRole] = useState<EeRole | null>(null);
  const [permissions, setPermissions] = useState<EePermission[]>([]);
  const [newRoleName, setNewRoleName] = useState('');
  const [newRoleDesc, setNewRoleDesc] = useState('');
  const [newPerm, setNewPerm] = useState({ resource_type: 'agent', resource_pattern: '*', action: '*' });
  const [assignLogin, setAssignLogin] = useState('');

  // Invite state
  const [invites, setInvites] = useState<{ code: string; role: string; created_by: string; expires_at: number; used_by: string | null }[]>([]);
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

  const fetchRoles = useCallback(async () => {
    try {
      const res = await apiFetch('/api/ee/rbac/roles');
      if (res.status === 404 || res.status === 502) { setEeAvailable(false); return; }
      if (!res.ok) return;
      setEeAvailable(true);
      setRoles(await res.json());
    } catch { setEeAvailable(false); }
  }, []);

  const fetchPermissions = useCallback(async (roleId: number) => {
    try {
      const res = await apiFetch(`/api/ee/rbac/roles/${roleId}/permissions`);
      if (!res.ok) return;
      setPermissions(await res.json());
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

  useEffect(() => {
    if (status === 'authed') { fetchUsers(); fetchRoles(); fetchInvites(); }
  }, [status, fetchUsers, fetchRoles, fetchInvites]);

  useEffect(() => {
    if (selectedRole) fetchPermissions(selectedRole.id);
    else setPermissions([]);
  }, [selectedRole, fetchPermissions]);

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

  const createRole = async () => {
    if (!newRoleName.trim()) return;
    setError(null);
    try {
      const res = await apiFetch('/api/ee/rbac/roles', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ name: newRoleName.trim(), description: newRoleDesc.trim() || null }),
      });
      if (!res.ok) throw new Error(await res.text());
      setNewRoleName(''); setNewRoleDesc('');
      await fetchRoles();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const deleteRole = async (id: number) => {
    setError(null);
    try {
      await apiFetch(`/api/ee/rbac/roles/${id}`, { method: 'DELETE' });
      if (selectedRole?.id === id) setSelectedRole(null);
      await fetchRoles();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const addPermission = async () => {
    if (!selectedRole) return;
    setError(null);
    try {
      const res = await apiFetch(`/api/ee/rbac/roles/${selectedRole.id}/permissions`, {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify(newPerm),
      });
      if (!res.ok) throw new Error(await res.text());
      await fetchPermissions(selectedRole.id);
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const removePermission = async (permId: number) => {
    try {
      await apiFetch(`/api/ee/rbac/permissions/${permId}`, { method: 'DELETE' });
      if (selectedRole) await fetchPermissions(selectedRole.id);
    } catch { /* ignore */ }
  };

  const assignRole = async () => {
    if (!selectedRole || !assignLogin.trim()) return;
    try {
      await apiFetch('/api/ee/rbac/assignments', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ login: assignLogin.trim(), role_id: selectedRole.id }),
      });
      setAssignLogin('');
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

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
            <button className="btn" onClick={() => { fetchUsers(); fetchRoles(); }}>↻ refresh</button>
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

            {/* EE ROLES + PERMISSIONS */}
            {eeAvailable && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginTop: 12 }}>
                <div className="panel">
                  <div className="panel-head">
                    <div className="panel-title"><span className="ico">⚙</span> ROLES <span className="meta">EE</span></div>
                  </div>
                  <div className="panel-body" style={{ padding: 12 }}>
                    <div style={{ display: 'flex', gap: 6, marginBottom: 12 }}>
                      <input className="input" placeholder="Role name" value={newRoleName} onChange={(e) => setNewRoleName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && createRole()} style={{ flex: 1 }} />
                      <input className="input" placeholder="Description" value={newRoleDesc} onChange={(e) => setNewRoleDesc(e.target.value)} style={{ flex: 1 }} />
                      <button className="btn btn-accent" onClick={createRole}>+</button>
                    </div>
                    {roles.length === 0 ? (
                      <div className="mono muted" style={{ fontSize: 12 }}>No custom roles. Create one to control per-user machine access.</div>
                    ) : (
                      <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                        {roles.map((r) => (
                          <div key={r.id} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 8px', borderRadius: 4, cursor: 'pointer', background: selectedRole?.id === r.id ? 'var(--bg-2)' : 'transparent' }} onClick={() => setSelectedRole(r)}>
                            <span className="mono" style={{ flex: 1, color: 'var(--fg)', fontSize: 13 }}>{r.name}</span>
                            <span className="mono muted" style={{ fontSize: 11 }}>{r.description}</span>
                            <button className="btn btn-sm" style={{ color: 'var(--err)', padding: '2px 6px' }} onClick={(e) => { e.stopPropagation(); deleteRole(r.id); }}>✕</button>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </div>

                <div className="panel">
                  <div className="panel-head">
                    <div className="panel-title">
                      <span className="ico">⊡</span> PERMISSIONS
                      {selectedRole && <span className="meta"> — {selectedRole.name}</span>}
                    </div>
                  </div>
                  <div className="panel-body" style={{ padding: 12 }}>
                    {!selectedRole ? (
                      <div className="mono muted" style={{ fontSize: 12 }}>Select a role to manage its permissions.</div>
                    ) : (
                      <>
                        <div style={{ display: 'flex', gap: 6, marginBottom: 10, flexWrap: 'wrap' }}>
                          <select className="input" value={newPerm.resource_type} onChange={(e) => setNewPerm({ ...newPerm, resource_type: e.target.value })} style={{ width: 110 }}>
                            <option value="agent">agent</option>
                            <option value="service">service</option>
                            <option value="container">container</option>
                            <option value="terminal">terminal</option>
                            <option value="config">config</option>
                            <option value="backup">backup</option>
                            <option value="k8s">k8s</option>
                          </select>
                          <input className="input" placeholder="Pattern (* or prefix*)" value={newPerm.resource_pattern} onChange={(e) => setNewPerm({ ...newPerm, resource_pattern: e.target.value })} style={{ width: 130 }} />
                          <select className="input" value={newPerm.action} onChange={(e) => setNewPerm({ ...newPerm, action: e.target.value })} style={{ width: 90 }}>
                            <option value="*">* (all)</option>
                            <option value="read">read</option>
                            <option value="write">write</option>
                            <option value="exec">exec</option>
                            <option value="delete">delete</option>
                          </select>
                          <button className="btn btn-accent" onClick={addPermission}>+</button>
                        </div>
                        {permissions.length === 0 ? (
                          <div className="mono muted" style={{ fontSize: 12 }}>No permissions yet.</div>
                        ) : (
                          <table className="tbl">
                            <thead><tr><th>TYPE</th><th>PATTERN</th><th>ACTION</th><th style={{ width: 30 }}></th></tr></thead>
                            <tbody>
                              {permissions.map((p) => (
                                <tr key={p.id}>
                                  <td className="mono">{p.resource_type}</td>
                                  <td className="mono">{p.resource_pattern}</td>
                                  <td className="mono">{p.action}</td>
                                  <td><button className="btn btn-sm" style={{ color: 'var(--err)', padding: '2px 6px' }} onClick={() => removePermission(p.id)}>✕</button></td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        )}
                        <div style={{ marginTop: 12, borderTop: '1px solid var(--bd)', paddingTop: 10 }}>
                          <div className="mono muted" style={{ fontSize: 11, marginBottom: 4 }}>ASSIGN TO USER</div>
                          <div style={{ display: 'flex', gap: 6 }}>
                            <input className="input" placeholder="username" value={assignLogin} onChange={(e) => setAssignLogin(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && assignRole()} style={{ flex: 1 }} />
                            <button className="btn btn-accent" onClick={assignRole}>assign</button>
                          </div>
                        </div>
                      </>
                    )}
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      </main>
    </div>
  );
}
