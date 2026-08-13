'use client';

import { useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useWebSocket } from '@/components/providers/WebSocketProvider';
import { useSession } from '@/components/providers/SessionProvider';

type Item = {
  ico: string;
  label: string;
  meta: string;
  action: () => void;
};

export default function CommandPalette({
  onSelectAgent,
}: {
  onSelectAgent: (agentId: string) => void;
}) {
  const router = useRouter();
  const { agents } = useWebSocket();
  const { role } = useSession();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [idx, setIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const items: Item[] = useMemo(() => {
    const base: Item[] = [
      { ico: '▤', label: 'Fleet overview', meta: 'go', action: () => router.push('/') },
      { ico: '↗', label: 'Fan-out', meta: 'go', action: () => router.push('/fan-out') },
      { ico: '≡', label: 'Activity log', meta: 'go', action: () => router.push('/activity') },
      { ico: '◇', label: 'Notifications', meta: 'go', action: () => router.push('/notifications') },
      { ico: '⚿', label: 'Manage tokens', meta: 'go', action: () => router.push('/tokens') },
      { ico: '＋', label: 'Connect new agent', meta: 'go', action: () => router.push('/device') },
      { ico: '⌘', label: 'Account & 2FA', meta: 'go', action: () => router.push('/security') },
      // Admin entry only for admins — viewers landing on /admin just hit the
      // 'access required' page (the sidebar already role-gates this link).
      ...(role === 'admin'
        ? [{ ico: '⌬', label: 'Admin · users & seats', meta: 'go', action: () => router.push('/admin') }]
        : []),
      { ico: '›_', label: 'Terminal · multi-host', meta: 'go', action: () => router.push('/terminal') },
    ];
    const agentItems: Item[] = agents.map((a) => ({
      ico: '▢',
      label: 'host: ' + a.replace(/-id$/, ''),
      meta: 'agent',
      action: () => onSelectAgent(a),
    }));
    return [...base, ...agentItems];
  }, [router, agents, onSelectAgent, role]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter((i) => i.label.toLowerCase().includes(q));
  }, [items, query]);

  useEffect(() => {
    setIdx(0);
  }, [query, open]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setOpen((o) => !o);
        return;
      }
      if (!open) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        setOpen(false);
      } else if (e.key === 'ArrowDown') {
        e.preventDefault();
        setIdx((i) => Math.min(filtered.length - 1, i + 1));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setIdx((i) => Math.max(0, i - 1));
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const it = filtered[idx];
        if (it) {
          it.action();
          setOpen(false);
        }
      }
    }
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, filtered, idx]);

  useEffect(() => {
    if (open) {
      setQuery('');
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && setOpen(false)}>
      <div className="palette">
        <input
          ref={inputRef}
          className="palette-input"
          placeholder="Type a command, host, or container…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <div className="palette-list">
          {filtered.length === 0 ? (
            <div className="palette-row" style={{ color: 'var(--fg-3)' }}>No matches</div>
          ) : (
            filtered.map((it, i) => (
              <div
                key={it.label}
                className={`palette-row ${i === idx ? 'on' : ''}`}
                onMouseEnter={() => setIdx(i)}
                onClick={() => {
                  it.action();
                  setOpen(false);
                }}
              >
                <span className="ico">{it.ico}</span>
                <span>{it.label}</span>
                <span className="meta">{it.meta}</span>
              </div>
            ))
          )}
        </div>
        <div className="palette-foot">
          <span><kbd>↑↓</kbd> navigate</span>
          <span><kbd>↵</kbd> open</span>
          <span><kbd>esc</kbd> close</span>
          <span style={{ marginLeft: 'auto' }}>
            try: <span style={{ color: 'var(--accent)' }}>/host</span>{' '}
            <span style={{ color: 'var(--accent)' }}>/run</span>
          </span>
        </div>
      </div>
    </div>
  );
}
