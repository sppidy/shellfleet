'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import { Loader2Icon } from 'lucide-react';

const DEFAULT_DOC = `{
  "groups": {
    "ops-team": ["alice", "bob"],
    "dev-team": ["charlie", "dave"]
  },
  "acls": [
    {
      "src": ["ops-team"],
      "dst": ["*"],
      "allow": ["*"]
    },
    {
      "src": ["dev-team"],
      "dst": ["staging-*"],
      "allow": ["agent:View", "agent:Terminal", "container:*"]
    },
    {
      "src": ["dev-team"],
      "dst": ["prod-*"],
      "allow": ["agent:View", "container:Logs"],
      "deny": ["agent:Terminal", "container:Exec"]
    }
  ]
}`;

export default function PolicyPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [doc, setDoc] = useState('');
  const [saved, setSaved] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [actions, setActions] = useState<string[]>([]);
  const [testLogin, setTestLogin] = useState('');
  const [testAction, setTestAction] = useState('');
  const [testResource, setTestResource] = useState('');
  const [testResult, setTestResult] = useState<{ allowed: boolean; reason: string } | null>(null);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  const loadDoc = useCallback(async () => {
    try {
      const res = await apiFetch('/api/ee/acl/document');
      if (res.status === 404 || res.status === 502) {
        setDoc(DEFAULT_DOC);
        return;
      }
      if (!res.ok) return;
      const data = await res.json();
      setDoc(JSON.stringify(data, null, 2));
      setSaved(true);
    } catch { setDoc(DEFAULT_DOC); }
  }, []);

  const loadActions = useCallback(async () => {
    try {
      const res = await apiFetch('/api/ee/acl/actions');
      if (res.ok) setActions(await res.json());
    } catch { /* ignore */ }
  }, []);

  useEffect(() => {
    if (status === 'authed') { loadDoc(); loadActions(); }
  }, [status, loadDoc, loadActions]);

  const saveDoc = async () => {
    setError(null); setSuccess(null);
    try {
      const parsed = JSON.parse(doc);
      const res = await apiFetch('/api/ee/acl/document', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ document: parsed }),
      });
      if (!res.ok) {
        const text = await res.text();
        throw new Error(text);
      }
      setSaved(true);
      setSuccess('Policy saved.');
      setTimeout(() => setSuccess(null), 3000);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'invalid JSON');
    }
  };

  const testEval = async () => {
    if (!testLogin || !testAction || !testResource) return;
    try {
      const res = await apiFetch('/api/ee/acl/evaluate', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ login: testLogin, action: testAction, resource: testResource }),
      });
      if (res.ok) setTestResult(await res.json());
    } catch { /* ignore */ }
  };

  if (status !== 'authed') {
    return (<div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>);
  }
  if (role !== 'admin') {
    return (<div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>admin required</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>);
  }

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>��&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">access policy</span>
          </div>
          <div className="topbar-actions">
            <button className="btn btn-accent" onClick={saveDoc} disabled={saved}>
              {saved ? '✓ saved' : '● save policy'}
            </button>
          </div>
        </div>

        <div className="scroll">
          <div className="pane">
            {error && (
              <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}>
                <div className="panel-body" style={{ color: 'var(--err)', fontSize: 12, fontFamily: 'var(--mono)' }}>{error}</div>
              </div>
            )}
            {success && (
              <div className="panel" style={{ borderColor: 'var(--accent-bd)', marginBottom: 12 }}>
                <div className="panel-body" style={{ color: 'var(--accent)', fontSize: 12, fontFamily: 'var(--mono)' }}>{success}</div>
              </div>
            )}

            <div style={{ display: 'grid', gridTemplateColumns: '1fr 280px', gap: 12 }}>
              {/* Editor */}
              <div className="panel">
                <div className="panel-head">
                  <div className="panel-title"><span className="ico">⚡</span> ACL POLICY</div>
                </div>
                <div className="panel-body" style={{ padding: 0 }}>
                  <textarea
                    value={doc}
                    onChange={(e) => { setDoc(e.target.value); setSaved(false); }}
                    spellCheck={false}
                    style={{
                      width: '100%', minHeight: 500, resize: 'vertical',
                      fontFamily: 'var(--mono)', fontSize: 12, lineHeight: 1.5,
                      background: 'var(--bg)', color: 'var(--fg)',
                      border: 'none', padding: 16, outline: 'none',
                    }}
                  />
                </div>
              </div>

              {/* Sidebar: actions reference + test */}
              <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                <div className="panel">
                  <div className="panel-head">
                    <div className="panel-title"><span className="ico">?</span> TEST</div>
                  </div>
                  <div className="panel-body" style={{ padding: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
                    <input className="input" placeholder="login (e.g. charlie)" value={testLogin} onChange={(e) => setTestLogin(e.target.value)} />
                    <input className="input" placeholder="action (e.g. agent:Terminal)" value={testAction} onChange={(e) => setTestAction(e.target.value)} />
                    <input className="input" placeholder="resource (e.g. prod-web-id)" value={testResource} onChange={(e) => setTestResource(e.target.value)} />
                    <button className="btn" onClick={testEval}>evaluate</button>
                    {testResult && (
                      <div className="mono" style={{ fontSize: 11, color: testResult.allowed ? 'var(--accent)' : 'var(--err)' }}>
                        {testResult.allowed ? '✓ ALLOW' : '✕ DENY'} — {testResult.reason}
                      </div>
                    )}
                  </div>
                </div>

                <div className="panel">
                  <div className="panel-head">
                    <div className="panel-title"><span className="ico">≡</span> ACTIONS</div>
                  </div>
                  <div className="panel-body" style={{ padding: 10, maxHeight: 350, overflowY: 'auto' }}>
                    {actions.map((a) => (
                      <div key={a} className="mono" style={{ fontSize: 11, color: 'var(--fg-2)', lineHeight: 1.8 }}>{a}</div>
                    ))}
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
