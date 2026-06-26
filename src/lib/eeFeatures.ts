// Maps a dashboard surface to the EE license feature that entitles it. A
// surface is shown only when EE is reachable; if EE is up but the feature
// isn't in the license's advertised set, the surface renders disabled with an
// upsell (see EeUpsell). The server enforces the same entitlement (402), so
// this is UX, not the security boundary.

/** Tab id -> the license feature it requires, plus a display label. */
export const EE_TAB_FEATURE: Record<string, { feature: string; label: string }> = {
  ai: { feature: 'ai-analysis', label: 'AI Analysis' },
  'api-keys': { feature: 'api-keys', label: 'API Keys' },
};

/** True when the license advertises `name`. `features` null = unknown/EE-down. */
export function hasFeature(features: string[] | null | undefined, name: string): boolean {
  return Array.isArray(features) && features.includes(name);
}

/** Shape of GET /api/ee/license/status. */
export interface LicenseStatus {
  customer: string;
  seats: number;
  features: string[];
  healthy: boolean;
  expires_at: number;
  days_remaining: number;
}
