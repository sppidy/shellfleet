'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

interface Role {
  id: number;
  name: string;
  description: string | null;
  created_at: number;
}

interface Permission {
  id: number;
  role_id: number;
  resource_type: string;
  resource_pattern: string;
  action: string;
}

interface RoleAssignment {
  login: string;
  role_id: number;
}

export default function RolesPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [roles, setRoles] = useState<Role[]>([]);
  const [selectedRole, setSelectedRole] = useState<Role | null>(null);
  const [permissions, setPermissions] = useState<Permission[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [newRoleName, setNewRoleName] = useState('');
  const [newRoleDesc, setNewRoleDesc] = useState('');
  const [newPerm, setNewPerm] = useState({ resource_type: 'agent', resource_pattern: '*', action: '*' });
  const [assignLogin, setAssignLogin] = useState('');

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
    if (status === 'pending_mfa') router.replace('/mfa');
  }, [status, router]);

  const fetchRoles = useCallback(async () => {
    try {
      const res = await apiFetch('/api/ee/rbac/roles');
      if (res.status === 404 || res.status === 502) {
        setError('EE sidecar not available — RBAC requires Enterprise Edition');
        return;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setRoles(await res.json());
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  }, []);

  const fetchPermissions = useCallback(async (roleId: number) => {
    try {
      const res = await apiFetch(`/api/ee/rbac/roles/${roleId}/permissions`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setPermissions(await res.json());
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  }, []);

  useEffect(() => {
    if (status === 'authed') fetchRoles();
  }, [status, fetchRoles]);

  useEffect(() => {
    if (selectedRole) fetchPermissions(selectedRole.id);
    else setPermissions([]);
  }, [selectedRole, fetchPermissions]);

  const createRole = async () => {
    if (!newRoleName.trim()) return;
    setError(null);
    try {
      const res = await apiFetch('/api/ee/rbac/roles', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ name: newRoleName.trim(), description: newRoleDesc.trim() || null }),
      });
      if (!res.ok) throw new Error(await res.text());
      setNewRoleName('');
      setNewRoleDesc('');
      await fetchRoles();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  };

  const deleteRole = async (id: number) => {
    setError(null);
    try {
      const res = await apiFetch(`/api/ee/rbac/roles/${id}`, { method: 'DELETE' });
      if (!res.ok) throw new Error(await res.text());
      if (selectedRole?.id === id) setSelectedRole(null);
      await fetchRoles();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  };

  const addPermission = async () => {
    if (!selectedRole) return;
    setError(null);
    try {
      const res = await apiFetch(`/api/ee/rbac/roles/${selectedRole.id}/permissions`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(newPerm),
      });
      if (!res.ok) throw new Error(await res.text());
      await fetchPermissions(selectedRole.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  };

  const removePermission = async (permId: number) => {
    setError(null);
    try {
      const res = await apiFetch(`/api/ee/rbac/permissions/${permId}`, { method: 'DELETE' });
      if (!res.ok) throw new Error(await res.text());
      if (selectedRole) await fetchPermissions(selectedRole.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  };

  const assignRole = async () => {
    if (!selectedRole || !assignLogin.trim()) return;
    setError(null);
    try {
      const res = await apiFetch('/api/ee/rbac/assignments', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ login: assignLogin.trim(), role_id: selectedRole.id }),
      });
      if (!res.ok) throw new Error(await res.text());
      setAssignLogin('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed');
    }
  };

  if (status !== 'authed') {
    return (
      <div className="center-screen">
        <Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} />
      </div>
    );
  }

  if (role !== 'admin') {
    return (
      <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}>
        <div className="mono" style={{ color: 'var(--err)' }}>
          /roles requires the admin role.
        </div>
        <button className="btn" onClick={() => router.push('/')}>
          ← back to dashboard
        </button>
      </div>
    );
  }

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
            <span className="here">roles (EE)</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={fetchRoles} title="Refresh">
              ↻ refresh
            </button>
          </div>
        </div>

        <div className="scroll">
          <div className="pane">
            {error && (
              <div className="panel" style={{ borderColor: 'var(--err-bd)' }}>
                <div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div>
              </div>
            )}

            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
              {/* Left: Roles list */}
              <div className="panel">
                <div className="panel-head">
                  <div className="panel-title">
                    <span className="ico">⚙</span> ROLES
                  </div>
                </div>
                <div className="panel-body" style={{ padding: 12 }}>
                  <div style={{ display: 'flex', gap: 6, marginBottom: 12 }}>
                    <input
                      className="input"
                      placeholder="Role name"
                      value={newRoleName}
                      onChange={(e) => setNewRoleName(e.target.value)}
                      onKeyDown={(e) => e.key === 'Enter' && createRole()}
                      style={{ flex: 1 }}
                    />
                    <input
                      className="input"
                      placeholder="Description (optional)"
                      value={newRoleDesc}
                      onChange={(e) => setNewRoleDesc(e.target.value)}
                      style={{ flex: 1 }}
                    />
                    <button className="btn btn-accent" onClick={createRole}>+ add</button>
                  </div>
                  {roles.length === 0 ? (
                    <div className="mono muted" style={{ fontSize: 12 }}>No custom roles yet.</div>
                  ) : (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                      {roles.map((r) => (
                        <div
                          key={r.id}
                          style={{
                            display: 'flex', alignItems: 'center', gap: 8,
                            padding: '6px 8px', borderRadius: 4, cursor: 'pointer',
                            background: selectedRole?.id === r.id ? 'var(--bg-2)' : 'transparent',
                          }}
                          onClick={() => setSelectedRole(r)}
                        >
                          <span className="mono" style={{ flex: 1, color: 'var(--fg)' }}>{r.name}</span>
                          <span className="mono muted" style={{ fontSize: 11 }}>{r.description}</span>
                          <button
                            className="btn btn-sm"
                            style={{ color: 'var(--err)' }}
                            onClick={(e) => { e.stopPropagation(); deleteRole(r.id); }}
                          >
                            ✕
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>

              {/* Right: Permissions for selected role */}
              <div className="panel">
                <div className="panel-head">
                  <div className="panel-title">
                    <span className="ico">⊡</span> PERMISSIONS
                    {selectedRole && (
                      <span className="meta"> — {selectedRole.name}</span>
                    )}
                  </div>
                </div>
                <div className="panel-body" style={{ padding: 12 }}>
                  {!selectedRole ? (
                    <div className="mono muted" style={{ fontSize: 12 }}>Select a role to manage permissions.</div>
                  ) : (
                    <>
                      <div style={{ display: 'flex', gap: 6, marginBottom: 12, flexWrap: 'wrap' }}>
                        <select
                          className="input"
                          value={newPerm.resource_type}
                          onChange={(e) => setNewPerm({ ...newPerm, resource_type: e.target.value })}
                          style={{ width: 120 }}
                        >
                          <option value="agent">agent</option>
                          <option value="service">service</option>
                          <option value="container">container</option>
                          <option value="terminal">terminal</option>
                          <option value="config">config</option>
                          <option value="backup">backup</option>
                          <option value="k8s">k8s</option>
                        </select>
                        <input
                          className="input"
                          placeholder="Pattern (* or prefix*)"
                          value={newPerm.resource_pattern}
                          onChange={(e) => setNewPerm({ ...newPerm, resource_pattern: e.target.value })}
                          style={{ width: 140 }}
                        />
                        <select
                          className="input"
                          value={newPerm.action}
                          onChange={(e) => setNewPerm({ ...newPerm, action: e.target.value })}
                          style={{ width: 100 }}
                        >
                          <option value="*">* (all)</option>
                          <option value="read">read</option>
                          <option value="write">write</option>
                          <option value="exec">exec</option>
                          <option value="delete">delete</option>
                        </select>
                        <button className="btn btn-accent" onClick={addPermission}>+ add</button>
                      </div>
                      {permissions.length === 0 ? (
                        <div className="mono muted" style={{ fontSize: 12 }}>No permissions. Add one above.</div>
                      ) : (
                        <table className="tbl">
                          <thead>
                            <tr>
                              <th>RESOURCE</th>
                              <th>PATTERN</th>
                              <th>ACTION</th>
                              <th style={{ width: 40 }}></th>
                            </tr>
                          </thead>
                          <tbody>
                            {permissions.map((p) => (
                              <tr key={p.id}>
                                <td className="mono">{p.resource_type}</td>
                                <td className="mono">{p.resource_pattern}</td>
                                <td className="mono">{p.action}</td>
                                <td>
                                  <button
                                    className="btn btn-sm"
                                    style={{ color: 'var(--err)' }}
                                    onClick={() => removePermission(p.id)}
                                  >
                                    ✕
                                  </button>
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      )}

                      <div style={{ marginTop: 16, borderTop: '1px solid var(--bd)', paddingTop: 12 }}>
                        <div className="mono muted" style={{ fontSize: 11, marginBottom: 6 }}>ASSIGN TO USER</div>
                        <div style={{ display: 'flex', gap: 6 }}>
                          <input
                            className="input"
                            placeholder="Login (e.g. sppidy)"
                            value={assignLogin}
                            onChange={(e) => setAssignLogin(e.target.value)}
                            onKeyDown={(e) => e.key === 'Enter' && assignRole()}
                            style={{ flex: 1 }}
                          />
                          <button className="btn btn-accent" onClick={assignRole}>assign</button>
                        </div>
                      </div>
                    </>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
