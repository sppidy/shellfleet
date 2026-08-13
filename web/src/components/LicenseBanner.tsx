'use client';

import { useEffect, useState } from 'react';
import { apiFetch } from '@/lib/api';

export default function LicenseBanner() {
  const [daysRemaining, setDaysRemaining] = useState<number | null>(null);

  useEffect(() => {
    apiFetch('/api/ee/license/status')
      .then(async (res) => {
        if (!res.ok) return;
        const data = await res.json();
        if (typeof data.days_remaining === 'number' && data.days_remaining >= 0) {
          setDaysRemaining(data.days_remaining);
        }
      })
      .catch(() => {});
  }, []);

  if (daysRemaining === null || daysRemaining > 30) return null;

  const urgent = daysRemaining <= 7;

  return (
    <div
      style={{
        padding: '8px 16px',
        background: urgent ? 'var(--err-bg)' : 'var(--warn-bg, rgba(210,153,34,0.08))',
        borderBottom: `1px solid ${urgent ? 'var(--err-bd)' : 'var(--warn-bd, rgba(210,153,34,0.3))'}`,
        color: urgent ? 'var(--err)' : 'var(--warn, #d29922)',
        fontFamily: 'var(--mono)',
        fontSize: 12,
        textAlign: 'center',
      }}
    >
      {daysRemaining === 0
        ? 'Your ShellFleet EE license expires today. Contact support to renew.'
        : `Your ShellFleet EE license expires in ${daysRemaining} day${daysRemaining === 1 ? '' : 's'}. Renew to avoid feature degradation.`}
    </div>
  );
}
