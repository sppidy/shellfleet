'use client';

import { useSession } from '@/components/providers/SessionProvider';

/**
 * Slim banner pinned to the top of the main shell when the active
 * session is in the viewer role. Non-dismissable on purpose — the
 * backend will 403 destructive ops, so a sticky reminder is the
 * least-confusing UX.
 */
export default function ViewerBanner() {
  const { role, status } = useSession();
  if (status !== 'authed' || role !== 'viewer') return null;

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        background: 'var(--warn-bg, #2a2310)',
        color: 'var(--warn, #d8b65a)',
        borderBottom: '1px solid var(--warn-bd, #4a3a18)',
        padding: '6px 14px',
        fontFamily: 'var(--mono)',
        fontSize: 12,
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        zIndex: 5,
      }}
    >
      <span aria-hidden>○</span>
      <span>
        viewer access — destructive actions are disabled. ask an admin to
        promote you at <code>/admin</code> if you need write access.
      </span>
    </div>
  );
}
