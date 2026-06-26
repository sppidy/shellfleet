'use client';

import { useState } from 'react';
import { apiFetch } from '@/lib/api';
import { toUnixExpiry, type ApiKeyCreated } from '@/lib/apiKeys';
import { Loader2Icon } from 'lucide-react';

/**
 * Create a new API key. Controlled name + optional expiry date. On success,
 * hands the one-time secret back to the parent via `onCreated`.
 */
export default function ApiKeyCreateForm({
  onCreated, onCancel,
}: {
  onCreated: (created: ApiKeyCreated) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState('');
  const [expiry, setExpiry] = useState(''); // 'YYYY-MM-DD' or ''
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const today = new Date().toISOString().slice(0, 10);
  const canSave = name.trim().length > 0 && name.trim().length <= 100
    && (expiry === '' || expiry > today);

  const save = async () => {
    if (!canSave) return;
    setBusy(true); setError(null);
    try {
      const body = JSON.stringify({ name: name.trim(), expires_at: toUnixExpiry(expiry) });
      const res = await apiFetch('/api/ee/keys', {
        method: 'POST', headers: { 'content-type': 'application/json' }, body,
      });
      if (!res.ok) { setError(await res.text().catch(() => `HTTP ${res.status}`) || `HTTP ${res.status}`); return; }
      const created = await res.json() as ApiKeyCreated;
      setName(''); setExpiry('');
      onCreated(created);
    } catch (e) { setError(e instanceof Error ? e.message : 'failed'); }
    finally { setBusy(false); }
  };

  return (
    <div className="panel" style={{ marginBottom: 12, borderColor: 'var(--accent-bd)' }}>
      <div className="panel-head">
        <div className="panel-title"><span className="ico">✎</span> NEW API KEY</div>
        <button className="btn btn-sm" onClick={onCancel}>cancel</button>
      </div>
      <div className="panel-body" style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input className="input" placeholder="Key name (e.g. ci-deploy)" value={name}
            maxLength={100} onChange={(e) => setName(e.target.value)} style={{ flex: 1, minWidth: 200 }} />
          <label className="mono muted" style={{ fontSize: 11, display: 'flex', alignItems: 'center', gap: 6 }}>
            expires
            <input className="input" type="date" min={today} value={expiry}
              onChange={(e) => setExpiry(e.target.value)} style={{ width: 150 }} />
          </label>
        </div>
        {error && <div className="mono" style={{ color: 'var(--err)', fontSize: 11 }}>{error}</div>}
        <div style={{ display: 'flex', gap: 8 }}>
          <button className="btn btn-accent" disabled={!canSave || busy} onClick={save}>
            {busy ? <Loader2Icon className="w-4 h-4 animate-spin" /> : 'create key'}
          </button>
          <span className="mono muted" style={{ fontSize: 11, alignSelf: 'center' }}>
            leave expiry empty for a non-expiring key
          </span>
        </div>
      </div>
    </div>
  );
}
