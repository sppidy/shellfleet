'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface Entry { id: number; cidr: string; name: string | null; enabled: number; created_at: number }

export default function IpAllowlistPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [cidr, setCidr] = useState('');
  const [name, setName] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/ee/ip-allowlist/');
      if (!res.ok) { setError(`HTTP ${res.status}`); setEntries([]); return; }
      setEntries(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setEntries([]); }
  }, []);

  useEffect(() => { if (status === 'authed') load(); }, [status, load]);

  const add = async () => {
    if (!cidr.trim()) return;
    setError(null);
    try {
      const res = await apiFetch('/api/ee/ip-allowlist/', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ cidr: cidr.trim(), name: name.trim() || null }),
      });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      setCidr(''); setName(''); await load();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const remove = async (id: number) => {
    if (!confirm('Remove this allow-list entry?')) return;
    try {
      await apiFetch(`/api/ee/ip-allowlist/${id}`, { method: 'DELETE' });
      await load();
    } catch { /* ignore */ }
  };

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/ip-allowlist requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">ip allow-list</span>
          </div>
          <div className="topbar-actions"><button className="btn" onClick={load}>↻ refresh</button></div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="ip-allowlist" label="IP Allow-list">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">＋</span> ADD CIDR</div></div>
                <div className="panel-body" style={{ display: 'flex', gap: 8, alignItems: 'end', padding: 12 }}>
                  <div className="field" style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                    <label className="mono muted" style={{ fontSize: 11 }}>CIDR</label>
                    <input className="input" placeholder="203.0.113.0/24" value={cidr} onChange={(e) => setCidr(e.target.value)} style={{ width: 200 }} />
                  </div>
                  <div className="field" style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                    <label className="mono muted" style={{ fontSize: 11 }}>NAME (optional)</label>
                    <input className="input" placeholder="office vpn" value={name} onChange={(e) => setName(e.target.value)} style={{ width: 200 }} />
                  </div>
                  <button className="btn btn-accent" onClick={add} disabled={!cidr.trim()}>add</button>
                </div>
                <div className="panel-body" style={{ paddingTop: 0 }}>
                  <span className="mono muted" style={{ fontSize: 11 }}>When the allow-list is non-empty, EE rejects API access from IPs outside it.</span>
                </div>
              </div>
              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">≣</span> ALLOWED RANGES</div></div>
                <div className="panel-body flush">
                  {entries === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : entries.length === 0 ? <div className="empty">Empty — all IPs allowed.</div> : (
                    <table className="tbl"><thead><tr><th>CIDR</th><th>NAME</th><th>STATUS</th><th style={{ width: 90 }}>ACTIONS</th></tr></thead>
                      <tbody>{entries.map((e) => (
                        <tr key={e.id}>
                          <td className="mono">{e.cidr}</td>
                          <td className="mono muted">{e.name || '—'}</td>
                          <td className="mono" style={{ color: e.enabled ? 'var(--accent)' : 'var(--fg-2)' }}>{e.enabled ? 'enabled' : 'disabled'}</td>
                          <td><button className="btn btn-sm" style={{ color: 'var(--err)', padding: '2px 8px' }} onClick={() => remove(e.id)}>remove</button></td>
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
