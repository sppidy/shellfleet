'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface ScimStatus { configured: boolean; users: number; base_url: string | null }

export default function ScimPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [data, setData] = useState<ScimStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/ee/scim/status');
      if (!res.ok) { setError(`HTTP ${res.status}`); return; }
      setData(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/scim requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  const baseUrl = data?.base_url || 'https://<your-ee-public-url>/scim/v2';

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">scim provisioning</span>
          </div>
          <div className="topbar-actions"><button className="btn" onClick={load}>↻ refresh</button></div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="scim" label="SCIM Provisioning">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {data === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div> : (<>
                <div className="cards" style={{ display: 'grid', gridTemplateColumns: 'repeat(2,1fr)', gap: 10, marginBottom: 14 }}>
                  <div className="panel"><div className="panel-body">
                    <div className="mono muted" style={{ fontSize: 11 }}>STATUS</div>
                    <div style={{ fontSize: 18, fontFamily: 'var(--mono)', color: data.configured ? 'var(--accent)' : 'var(--warn)' }}>{data.configured ? 'configured' : 'not configured'}</div>
                  </div></div>
                  <div className="panel"><div className="panel-body">
                    <div className="mono muted" style={{ fontSize: 11 }}>PROVISIONED USERS</div>
                    <div style={{ fontSize: 18, fontFamily: 'var(--mono)' }}>{data.users}</div>
                  </div></div>
                </div>

                <div className="panel" style={{ marginBottom: 12 }}>
                  <div className="panel-head"><div className="panel-title"><span className="ico">⇄</span> SCIM 2.0 ENDPOINT</div></div>
                  <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                    <div className="mono muted" style={{ fontSize: 12 }}>Point your identity provider (Okta, Entra ID, etc.) at this base URL:</div>
                    <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                      <code style={{ background: 'var(--bg)', border: '1px solid var(--line)', borderRadius: 4, padding: '6px 10px', fontSize: 12, color: 'var(--accent)', flex: 1, wordBreak: 'break-all' }}>{baseUrl}</code>
                      <button className="btn btn-sm" onClick={() => navigator.clipboard.writeText(baseUrl)}>copy</button>
                    </div>
                    {!data.base_url && <div className="mono" style={{ fontSize: 11, color: 'var(--warn)' }}>Set EE_PUBLIC_URL on the EE sidecar so this resolves to your real host.</div>}
                  </div>
                </div>

                <div className="panel">
                  <div className="panel-head"><div className="panel-title"><span className="ico">▸</span> SETUP</div></div>
                  <div className="panel-body">
                    <ol className="mono muted" style={{ fontSize: 12, paddingLeft: 18, lineHeight: 1.9 }}>
                      <li>Set <span style={{ color: 'var(--fg-2)' }}>EE_SCIM_TOKEN</span> on the EE sidecar to a long random secret.</li>
                      <li>In your IdP&apos;s SCIM app, use the base URL above and that token as the bearer.</li>
                      <li>The IdP pushes user create / update / deactivate; provisioned users appear in the count above and in Users &amp; seats.</li>
                    </ol>
                    <div className="mono muted" style={{ fontSize: 11, marginTop: 6 }}>SCIM is IdP-driven: user management happens in your IdP, not here. This page surfaces the integration status.</div>
                  </div>
                </div>
              </>)}
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
