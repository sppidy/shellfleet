'use client';

import { createContext, useContext, useEffect, useState } from 'react';

type SessionStatus = 'loading' | 'authed' | 'guest';

interface SessionContextValue {
  user: string | null;
  status: SessionStatus;
  logout: () => void;
}

const SessionContext = createContext<SessionContextValue | null>(null);

export function SessionProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<string | null>(null);
  const [status, setStatus] = useState<SessionStatus>('loading');

  useEffect(() => {
    let cancelled = false;
    fetch('/api/me', { credentials: 'same-origin' })
      .then(async (res) => {
        if (cancelled) return;
        if (res.ok) {
          const data = (await res.json()) as { user: string };
          setUser(data.user);
          setStatus('authed');
        } else {
          setStatus('guest');
        }
      })
      .catch(() => {
        if (!cancelled) setStatus('guest');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const logout = () => {
    window.location.href = '/auth/logout';
  };

  return (
    <SessionContext.Provider value={{ user, status, logout }}>
      {children}
    </SessionContext.Provider>
  );
}

export function useSession() {
  const ctx = useContext(SessionContext);
  if (!ctx) throw new Error('useSession must be used within SessionProvider');
  return ctx;
}
