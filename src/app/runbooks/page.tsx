'use client';

import { Fragment, useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import { Loader2Icon } from 'lucide-react';

interface Runbook { id: number; name: string; description: string; steps: string; tags: string; created_by: string }
interface RunbookRun { id: number; runbook_id: number; agent_id: string; started_by: string; status: string; results: string; started_at: number; finished_at: number | null; error: string | null }
interface StepResult { step_index: number; name: string; status: string; exit_code?: number; stdout?: string; duration_ms?: number; error?: string }
interface EditStep { type: 'command' | 'gate'; name: string; command: string; prompt: string }

const parseResults = (r: string): StepResult[] => { try { return JSON.parse(r) as StepResult[]; } catch { return []; } };
const stepCount = (steps: string) => { try { return (JSON.parse(steps) as unknown[]).length; } catch { return 0; } };
const fmtTs = (t: number) => new Date(t * 1000).toLocaleString();
const blankStep = (): EditStep => ({ type: 'command', name: '', command: '', prompt: '' });

export default function RunbooksPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [books, setBooks] = useState<Runbook[] | null>(null);
  const [runs, setRuns] = useState<RunbookRun[]>([]);
  const [agents, setAgents] = useState<string[]>([]);
  const [agent, setAgent] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);

  // authoring form
  const [editing, setEditing] = useState<number | 'new' | null>(null);
  const [fName, setFName] = useState('');
  const [fDesc, setFDesc] = useState('');
  const [fSteps, setFSteps] = useState<EditStep[]>([blankStep()]);

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [b, r, t] = await Promise.all([
        apiFetch('/api/ee/runbooks'),
        apiFetch('/api/ee/runbooks/runs'),
        apiFetch('/api/tokens'),
      ]);
      if (b.ok) setBooks(await b.json()); else { setError(`HTTP ${b.status}`); setBooks([]); }
      if (r.ok) setRuns(await r.json());
      if (t.ok) { const toks: { hostname?: string }[] = await t.json(); setAgents(toks.filter((x) => x.hostname).map((x) => `${x.hostname}-id`)); }
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setBooks([]); }
  }, []);

  useEffect(() => { if (status === 'authed') load(); }, [status, load]);

  const openNew = () => { setEditing('new'); setFName(''); setFDesc(''); setFSteps([blankStep()]); setMsg(null); setError(null); };
  const openEdit = (rb: Runbook) => {
    setEditing(rb.id); setFName(rb.name); setFDesc(rb.description);
    try {
      const parsed = JSON.parse(rb.steps) as { name?: string; type?: string; command?: string; prompt?: string }[];
      setFSteps(parsed.map((s) => ({ type: s.type === 'gate' ? 'gate' : 'command', name: s.name || '', command: s.command || '', prompt: s.prompt || '' })));
    } catch { setFSteps([blankStep()]); }
    setMsg(null); setError(null);
  };
  const cancel = () => setEditing(null);

  const setStep = (i: number, patch: Partial<EditStep>) => setFSteps((prev) => prev.map((s, j) => (j === i ? { ...s, ...patch } : s)));
  const addStep = () => setFSteps((prev) => [...prev, blankStep()]);
  const rmStep = (i: number) => setFSteps((prev) => prev.filter((_, j) => j !== i));

  const save = async () => {
    setError(null);
    if (!fName.trim()) { setError('name is required'); return; }
    // Build steps payload in the wire shape the validator expects.
    const steps = fSteps.map((s) => s.type === 'command'
      ? { name: s.name.trim() || 'step', type: 'command', command: s.command }
      : { name: s.name.trim() || 'gate', type: 'gate', prompt: s.prompt });
    const body = { name: fName.trim(), description: fDesc.trim(), steps, tags: [] };
    try {
      const res = editing === 'new'
        ? await apiFetch('/api/ee/runbooks', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
        : await apiFetch(`/api/ee/runbooks/${editing}`, { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      setMsg(editing === 'new' ? `created "${fName.trim()}"` : `updated "${fName.trim()}"`);
      setEditing(null); await load();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const del = async (rb: Runbook) => {
    if (!confirm(`Delete runbook "${rb.name}"?`)) return;
    try {
      const res = await apiFetch(`/api/ee/runbooks/${rb.id}`, { method: 'DELETE' });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      await load();
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const run = async (rb: Runbook) => {
    if (!agent) { setError('select a target agent first'); return; }
    setMsg(null); setError(null);
    try {
      const res = await apiFetch(`/api/ee/runbooks/${rb.id}/run`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ agent_id: agent }) });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); return; }
      setMsg(`started "${rb.name}" on ${agent.replace(/-id$/, '')}`);
      setTimeout(load, 1500);
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
  };

  const statusColor = (s: string) => (s === 'completed' || s === 'ok' ? 'var(--accent)' : s === 'failed' ? 'var(--err)' : s === 'running' ? 'var(--warn)' : 'var(--fg-2)');

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/runbooks requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <span className="here">runbooks</span>
          </div>
          <div className="topbar-actions">
            <select className="input" value={agent} onChange={(e) => setAgent(e.target.value)} style={{ width: 180 }}>
              <option value="">— target agent —</option>
              {agents.map((a) => <option key={a} value={a}>{a.replace(/-id$/, '')}</option>)}
            </select>
            <button className="btn btn-accent" onClick={openNew}>+ new runbook</button>
            <button className="btn" onClick={load}>↻</button>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="runbooks" label="Runbooks">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              {msg && <div className="panel" style={{ borderColor: 'var(--accent-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--accent)' }}>{msg}</div></div>}

              {editing !== null && (
                <div className="panel" style={{ marginBottom: 12, borderColor: 'var(--accent-bd)' }}>
                  <div className="panel-head"><div className="panel-title"><span className="ico">✎</span> {editing === 'new' ? 'NEW RUNBOOK' : 'EDIT RUNBOOK'}</div></div>
                  <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 10, padding: 12 }}>
                    <div style={{ display: 'flex', gap: 8 }}>
                      <input className="input" placeholder="name" value={fName} onChange={(e) => setFName(e.target.value)} style={{ flex: 1 }} />
                    </div>
                    <input className="input" placeholder="description (optional)" value={fDesc} onChange={(e) => setFDesc(e.target.value)} />
                    <div className="mono muted" style={{ fontSize: 11 }}>STEPS (run top-to-bottom; command runs on the agent, gate pauses for confirmation)</div>
                    {fSteps.map((s, i) => (
                      <div key={i} style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                        <span className="mono muted" style={{ width: 18 }}>{i + 1}</span>
                        <select className="input" value={s.type} onChange={(e) => setStep(i, { type: e.target.value as 'command' | 'gate' })} style={{ width: 100 }}>
                          <option value="command">command</option>
                          <option value="gate">gate</option>
                        </select>
                        <input className="input" placeholder="step name" value={s.name} onChange={(e) => setStep(i, { name: e.target.value })} style={{ width: 140 }} />
                        {s.type === 'command'
                          ? <input className="input" placeholder="shell command" value={s.command} onChange={(e) => setStep(i, { command: e.target.value })} style={{ flex: 1, fontFamily: 'var(--mono)' }} />
                          : <input className="input" placeholder="confirmation prompt" value={s.prompt} onChange={(e) => setStep(i, { prompt: e.target.value })} style={{ flex: 1 }} />}
                        <button className="btn btn-sm" style={{ color: 'var(--err)' }} disabled={fSteps.length <= 1} onClick={() => rmStep(i)}>✕</button>
                      </div>
                    ))}
                    <div style={{ display: 'flex', gap: 8 }}>
                      <button className="btn btn-sm" onClick={addStep}>+ add step</button>
                      <div style={{ flex: 1 }} />
                      <button className="btn" onClick={cancel}>cancel</button>
                      <button className="btn btn-accent" onClick={save}>{editing === 'new' ? 'create' : 'save'}</button>
                    </div>
                  </div>
                </div>
              )}

              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">▦</span> RUNBOOKS</div></div>
                <div className="panel-body flush">
                  {books === null ? <div className="empty"><Loader2Icon className="w-5 h-5 animate-spin" /></div>
                    : books.length === 0 ? <div className="empty">No runbooks yet — click “+ new runbook”.</div> : (
                    <table className="tbl"><thead><tr><th>NAME</th><th>DESCRIPTION</th><th>STEPS</th><th style={{ width: 150 }}>ACTIONS</th></tr></thead>
                      <tbody>{books.map((rb) => (
                        <tr key={rb.id}>
                          <td className="mono">{rb.name}</td>
                          <td className="mono muted">{rb.description || '—'}</td>
                          <td className="mono">{stepCount(rb.steps)}</td>
                          <td style={{ whiteSpace: 'nowrap' }}>
                            <button className="btn btn-sm btn-accent" disabled={!agent} onClick={() => run(rb)} title={agent ? '' : 'pick a target agent'}>run</button>{' '}
                            <button className="btn btn-sm" onClick={() => openEdit(rb)}>edit</button>{' '}
                            <button className="btn btn-sm" style={{ color: 'var(--err)' }} onClick={() => del(rb)}>del</button>
                          </td>
                        </tr>
                      ))}</tbody></table>
                  )}
                </div>
              </div>

              <div className="panel">
                <div className="panel-head"><div className="panel-title"><span className="ico">↻</span> RUN HISTORY</div></div>
                <div className="panel-body flush">
                  {runs.length === 0 ? <div className="empty">No runs yet.</div> : (
                    <table className="tbl"><thead><tr><th>WHEN</th><th>AGENT</th><th>BY</th><th>STATUS</th><th style={{ width: 70 }}></th></tr></thead>
                      <tbody>{runs.map((r) => {
                        const res = parseResults(r.results);
                        return (
                          <Fragment key={r.id}>
                            <tr>
                              <td className="mono muted" style={{ fontSize: 11 }}>{fmtTs(r.started_at)}</td>
                              <td className="mono">{r.agent_id.replace(/-id$/, '')}</td>
                              <td className="mono">{r.started_by}</td>
                              <td className="mono" style={{ color: statusColor(r.status) }}>{r.status}</td>
                              <td><button className="btn btn-sm" onClick={() => setExpanded(expanded === r.id ? null : r.id)}>{expanded === r.id ? 'hide' : 'steps'}</button></td>
                            </tr>
                            {expanded === r.id && (
                              <tr><td colSpan={5} style={{ background: 'var(--bg)' }}>
                                {r.error && <div className="mono" style={{ color: 'var(--err)', fontSize: 12, marginBottom: 6 }}>{r.error}</div>}
                                {res.length === 0 ? <span className="mono muted" style={{ fontSize: 12 }}>no step results</span> : res.map((s) => (
                                  <div key={s.step_index} style={{ marginBottom: 6 }}>
                                    <span className="mono" style={{ color: statusColor(s.status), fontSize: 12 }}>[{s.status}]</span>{' '}
                                    <span className="mono" style={{ fontSize: 12 }}>{s.name}</span>
                                    {s.exit_code != null && <span className="mono muted" style={{ fontSize: 11 }}> (exit {s.exit_code})</span>}
                                    {(s.stdout || s.error) && <pre style={{ margin: '3px 0 0', fontSize: 11, color: 'var(--fg-2)', whiteSpace: 'pre-wrap', wordBreak: 'break-all', maxHeight: 160, overflow: 'auto' }}>{s.error || s.stdout}</pre>}
                                  </div>
                                ))}
                              </td></tr>
                            )}
                          </Fragment>
                        );
                      })}</tbody></table>
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
