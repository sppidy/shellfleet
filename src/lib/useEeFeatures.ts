'use client';

import { useEffect, useState } from 'react';
import { apiFetch } from './api';
import type { LicenseStatus } from './eeFeatures';

/**
 * Fetches the EE license's advertised features once. `features` is null while
 * loading, then the advertised set (empty when EE is down or the license is
 * unhealthy). Backs the disabled+upsell gating on EE pages; the server enforces
 * the same entitlement (402), so this is UX, not the security boundary.
 */
export function useEeFeatures() {
  const [features, setFeatures] = useState<string[] | null>(null);
  const [status, setStatus] = useState<LicenseStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    apiFetch('/api/ee/license/status')
      .then(async (res) => {
        if (cancelled) return;
        if (res.ok) {
          const d: LicenseStatus = await res.json();
          setStatus(d);
          setFeatures(Array.isArray(d.features) ? d.features : []);
        } else {
          setFeatures([]);
        }
      })
      .catch(() => {
        if (!cancelled) setFeatures([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { features, status, loading: features === null };
}
