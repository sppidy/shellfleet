'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface OrgCost { org: string; seats: number; agents: number; seat_cost: number; agent_cost: number; total: number }
interface TagCost { tag: string; agents: number; agent_cost: number }
interface Breakdown {
  period: string;
  total_seats: number;
  used_seats: number;
  total_agents: number;
  license_price: number;
  currency: string;
  seat_cost_each: number;
  agent_cost_each: number;
  by_org: OrgCost[];
  by_tag: TagCost[];
  unattributed: { seats: number; agents: number; cost: number };
}

export default function CostPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [data, setData] = useState<Breakdown | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (status === 'guest') router.replace('/login');
  }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch('/api/ee/cost/current');
      if (!res.ok) { setError(`HTTP ${res.status}`); return; }
      setData(await res.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  }, []);

    useEffect(() => { if (status === 'authed' && role === 'admin') load(); }, [status, role, load]);

  const money = (n: number) => `${data?.currency ?? ''} ${n.toLocaleString(undefined, { maximumFractionDigits: 2 })}`;

  if (status !== 'authed') {
    return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  }
  if (role !== 'admin') {
    return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/cost requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;
  }

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">cost attribution</span>
          </div>
          <div className="topbar-actions">
            <button className="btn" onClick={load}>↻ refresh</button>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="cost" label="Cost Attribution">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {data === null ? (
                <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
              ) : (<>
                <div className="cards" style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 10, marginBottom: 14 }}>
                  <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 11 }}>PERIOD</div><div style={{ fontSize: 20, fontFamily: 'var(--mono)' }}>{data.period}</div></div></div>
                  <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 11 }}>SEATS</div><div style={{ fontSize: 20, fontFamily: 'var(--mono)' }}>{data.used_seats}/{data.total_seats}</div></div></div>
                  <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 11 }}>AGENTS</div><div style={{ fontSize: 20, fontFamily: 'var(--mono)' }}>{data.total_agents}</div></div></div>
                  <div className="panel"><div className="panel-body"><div className="mono muted" style={{ fontSize: 11 }}>LICENSE / MO</div><div style={{ fontSize: 20, fontFamily: 'var(--mono)', color: 'var(--accent)' }}>{money(data.license_price)}</div></div></div>
                </div>

                <div className="panel" style={{ marginBottom: 12 }}>
                  <div className="panel-head"><div className="panel-title"><span className="ico">◈</span> BY ORGANIZATION</div></div>
                  <div className="panel-body flush">
                    {data.by_org.length === 0 ? <div className="empty">No org attribution.</div> : (
                      <table className="tbl"><thead><tr><th>ORG</th><th>SEATS</th><th>AGENTS</th><th>SEAT COST</th><th>AGENT COST</th><th>TOTAL</th></tr></thead>
                        <tbody>{data.by_org.map((o) => (
                          <tr key={o.org}><td className="mono">{o.org}</td><td className="mono">{o.seats}</td><td className="mono">{o.agents}</td><td className="mono">{money(o.seat_cost)}</td><td className="mono">{money(o.agent_cost)}</td><td className="mono" style={{ color: 'var(--accent)' }}>{money(o.total)}</td></tr>
                        ))}</tbody></table>
                    )}
                  </div>
                </div>

                <div className="panel" style={{ marginBottom: 12 }}>
                  <div className="panel-head"><div className="panel-title"><span className="ico">⊙</span> BY TAG</div></div>
                  <div className="panel-body flush">
                    {data.by_tag.length === 0 ? <div className="empty">No tag attribution.</div> : (
                      <table className="tbl"><thead><tr><th>TAG</th><th>AGENTS</th><th>AGENT COST</th></tr></thead>
                        <tbody>{data.by_tag.map((t) => (
                          <tr key={t.tag}><td className="mono">{t.tag}</td><td className="mono">{t.agents}</td><td className="mono">{money(t.agent_cost)}</td></tr>
                        ))}</tbody></table>
                    )}
                  </div>
                </div>

                <div className="panel">
                  <div className="panel-head"><div className="panel-title"><span className="ico">∅</span> UNATTRIBUTED</div></div>
                  <div className="panel-body"><span className="mono muted">{data.unattributed.seats} seats · {data.unattributed.agents} agents · {money(data.unattributed.cost)}</span></div>
                </div>
              </>)}
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
