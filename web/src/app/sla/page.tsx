'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface UptimeSummary { agent_id: string; total_up: number; total_down: number; uptime_pct: number; days: number }
interface Breach { id: number; agent_id: string; window_start: string; window_end: string; actual_pct: number; target_pct: number; acknowledged: number }

const dur = (s: number) => {
  if (s <= 0) return '0';
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  return d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h ${m}m` : `${m}m`;
};

export default function SlaPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [summary, setSummary] = useState<UptimeSummary[] | null>(null);
  const [breaches, setBreaches] = useState<Breach[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [s, b] = await Promise.all([
        apiFetch('/api/ee/sla/uptime/summary'),
        apiFetch('/api/ee/sla/breaches'),
      ]);
      if (s.ok) setSummary(await s.json()); else { setError(`HTTP ${s.status}`); setSummary([]); }
      if (b.ok) setBreaches(await b.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setSummary([]); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  const pctColor = (p: number) => (p >= 99.9 ? 'var(--accent)' : p >= 99 ? 'var(--fg)' : p >= 95 ? 'var(--warn)' : 'var(--err)');

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/sla requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">sla &amp; uptime</span>
          </div>
          <div className="topbar-actions"><button className="btn" onClick={load}>↻ refresh</button></div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="sla" label="SLA & Uptime">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">▲</span> UPTIME (per agent)</div></div>
                <div className="panel-body flush">
                  {summary === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : summary.length === 0 ? <div className="empty">No uptime data yet.</div> : (
                    <table className="tbl"><thead><tr><th>AGENT</th><th>UPTIME</th><th>UP</th><th>DOWN</th><th>WINDOW</th></tr></thead>
                      <tbody>{summary.map((u) => (
                        <tr key={u.agent_id}>
                          <td className="mono">{u.agent_id.replace(/-id$/, '')}</td>
                          <td className="mono" style={{ color: pctColor(u.uptime_pct) }}>{u.uptime_pct.toFixed(2)}%</td>
                          <td className="mono muted">{dur(u.total_up)}</td>
                          <td className="mono" style={{ color: u.total_down > 0 ? 'var(--warn)' : 'var(--fg-2)' }}>{dur(u.total_down)}</td>
                          <td className="mono muted">{u.days}d</td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>
              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">⚠</span> SLA BREACHES</div></div>
                <div className="panel-body flush">
                  {breaches.length === 0 ? <div className="empty">No breaches recorded.</div> : (
                    <table className="tbl"><thead><tr><th>AGENT</th><th>WINDOW</th><th>ACTUAL</th><th>TARGET</th><th>ACK</th></tr></thead>
                      <tbody>{breaches.map((b) => (
                        <tr key={b.id}>
                          <td className="mono">{b.agent_id.replace(/-id$/, '')}</td>
                          <td className="mono muted" style={{ fontSize: 11 }}>{b.window_start} → {b.window_end}</td>
                          <td className="mono" style={{ color: 'var(--err)' }}>{b.actual_pct.toFixed(2)}%</td>
                          <td className="mono">{b.target_pct.toFixed(2)}%</td>
                          <td className="mono">{b.acknowledged ? '✓' : '—'}</td>
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
