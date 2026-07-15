'use client';

import { createContext, useCallback, useContext, useEffect, useState } from 'react';

export type SessionStatus = 'loading' | 'authed' | 'guest' | 'pending_mfa';
export type Role = 'admin' | 'viewer';

interface SessionContextValue {
  user: string | null;
  role: Role | null;
  /** True iff the user has TOTP enrolled — the dashboard shows a
   *  "secure your account" nudge when this is false. */
  mfaEnabled: boolean;
  status: SessionStatus;
  /** Force a re-fetch of /api/me. Called after enroll / disable / verify
   *  so the rest of the UI reacts immediately. */
  refresh: () => void;
  logout: () => void;
}

const SessionContext = createContext<SessionContextValue | null>(null);

interface MeResponse {
  user: string;
  role: Role;
  mfa_enabled: boolean;
  mfa_verified: boolean;
}

export function SessionProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<string | null>(null);
  const [role, setRole] = useState<Role | null>(null);
  const [mfaEnabled, setMfaEnabled] = useState<boolean>(false);
  const [status, setStatus] = useState<SessionStatus>('loading');

  const refresh = useCallback(async () => {
    // /api/me is the session probe. Most failures are short-lived
    // (Cloudflare challenge, 5xx blip, network flake), so we retry
    // a few times with backoff before giving up. A persistent 403
    // — typically an ad-blocker matching `*/api/me` against a
    // privacy filter list — eventually flips to 'guest' so the
    // dashboard doesn't sit on an infinite spinner.
    const ATTEMPT_DELAYS_MS = [0, 400, 1200, 3000];
    for (let i = 0; i < ATTEMPT_DELAYS_MS.length; i++) {
      if (ATTEMPT_DELAYS_MS[i] > 0) {
        await new Promise((r) => setTimeout(r, ATTEMPT_DELAYS_MS[i]));
      }
      try {
        const res = await fetch('/api/me', { credentials: 'same-origin' });
        if (res.ok) {
          const data = (await res.json()) as MeResponse;
          setUser(data.user);
          setRole(data.role);
          setMfaEnabled(data.mfa_enabled);
          setStatus(data.mfa_verified ? 'authed' : 'pending_mfa');
          return;
        }
        if (res.status === 401) {
          setUser(null);
          setRole(null);
          setMfaEnabled(false);
          setStatus('guest');
          return;
        }
        // 403 / 429 / 5xx — fall through to retry.
        if (i === ATTEMPT_DELAYS_MS.length - 1) {
          console.error(
            `[shellfleet] /api/me persistently returning ${res.status}. ` +
              `If this is your dashboard, check your browser's ad-blocker / ` +
              `privacy extension — many block paths matching /api/me/.`,
          );
          setUser(null);
          setRole(null);
          setMfaEnabled(false);
          setStatus('guest');
          return;
        }
      } catch (err) {
        if (i === ATTEMPT_DELAYS_MS.length - 1) {
          console.error('[shellfleet] /api/me unreachable:', err);
          setUser(null);
          setRole(null);
          setMfaEnabled(false);
          setStatus('guest');
          return;
        }
      }
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const logout = () => {
    window.location.href = '/auth/logout';
  };

  return (
    <SessionContext.Provider value={{ user, role, mfaEnabled, status, refresh, logout }}>
      {children}
    </SessionContext.Provider>
  );
}

export function useSession() {
  const ctx = useContext(SessionContext);
  if (!ctx) throw new Error('useSession must be used within SessionProvider');
  return ctx;
}

/** Convenience for components that need to disable destructive UI for
 *  viewers. Returns `true` when the current role is admin AND the
 *  session is fully verified. */
export function useCanWrite(): boolean {
  const { role, status } = useSession();
  return status === 'authed' && role === 'admin';
}
