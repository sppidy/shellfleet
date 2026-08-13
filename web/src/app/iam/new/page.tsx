'use client';

import { useRouter } from 'next/navigation';
import { useState, useEffect, useCallback } from 'react';
import { apiFetch } from '@/lib/api';
import { useSession } from '@/components/providers/SessionProvider';
import EeFeatureGate from '@/components/EeFeatureGate';
import {
  parseActionsTextarea,
  normalizeActions,
  validateActionPattern,
  expandWildcardMinus,
  type ActionsResponse,
  type ActionCategory,
} from '@/lib/iamPolicy';
import { Loader2Icon } from 'lucide-react';

interface StatementForm {
  key: number;
  effect: string;
  actions: string[];
  textareaValue: string;
  expandedCategory: string | null;
  allActions: string[];
  parseResult: { kind: 'valid' | 'invalid' | 'malformed'; message?: string } | null;
}

let nextKey = 1;

export default function IamNewPage() {
  const router = useRouter();
  const { role, status } = useSession();
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [statements, setStatements] = useState<StatementForm[]>([]);
  const [allActions, setAllActions] = useState<string[]>([]);
  const [categories, setCategories] = useState<ActionCategory[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  useEffect(() => { if (status === 'guest') router.replace('/login'); }, [status, router]);

  const loadActions = useCallback(async () => {
    try {
      const res = await apiFetch('/api/ee/iam/actions');
      if (!res.ok) return;
      const data: ActionsResponse = await res.json();
      setCategories(data.categories);
      const flat = data.categories.flatMap((c) => c.actions);
      setAllActions(flat);
      // Start with one empty statement
      const key = nextKey++;
      setStatements([{
        key, effect: 'Allow', actions: [],
        textareaValue: '[]',
        expandedCategory: null,
        allActions: flat,
        parseResult: { kind: 'valid' },
      }]);
    } finally { setLoading(false); }
  }, []);

  useEffect(() => { if (status === 'authed' && role === 'admin') loadActions(); }, [status, role, loadActions]);

  const updateStatement = (key: number, fn: (s: StatementForm) => StatementForm) => {
    setStatements((prev) => prev.map((s) => s.key === key ? fn(s) : s));
  };

  const toggleGlobalAll = (stmt: StatementForm) => {
    const isAll = stmt.actions.length === 1 && stmt.actions[0] === '*';
    if (isAll) {
      // deselect
      updateStatement(stmt.key, (s) => ({ ...s, actions: [], textareaValue: '[]', parseResult: { kind: 'valid' } }));
    } else {
      updateStatement(stmt.key, (s) => ({ ...s, actions: ['*'], textareaValue: '["*"]', parseResult: { kind: 'valid' } }));
    }
  };

  const toggleCategory = (stmt: StatementForm, prefix: string) => {
    const catActions = allActions.filter((a) => a.startsWith(prefix + ':'));
    const allSelected = catActions.every((a) => stmt.actions.includes(a));
    let next: string[];
    if (allSelected) {
      next = stmt.actions.filter((a) => !a.startsWith(prefix + ':'));
    } else {
      const withoutCategory = stmt.actions.filter((a) => !a.startsWith(prefix + ':'));
      next = [...withoutCategory, ...catActions];
    }
    const normalized = normalizeActions(next, allActions);
    updateStatement(stmt.key, (s) => ({
      ...s,
      actions: normalized,
      textareaValue: JSON.stringify(normalized),
      parseResult: { kind: 'valid' },
    }));
  };

  const toggleAction = (stmt: StatementForm, action: string) => {
    const has = stmt.actions.includes(action);
    let next: string[];
    if (has) {
      // if "*" is selected, expand to all-minus-one
      if (stmt.actions.length === 1 && stmt.actions[0] === '*') {
        next = expandWildcardMinus(action, allActions);
      } else {
        next = stmt.actions.filter((a) => a !== action);
      }
    } else {
      next = [...stmt.actions, action];
    }
    const normalized = normalizeActions(next, allActions);
    updateStatement(stmt.key, (s) => ({
      ...s,
      actions: normalized,
      textareaValue: JSON.stringify(normalized),
      parseResult: { kind: 'valid' },
    }));
  };

  const onTextareaChange = (stmt: StatementForm, text: string) => {
    const result = parseActionsTextarea(text, allActions);
    if (result.kind === 'valid') {
      updateStatement(stmt.key, (s) => ({
        ...s,
        textareaValue: text,
        actions: result.actions,
        parseResult: { kind: 'valid' },
      }));
    } else if (result.kind === 'invalid') {
      updateStatement(stmt.key, (s) => ({
        ...s,
        textareaValue: text,
        parseResult: { kind: 'invalid', message: result.errors.join(', ') },
      }));
    } else {
      updateStatement(stmt.key, (s) => ({
        ...s,
        textareaValue: text,
        parseResult: { kind: 'malformed', message: result.error },
      }));
    }
  };

  const addStatement = () => {
    const key = nextKey++;
    setStatements((prev) => [...prev, {
      key, effect: 'Allow', actions: [],
      textareaValue: '[]',
      expandedCategory: null,
      allActions,
      parseResult: { kind: 'valid' },
    }]);
  };

  const removeStatement = (key: number) => {
    setStatements((prev) => prev.filter((s) => s.key !== key));
  };

  const validate = (): boolean => {
    const errs: Record<string, string> = {};
    const trimmedName = name.trim();
    if (!trimmedName) errs.name = 'Name is required';
    else if (trimmedName.length > 100) errs.name = 'Name must be <= 100 characters';
    if (description.length > 500) errs.description = 'Description must be <= 500 characters';
    if (statements.length === 0) errs.statements = 'At least one statement is required';
    for (const s of statements) {
      if (s.actions.length === 0) {
        errs[`stmt-${s.key}`] = 'Each statement needs at least one action';
      }
    }
    setFieldErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const save = async () => {
    if (!validate()) return;
    setSaving(true);
    setError(null);
    try {
      const res = await apiFetch('/api/ee/iam/policies', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          name: name.trim(),
          description: description.trim() || null,
          statements: statements.map((s) => ({
            effect: s.effect,
            actions: s.actions,
          })),
        }),
      });
      if (!res.ok) { setError(await res.text() || `HTTP ${res.status}`); setSaving(false); return; }
      const created = await res.json();
      router.push(`/iam/${created.id}`);
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); setSaving(false); }
  };

  if (status !== 'authed') return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;
  if (role !== 'admin') return <div className="center-screen" style={{ flexDirection: 'column', gap: 12 }}><div className="mono" style={{ color: 'var(--err)' }}>/iam requires the admin role.</div><button className="btn" onClick={() => router.push('/')}>← back</button></div>;

  if (loading) return <div className="center-screen"><Loader2Icon className="w-6 h-6 animate-spin" style={{ color: 'var(--fg-2)' }} /></div>;

  const isAllSelected = (stmt: StatementForm) => stmt.actions.length === 1 && stmt.actions[0] === '*';
  const isCategorySelected = (stmt: StatementForm, prefix: string) => {
    const catActions = allActions.filter((a) => a.startsWith(prefix + ':'));
    return catActions.length > 0 && catActions.every((a) => stmt.actions.includes(a));
  };

  return (
    <div className="app-shell" style={{ gridTemplateColumns: '1fr' }}>
      <main className="main">
        <div className="topbar">
          <div className="breadcrumb">
            <span className="prompt">$</span>
            <button type="button" className="nav-item" onClick={() => router.push('/')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>←&nbsp;back</button>
            <span className="sep">/</span>
            <button type="button" className="nav-item" onClick={() => router.push('/iam')} style={{ height: 'auto', padding: '0 4px', display: 'inline-flex' }}>iam</button>
            <span className="sep">/</span>
            <span className="here">new</span>
          </div>
        </div>
        <div className="scroll">
          <EeFeatureGate feature="acl" label="IAM Policies">
            <div className="pane">
              {error && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{error}</div></div>}
              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head"><div className="panel-title"><span className="ico">⊡</span> CREATE POLICY</div></div>
                <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div className="field">
                    <label className="mono muted" style={{ fontSize: 11 }}>Name *</label>
                    <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. DeployBots" style={{ width: 300 }} />
                    {fieldErrors.name && <div className="mono" style={{ color: 'var(--err)', fontSize: 11, marginTop: 3 }}>{fieldErrors.name}</div>}
                  </div>
                  <div className="field">
                    <label className="mono muted" style={{ fontSize: 11 }}>Description</label>
                    <input className="input" value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Optional" style={{ width: 400 }} />
                    {fieldErrors.description && <div className="mono" style={{ color: 'var(--err)', fontSize: 11, marginTop: 3 }}>{fieldErrors.description}</div>}
                  </div>
                </div>
              </div>

              {fieldErrors.statements && <div className="panel" style={{ borderColor: 'var(--err-bd)', marginBottom: 12 }}><div className="panel-body" style={{ color: 'var(--err)' }}>{fieldErrors.statements}</div></div>}

              {statements.map((stmt, idx) => (
                <div key={stmt.key} className="panel" style={{ marginBottom: 12 }}>
                  <div className="panel-head">
                    <div className="panel-title">Statement {idx + 1}</div>
                    <div className="panel-actions">
                      <select className="input" value={stmt.effect} onChange={(e) => updateStatement(stmt.key, (s) => ({ ...s, effect: e.target.value }))} style={{ width: 100, marginRight: 8 }}>
                        <option value="Allow">Allow</option>
                        <option value="Deny">Deny</option>
                      </select>
                      {statements.length > 1 && (
                        <button className="btn btn-sm" style={{ color: 'var(--err)' }} onClick={() => removeStatement(stmt.key)}>remove</button>
                      )}
                    </div>
                  </div>
                  <div className="panel-body">
                    {/* Action picker */}
                    <div style={{ marginBottom: 8 }}>
                      <label className="mono muted" style={{ fontSize: 11, display: 'block', marginBottom: 4 }}>Actions</label>
                      {fieldErrors[`stmt-${stmt.key}`] && <div className="mono" style={{ color: 'var(--err)', fontSize: 11, marginBottom: 4 }}>{fieldErrors[`stmt-${stmt.key}`]}</div>}

                      {/* Global "All actions" toggle */}
                      <label style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6, cursor: 'pointer', fontWeight: isAllSelected(stmt) ? 700 : 400 }} onClick={() => toggleGlobalAll(stmt)}>
                        <input type="checkbox" checked={isAllSelected(stmt)} readOnly style={{ accentColor: 'var(--accent)' }} />
                        <span className="mono">All actions (*)</span>
                      </label>

                      {/* Category grid */}
                      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 4, marginBottom: 8 }}>
                        {categories.map((cat) => {
                          const selected = isCategorySelected(stmt, cat.name);
                          return (
                            <div key={cat.name} style={{ border: '1px solid var(--bd)', borderRadius: 4, padding: 4 }}>
                              <label style={{ display: 'flex', alignItems: 'center', gap: 4, cursor: 'pointer', fontWeight: selected ? 700 : 400, fontSize: 12 }} onClick={() => toggleCategory(stmt, cat.name)}>
                                <input type="checkbox" checked={selected || isAllSelected(stmt)} readOnly style={{ accentColor: 'var(--accent)', opacity: isAllSelected(stmt) ? 0.4 : 1 }} />
                                <span className="mono">{cat.name}:*</span>
                              </label>
                              {stmt.expandedCategory === cat.name && (
                                <div style={{ marginLeft: 12, marginTop: 2 }}>
                                  {cat.actions.map((a) => (
                                    <label key={a} style={{ display: 'flex', alignItems: 'center', gap: 4, cursor: 'pointer', fontSize: 11 }} onClick={() => toggleAction(stmt, a)}>
                                      <input type="checkbox" checked={stmt.actions.includes(a)} readOnly style={{ accentColor: 'var(--accent)' }} />
                                      <span className="mono muted">{a}</span>
                                    </label>
                                  ))}
                                </div>
                              )}
                              <button
                                type="button"
                                className="mono muted"
                                style={{ fontSize: 9, border: 'none', background: 'none', cursor: 'pointer', marginTop: 2, marginLeft: 18 }}
                                onClick={() => updateStatement(stmt.key, (s) => ({ ...s, expandedCategory: s.expandedCategory === cat.name ? null : cat.name }))}
                              >
                                {stmt.expandedCategory === cat.name ? '▲ collapse' : '▼ expand'}
                              </button>
                            </div>
                          );
                        })}
                      </div>

                      {/* Editable textarea */}
                      <textarea
                        className="input mono"
                        value={stmt.textareaValue}
                        onChange={(e) => onTextareaChange(stmt, e.target.value)}
                        rows={3}
                        style={{ width: '100%', fontSize: 11 }}
                      />
                      {stmt.parseResult?.message && (
                        <div className="mono" style={{ color: 'var(--err)', fontSize: 11, marginTop: 3 }}>{stmt.parseResult.message}</div>
                      )}
                    </div>

                    {/* Resources — disabled, wildcard-only */}
                    <div>
                      <label className="mono muted" style={{ fontSize: 11, display: 'block', marginBottom: 2 }}>Resources</label>
                      <input className="input mono" value='["*"]' disabled style={{ width: 200, opacity: 0.6, fontSize: 11 }} />
                      <div className="mono muted" style={{ fontSize: 10, marginTop: 2 }}>Resource scoping is not yet enforced — all policies apply to all resources in v1.</div>
                    </div>
                  </div>
                </div>
              ))}

              <div style={{ display: 'flex', gap: 8, marginBottom: 12 }}>
                <button className="btn" onClick={addStatement}>+ add statement</button>
              </div>

              <div style={{ display: 'flex', gap: 8 }}>
                <button className="btn btn-accent" onClick={save} disabled={saving}>{saving ? 'saving…' : 'save policy'}</button>
                <button className="btn" onClick={() => router.push('/iam')}>cancel</button>
              </div>
            </div>
          </EeFeatureGate>
        </div>
      </main>
    </div>
  );
}
