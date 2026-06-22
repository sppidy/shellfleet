'use client';

import { useState } from 'react';
import { apiFetch } from '@/lib/api';
import { Loader2Icon } from 'lucide-react';
import { SeriesChart, type Series, type Unit } from './SeriesChart';

const UNITS: Unit[] = ['percent', 'bytes', 'bytes_per_sec', 'cpu_seconds_per_sec', 'raw'];

export interface EditingPanel {
  id: string;            // db:<rowid>
  title: string;
  query: string;
  unit: string;
  source: string | null;
}

interface QueryResp {
  series: Series[];
  unit: string;
  expanded_query: string;
  upstream_status: string;
  upstream_error: string | null;
}

/**
 * Create/edit a custom metrics panel. `test` runs the query (unsaved) for a
 * chosen agent and previews the chart; `save` POSTs (create) or PUTs (edit).
 */
export default function MetricsPanelForm({
  sources, agents, editing, onSaved, onCancel,
}: {
  sources: string[];
  agents: string[];
  editing: EditingPanel | null;
  onSaved: () => void;
  onCancel: () => void;
}) {
  const [title, setTitle] = useState(editing?.title ?? '');
  const [query, setQuery] = useState(editing?.query ?? '');
  const [unit, setUnit] = useState<string>(editing?.unit && UNITS.includes(editing.unit as Unit) ? editing.unit : 'raw');
  const [source, setSource] = useState<string>(editing?.source ?? sources[0] ?? '');
  const [testAgent, setTestAgent] = useState<string>(agents[0] ?? '');
  const [testRange, setTestRange] = useState('1h');
  const [preview, setPreview] = useState<QueryResp | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canSave = title.trim() && query.trim() && source;

  const runTest = async () => {
    if (!query.trim() || !source) { setError('query and source are required to test'); return; }
    if (!testAgent) { setError('pick a test agent'); return; }
    setBusy(true); setError(null); setPreview(null);
    try {
      const res = await apiFetch('/api/ee/metrics/query/test', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ query, unit, source, agent_id: testAgent, range: testRange }),
      });
      if (!res.ok) { setError(await res.text().catch(() => `HTTP ${res.status}`) || `HTTP ${res.status}`); return; }
      setPreview(await res.json() as QueryResp);
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
    finally { setBusy(false); }
  };

  const save = async () => {
    if (!canSave) return;
    setBusy(true); setError(null);
    try {
      const body = JSON.stringify({ title: title.trim(), query, unit, source });
      const res = editing
        ? await apiFetch(`/api/ee/metrics/panels/${editing.id}`, { method: 'PUT', headers: { 'content-type': 'application/json' }, body })
        : await apiFetch('/api/ee/metrics/panels', { method: 'POST', headers: { 'content-type': 'application/json' }, body });
      if (!res.ok) { setError(await res.text().catch(() => `HTTP ${res.status}`) || `HTTP ${res.status}`); return; }
      onSaved();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
    finally { setBusy(false); }
  };

  return (
    <div className="panel" style={{ marginBottom: 12, borderColor: 'var(--accent-bd)' }}>
      <div className="panel-head">
        <div className="panel-title"><span className="ico">✎</span> {editing ? 'EDIT PANEL' : 'NEW PANEL'}</div>
        <button className="btn btn-sm" onClick={onCancel}>cancel</button>
      </div>
      <div className="panel-body" style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input className="input" placeholder="Panel title" value={title} onChange={(e) => setTitle(e.target.value)} style={{ flex: 1, minWidth: 180 }} />
          <select className="input" value={source} onChange={(e) => setSource(e.target.value)} style={{ width: 130 }}>
            {sources.length === 0 && <option value="">— no sources —</option>}
            {sources.map((s) => <option key={s} value={s}>{s}</option>)}
          </select>
          <select className="input" value={unit} onChange={(e) => setUnit(e.target.value)} style={{ width: 150 }}>
            {UNITS.map((u) => <option key={u} value={u}>{u}</option>)}
          </select>
        </div>
        <textarea className="input" placeholder="query template — use {hostname} / {instance} / {start} / {end} / {step}"
          value={query} onChange={(e) => setQuery(e.target.value)}
          style={{ fontFamily: 'var(--mono)', fontSize: 12, minHeight: 90, resize: 'vertical' }} />
        <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
          <span className="mono muted" style={{ fontSize: 11 }}>test against:</span>
          <select className="input" value={testAgent} onChange={(e) => setTestAgent(e.target.value)} style={{ width: 160 }}>
            <option value="">— agent —</option>
            {agents.map((a) => <option key={a} value={a}>{a.replace(/-id$/, '')}</option>)}
          </select>
          <select className="input" value={testRange} onChange={(e) => setTestRange(e.target.value)} style={{ width: 70 }}>
            <option value="1h">1h</option><option value="6h">6h</option><option value="24h">24h</option><option value="7d">7d</option>
          </select>
          <button className="btn" onClick={runTest} disabled={busy}>{busy ? <Loader2Icon className="w-4 h-4 animate-spin" /> : 'test'}</button>
          <button className="btn btn-accent" onClick={save} disabled={!canSave || busy}>{editing ? 'save' : 'create'}</button>
        </div>
        {error && <div className="mono" style={{ color: 'var(--err)', fontSize: 11 }}>{error}</div>}
        {preview && (
          preview.upstream_status !== 'success'
            ? <div className="mono" style={{ color: 'var(--warn)', fontSize: 11 }}>source error: {preview.upstream_error ?? 'unknown'}</div>
            : <SeriesChart series={preview.series} unit={(preview.unit as Unit) || (unit as Unit)} />
        )}
      </div>
    </div>
  );
}
