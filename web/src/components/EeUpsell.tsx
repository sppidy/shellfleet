'use client';

/**
 * Shown in place of an EE feature surface when EE is reachable but the license
 * doesn't include that feature. Matches the server-side gate (402) — the
 * feature genuinely can't be used, this just explains why and points to the fix.
 */
export default function EeUpsell({ feature, label }: { feature: string; label: string }) {
  return (
    <div className="pane">
      <div className="panel" style={{ borderColor: 'var(--warn-bd)' }}>
        <div className="panel-head">
          <div className="panel-title">
            <span className="ico">⊘</span> {label}
            <span className="meta" style={{ color: 'var(--warn)' }}>not in your license</span>
          </div>
        </div>
        <div className="panel-body" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div style={{ color: 'var(--fg-1)' }}>
            <strong>{label}</strong> isn&apos;t included in your ShellFleet EE license.
          </div>
          <div className="mono muted" style={{ fontSize: 12 }}>
            licensed feature: <span style={{ color: 'var(--fg-2)' }}>{feature}</span>
          </div>
          <div className="muted" style={{ fontSize: 12 }}>
            Add it to your plan to enable this feature, then re-issue and apply the license key.
          </div>
        </div>
      </div>
    </div>
  );
}
