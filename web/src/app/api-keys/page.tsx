'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import { useEeFeatures } from '@/lib/useEeFeatures';
import { hasFeature } from '@/lib/eeFeatures';
import EeFeatureGate from '@/components/EeFeatureGate';
import ApiKeyCreateForm from '@/components/ApiKeyCreateForm';
import { formatRelative, formatExpiry, fetchPolicies, type PolicySummary, type ApiKeyInfo, type ApiKeyCreated } from '@/lib/apiKeys';
import { Loader2Icon } from 'lucide-react';

export default function ApiKeysPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const { features } = useEeFeatures();
  const canWrite = hasFeature(features, 'api-keys');

  const [keys, setKeys] = useState<ApiKeyInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [reveal, setReveal] = useState<ApiKeyCreated | null>(null);
  const [viewLogin, setViewLogin] = useState(''); // admin user-switcher
  const [scope, setScope] = useState('');         // active scope label
  const [policyMap, setPolicyMap] = useState<Map<number, string>>(new Map()); // policy_id → name

  useEffect(() => { fetchPolicies().then((ps) => setPolicyMap(new Map(ps.map((p) => [p.id, p.name])))); }, []);
  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async (login?: string) => {
    setError(null);
    try {
      const qs = login ? `?login=${encodeURIComponent(login)}` : '';
      const res = await apiFetch(`/api/ee/keys${qs}`);
      if (res.status === 401) { window.location.href = '/login'; return; }
      if (!res.ok) throw new Error((await res.text().catch(() => `HTTP ${res.status}`)) || `HTTP ${res.status}`);
      setKeys(await res.json() as ApiKeyInfo[]);
      setScope(login ?? '');
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setKeys([]); }
  }, []);

  useEffect(() => { if (status === 'authed') load(); }, [status, load]);

  const revoke = async (k: ApiKeyInfo) => {
    if (!confirm(`Revoke key "${k.name}" (sf_live_…${k.prefix})? This cannot be undone.`)) return;
    try { await apiFetch(`/api/ee/keys/${k.id}`, { method: 'DELETE' }); await load(scope || undefined); } catch { /* ignore */ }
  };

  const onCreated = (created: ApiKeyCreated) => { setShowForm(false); setReveal(created); load(scope || undefined); };

  if (status !== 'authed') {
    return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  }

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">api keys</span>
          </div>
          <div className="topbar-actions">
            {role === 'admin' && (
              <>
                <input className="input" placeholder="view user login…" value={viewLogin}
                  onChange={(e) => setViewLogin(e.target.value)} style={{ width: 160 }} />
                <button className="btn" onClick={() => load(viewLogin.trim() || undefined)}>view</button>
              </>
            )}
            {canWrite && <button className="btn btn-accent" onClick={() => setShowForm((s) => !s)}>+ key</button>}
            <button className="btn" onClick={() => load(scope || undefined)} title="Reload">↻</button>
          </div>
        </div>
        <div className="scroll">
          <div className="pane">
            <div className="mono muted" style={{ fontSize: 11, marginBottom: 10 }}>
              Keys authenticate <span style={{ color: 'var(--fg-2)' }}>/api/v1</span> with your account role.
              Keys bound to a policy are limited to that policy&rsquo;s allowed actions.
              {scope && <> · viewing <span style={{ color: 'var(--fg-2)' }}>{scope}</span></>}
            </div>

            {reveal && (
              <div className="panel" style={{ borderColor: 'var(--accent-bd)', marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">⚿</span> COPY YOUR KEY NOW</div>
                  <button className="btn btn-sm" onClick={() => setReveal(null)}>done</button></div>
                <div className="panel-body" style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
                  <div className="mono" style={{ color: 'var(--warn)', fontSize: 11 }}>
                    This secret is shown once. Store it now — you won&rsquo;t be able to see it again.
                  </div>
                  <div style={{ display: 'flex', gap: 8 }}>
                    <code className="code" style={{ flex: 1, padding: 8, wordBreak: 'break-all' }}>{reveal.key}</code>
                    <button className="btn" onClick={() => navigator.clipboard?.writeText(reveal.key)}>copy</button>
                  </div>
                </div>
              </div>
            )}

            {showForm && canWrite && (
              <EeFeatureGate feature="api-keys" label="API Keys">
                <ApiKeyCreateForm onCreated={onCreated} onCancel={() => setShowForm(false)} />
              </EeFeatureGate>
            )}

            {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}

            {keys === null ? (
              <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
            ) : keys.length === 0 ? (
              <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 12 }}>
                No API keys{scope ? ` for ${scope}` : ''}.{canWrite ? ' Use "+ key" to create one.' : ''}
              </div></div></div>
            ) : (
              <div className="panel">
                <div className="panel-body flush">
                  <table className="tbl">
                    <thead>
                      <tr>
                        <th>NAME</th><th>PREFIX</th><th>POLICY</th><th>CREATED</th><th>LAST USED</th><th>EXPIRES</th><th />
                      </tr>
                    </thead>
                    <tbody>
                      {keys.map((k) => (
                        <tr key={k.id}>
                          <td className="mono" style={{ color: 'var(--fg)' }}>{k.name}</td>
                          <td className="mono muted">sf_live_&hellip;{k.prefix}</td>
                          <td className="mono muted">{k.policy_id ? (policyMap.get(k.policy_id) ?? `policy #${k.policy_id}`) : '—'}</td>
                          <td className="mono">{formatRelative(k.created_at)}</td>
                          <td className="mono muted">{k.last_used_at ? formatRelative(k.last_used_at) : 'never'}</td>
                          <td className="mono muted">{formatExpiry(k.expires_at)}</td>
                          <td className="actions">
                            <button className="btn sm danger" onClick={() => revoke(k)}>revoke</button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}
          </div>
        </div>
      </main>
    </div>
  );
}
