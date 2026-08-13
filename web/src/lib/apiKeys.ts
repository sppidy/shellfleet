// Pure helpers for the API-keys UI — node-testable, no React (arm64 vitest OOM
// guard). `now` params default to wall-clock but are injectable for tests.

import { apiFetch } from './api';

export interface PolicySummary {
  id: number;
  name: string;
  description: string | null;
  resource_bound: boolean;
}

/** Fetch bindable IAM policies from EE. Empty array on failure. */
export async function fetchPolicies(): Promise<PolicySummary[]> {
  try {
    const res = await apiFetch('/api/ee/iam/policies');
    if (!res.ok) return [];
    const all = await res.json() as PolicySummary[];
    return all.filter((p) => !p.resource_bound);
  } catch { return []; }
}

export interface ApiKeyInfo {
  id: number;
  prefix: string;
  name: string;
  policy_id: number | null;
  expires_at: number | null;
  created_at: number;
  last_used_at: number | null;
}

export interface ApiKeyCreated {
  id: number;
  key: string;
  prefix: string;
  name: string;
  expires_at: number | null;
}

const nowSecs = () => Math.floor(Date.now() / 1000);

/** Relative "Ns/Nm/Nh/Nd ago"; 0/falsy → "never". */
export function formatRelative(unixSeconds: number, now: number = nowSecs()): string {
  if (!unixSeconds) return 'never';
  const delta = Math.max(0, now - unixSeconds);
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

/** Past (non-null) expiry. Null = never expires. */
export function isExpired(unixSeconds: number | null, now: number = nowSecs()): boolean {
  return unixSeconds != null && unixSeconds <= now;
}

/** "never" | "expired" | "YYYY-MM-DD". */
export function formatExpiry(unixSeconds: number | null, now: number = nowSecs()): string {
  if (unixSeconds == null) return 'never';
  if (unixSeconds <= now) return 'expired';
  return new Date(unixSeconds * 1000).toISOString().slice(0, 10);
}

/** "" → null; "YYYY-MM-DD" → unix seconds at 23:59:59 UTC that day; junk → null. */
export function toUnixExpiry(dateStr: string): number | null {
  if (!dateStr) return null;
  const ms = Date.parse(`${dateStr}T23:59:59Z`);
  if (Number.isNaN(ms)) return null;
  return Math.floor(ms / 1000);
}
