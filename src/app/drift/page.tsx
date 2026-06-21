'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface Snapshot { id: string; agent_id: string; captured_at: number; packages: string; services: string; containers: string; configs: string; triggered_by: string }
interface Alert { id: number; agent_id: string; category: string; change_type: string; item_name: string; old_value: string | null; new_value: string | null; acknowledged: number }
type Matrix = Record<string, Record<string, string>>; // agent_id -> { item: value }

const count = (s: string) => { try { const v = JSON.parse(s); return Array.isArray(v) ? v.length : Object.keys(v || {}).length; } catch { return 0; } };
const fmtTs = (t: number) => new Date(t * 1000).toLocaleString();
const short = (a: string) => a.replace(/-id$/, '');

export default function DriftPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [snapshots, setSnapshots] = useState<Snapshot[] | null>(null);
  const [alerts, setAlerts] = useState<Alert[]>([]);
  const [matrix, setMatrix] = useState<Matrix | null>(null);
  const [category, setCategory] = useState('packages');
  const [diffOnly, setDiffOnly] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const loadCompare = useCallback(async (cat: string) => {
    setMatrix(null);
    try {
      const res = await apiFetch(`/api/ee/drift/compare?agents=*&category=${encodeURIComponent(cat)}`);
      if (res.ok) setMatrix(await res.json()); else setMatrix({});
    } catch { setMatrix({}); }
  }, []);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [s, a] = await Promise.all([apiFetch('/api/ee/drift/snapshots'), apiFetch('/api/ee/drift/alerts')]);
      if (s.ok) setSnapshots(await s.json()); else { setError(`HTTP ${s.status}`); setSnapshots([]); }
      if (a.ok) setAlerts(await a.json());
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setSnapshots([]); }
    loadCompare(category);
  }, [category, loadCompare]);

  useEffect(() => { if (status === 'authed') load(); }, [status, load]);

  // Build the comparison rows: union of items across nodes, value per node.
  const { nodes, rows } = useMemo(() => {
    const m = matrix || {};
    const nodes = Object.keys(m).sort();
    const itemSet = new Set<string>();
    for (const n of nodes) for (const k of Object.keys(m[n])) itemSet.add(k);
    let items = [...itemSet].sort();
    const isDrift = (item: string) => {
      const vals = nodes.map((n) => m[n][item] ?? null);
      const present = vals.filter((v) => v !== null);
      // drift = some node missing it, or differing values among nodes that have it
      return present.length !== nodes.length || new Set(present).size > 1;
    };
    if (diffOnly) items = items.filter(isDrift);
    const rows = items.map((item) => ({ item, drift: isDrift(item), values: nodes.map((n) => m[n][item] ?? null) }));
    return { nodes, rows };
  }, [matrix, diffOnly]);

  const changeColor = (c: string) => (c === 'added' ? 'var(--accent)' : c === 'removed' ? 'var(--err)' : 'var(--warn)');

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/drift requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">drift detection</span>
          </div>
          <div className="topbar-actions"><button className="btn" onClick={load}>↻ refresh</button></div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="drift" label="Drift Detection">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}

              {/* CROSS-NODE COMPARISON — are all nodes in sync? */}
              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head">
                  <div className="panel-title"><span className="ico">⊞</span> CROSS-NODE COMPARISON</div>
                  <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                    <select className="input" value={category} onChange={(e) => { setCategory(e.target.value); loadCompare(e.target.value); }} style={{ width: 130 }}>
                      <option value="packages">packages</option><option value="services">services</option><option value="containers">containers</option><option value="configs">configs</option>
                    </select>
                    <label className="mono muted" style={{ fontSize: 11, display: 'flex', gap: 4, alignItems: 'center', cursor: 'pointer' }}>
                      <input type="checkbox" checked={diffOnly} onChange={(e) => setDiffOnly(e.target.checked)} /> differences only
                    </label>
                  </div>
                </div>
                <div className="panel-body flush" style={{ overflowX: 'auto' }}>
                  {matrix === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : nodes.length === 0 ? <div className="empty">No snapshots yet — drift compares each node&apos;s latest snapshot. Trigger snapshots to populate.</div>
                    : rows.length === 0 ? <div className="empty" style={{ color: 'var(--accent)' }}>✓ All {nodes.length} nodes are in sync for {category}.</div> : (
                    <table className="tbl">
                      <thead><tr><th>{category.toUpperCase()}</th>{nodes.map((n) => <th key={n}>{short(n)}</th>)}</tr></thead>
                      <tbody>{rows.map((r) => (
                        <tr key={r.item} style={r.drift ? { background: 'rgba(230,180,80,.06)' } : undefined}>
                          <td className="mono">{r.item}</td>
                          {r.values.map((v, i) => (
                            <td key={i} className="mono" style={{ color: v === null ? 'var(--err)' : r.drift ? 'var(--warn)' : 'var(--fg-2)', fontSize: 11 }}>
                              {v === null ? '— absent' : v}
                            </td>
                          ))}
                        </tr>
                      ))}</tbody>
                    </table>
                  )}
                </div>
              </div>

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">⚠</span> DRIFT ALERTS (per-node, over time)</div></div>
                <div className="panel-body flush">
                  {alerts.length === 0 ? <div className="empty">No drift alerts.</div> : (
                    <table className="tbl"><thead><tr><th>AGENT</th><th>CATEGORY</th><th>CHANGE</th><th>ITEM</th><th>OLD → NEW</th><th>ACK</th></tr></thead>
                      <tbody>{alerts.map((a) => (
                        <tr key={a.id}>
                          <td className="mono">{short(a.agent_id)}</td>
                          <td className="mono muted">{a.category}</td>
                          <td className="mono" style={{ color: changeColor(a.change_type) }}>{a.change_type}</td>
                          <td className="mono">{a.item_name}</td>
                          <td className="mono muted" style={{ fontSize: 11 }}>{a.old_value || '∅'} → {a.new_value || '∅'}</td>
                          <td className="mono">{a.acknowledged ? '✓' : '—'}</td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>

              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">◷</span> SNAPSHOTS</div></div>
                <div className="panel-body flush">
                  {snapshots === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : snapshots.length === 0 ? <div className="empty">No snapshots captured.</div> : (
                    <table className="tbl"><thead><tr><th>WHEN</th><th>AGENT</th><th>PKGS</th><th>SERVICES</th><th>CONTAINERS</th><th>CONFIGS</th><th>BY</th></tr></thead>
                      <tbody>{snapshots.map((s) => (
                        <tr key={s.id}>
                          <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(s.captured_at)}</td>
                          <td className="mono">{short(s.agent_id)}</td>
                          <td className="mono">{count(s.packages)}</td>
                          <td className="mono">{count(s.services)}</td>
                          <td className="mono">{count(s.containers)}</td>
                          <td className="mono">{count(s.configs)}</td>
                          <td className="mono muted">{s.triggered_by}</td>
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
