'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface Recording {
  id: string; session_id: string; agent_id: string; login: string;
  session_type: string; started_at: number; ended_at: number | null;
  duration_secs: number | null; size_bytes: number | null; status: string;
}

const fmtDur = (s: number | null) => (s == null ? '—' : s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`);
const fmtBytes = (b: number | null) => (b == null ? '—' : b < 1024 ? `${b} B` : b < 1048576 ? `${(b / 1024).toFixed(1)} KiB` : `${(b / 1048576).toFixed(1)} MiB`);
const fmtTs = (t: number) => new Date(t * 1000).toLocaleString();

// Parse an asciicast v2 blob into the concatenated output stream. Line 1 is the
// header; the rest are [time, stream, data] events — we join the "o" output.
function castToText(raw: string): string {
  const lines = raw.split('\n').filter(Boolean);
  let out = '';
  for (let i = 1; i < lines.length; i++) {
    try {
      const ev = JSON.parse(lines[i]);
      if (Array.isArray(ev) && ev[1] === 'o') out += ev[2];
    } catch { /* skip */ }
  }
  return out;
}

export default function RecordingsPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [list, setList] = useState<Recording[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewing, setViewing] = useState<{ rec: Recording; text: string } | null>(null);
  const [loadingCast, setLoadingCast] = useState(false);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/ee/recordings');
      if (!res.ok) { setError(`HTTP ${res.status}`); setList([]); return; }
      setList(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setList([]); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  const view = async (rec: Recording) => {
    setLoadingCast(true); setViewing(null);
    try {
      const res = await apiFetch(`/api/ee/recordings/${rec.id}/play`);
      if (!res.ok) { setError(`play HTTP ${res.status}`); return; }
      setViewing({ rec, text: castToText(await res.text()) });
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
    finally { setLoadingCast(false); }
  };

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/recordings requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">session recordings</span>
          </div>
          <div className="topbar-actions"><button className="btn" onClick={load}>↻ refresh</button></div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="recordings" label="Session Recordings">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">◉</span> RECORDINGS</div></div>
                <div className="panel-body flush">
                  {list === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : list.length === 0 ? <div className="empty">No recordings yet.</div> : (
                    <table className="tbl"><thead><tr><th>WHEN</th><th>AGENT</th><th>USER</th><th>TYPE</th><th>DURATION</th><th>SIZE</th><th style={{ width: 70 }}></th></tr></thead>
                      <tbody>{list.map((r) => (
                        <tr key={r.id}>
                          <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(r.started_at)}</td>
                          <td className="mono">{r.agent_id.replace(/-id$/, '')}</td>
                          <td className="mono">{r.login}</td>
                          <td className="mono muted">{r.session_type}</td>
                          <td className="mono">{fmtDur(r.duration_secs)}</td>
                          <td className="mono muted">{fmtBytes(r.size_bytes)}</td>
                          <td><button className="btn btn-sm" onClick={() => view(r)}>view</button></td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>
              {loadingCast && <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /> loading session…</div>}
              {viewing && (
                <div className="panel">
                  <div className="panel-head"><div className="panel-title"><span className="ico">▶</span> {viewing.rec.agent_id.replace(/-id$/, '')} · {viewing.rec.login} · {fmtTs(viewing.rec.started_at)}</div></div>
                  <div className="panel-body">
                    <pre style={{ background: 'var(--bg)', border: '1px solid var(--line)', borderRadius: 4, padding: 12, maxHeight: 480, overflow: 'auto', fontFamily: 'var(--mono)', fontSize: 12, whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>{viewing.text || '(no terminal output captured)'}</pre>
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
