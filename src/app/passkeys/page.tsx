'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface Credential { id: string; name: string; aaguid: string | null; created_at: number; last_used_at: number | null }

const fmtTs = (t: number) => new Date(t * 1000).toLocaleString();

// base64url <-> ArrayBuffer (webauthn-rs uses URL-safe base64 without padding).
function b64urlToBuf(s: string): ArrayBuffer {
  const pad = s.length % 4 === 0 ? '' : '='.repeat(4 - (s.length % 4));
  const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + pad);
  const u = new Uint8Array(b.length);
  for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i);
  return u.buffer;
}
function bufToB64url(buf: ArrayBuffer): string {
  const u = new Uint8Array(buf);
  let s = '';
  for (let i = 0; i < u.length; i++) s += String.fromCharCode(u[i]);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export default function PasskeysPage() {
  const router = useRouter();
  const { status } = useSession();
  const [creds, setCreds] = useState<Credential[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/ee/webauthn/credentials');
      if (!res.ok) { setError(`HTTP ${res.status}`); setCreds([]); return; }
      setCreds(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setCreds([]); }
  }, []);

  useEffect(() => { if (status === 'authed') load(); }, [status, load]);

  const register = async () => {
    setBusy(true); setError(null); setMsg(null);
    try {
      const begin = await apiFetch('/api/ee/webauthn/register/begin', { method: 'POST' });
      if (!begin.ok) { setError(`begin: HTTP ${begin.status}`); return; }
      const { state_id, options } = await begin.json();
      const pk = options.publicKey;
      // Convert the server's base64url buffers to ArrayBuffers for the WebAuthn API.
      pk.challenge = b64urlToBuf(pk.challenge);
      pk.user.id = b64urlToBuf(pk.user.id);
      if (Array.isArray(pk.excludeCredentials)) {
        pk.excludeCredentials = pk.excludeCredentials.map((c: { id: string }) => ({ ...c, id: b64urlToBuf(c.id) }));
      }
      const cred = (await navigator.credentials.create({ publicKey: pk })) as PublicKeyCredential | null;
      if (!cred) { setError('passkey creation cancelled'); return; }
      const att = cred.response as AuthenticatorAttestationResponse;
      const payload = {
        id: cred.id,
        rawId: bufToB64url(cred.rawId),
        type: cred.type,
        response: {
          attestationObject: bufToB64url(att.attestationObject),
          clientDataJSON: bufToB64url(att.clientDataJSON),
        },
        extensions: cred.getClientExtensionResults(),
      };
      const finish = await apiFetch('/api/ee/webauthn/register/finish', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ state_id, credential: payload, name: `passkey ${new Date().toLocaleDateString()}` }),
      });
      if (!finish.ok) { setError(`finish: ${await finish.text() || finish.status}`); return; }
      setMsg('passkey registered'); await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'passkey registration failed');
    } finally { setBusy(false); }
  };

  const remove = async (id: string) => {
    if (!confirm('Remove this passkey?')) return;
    try {
      await apiFetch(`/api/ee/webauthn/credentials/${encodeURIComponent(id)}`, { method: 'DELETE' });
      await load();
    } catch { /* ignore */ }
  };

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">passkeys</span>
          </div>
          <div className="topbar-actions">
            <button className="btn btn-accent" onClick={register} disabled={busy}>{busy ? 'waiting…' : '+ register passkey'}</button>
            <button className="btn" onClick={load}>↻</button>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="webauthn" label="Passkeys (WebAuthn)">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {msg && <div className="panel" style={{ borderColor: 'var(--accent-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--accent)' }}>{msg}</div></div>}
              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">⚷</span> YOUR PASSKEYS</div></div>
                <div className="panel-body flush">
                  {creds === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : creds.length === 0 ? <div className="empty">No passkeys yet — click “register passkey”.</div> : (
                    <table className="tbl"><thead><tr><th>NAME</th><th>ADDED</th><th>LAST USED</th><th style={{ width: 80 }}></th></tr></thead>
                      <tbody>{creds.map((c) => (
                        <tr key={c.id}>
                          <td className="mono">{c.name}</td>
                          <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(c.created_at)}</td>
                          <td className="mono muted" style={{ fontSize: 11 }}>{c.last_used_at ? fmtTs(c.last_used_at) : 'never'}</td>
                          <td><button className="btn btn-sm" style={{ color: 'var(--err)' }} onClick={() => remove(c.id)}>remove</button></td>
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
