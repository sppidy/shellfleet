'use client';

import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * Draggable horizontal splitter. Two children, the left one's width
 * is controlled by a percentage that the operator can drag. Persists
 * the percentage to localStorage under the supplied key so a refresh
 * keeps the layout.
 *
 * Keep it minimal — no animations, no react-resizable, no portals.
 */
export default function HSplitter({
  storageKey,
  defaultLeftPct = 50,
  minLeftPct = 20,
  maxLeftPct = 80,
  left,
  right,
}: {
  storageKey: string;
  defaultLeftPct?: number;
  minLeftPct?: number;
  maxLeftPct?: number;
  left: React.ReactNode;
  right: React.ReactNode;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const [leftPct, setLeftPct] = useState<number>(() => {
    if (typeof window === 'undefined') return defaultLeftPct;
    const v = window.localStorage.getItem(storageKey);
    if (!v) return defaultLeftPct;
    const n = parseFloat(v);
    return Number.isFinite(n) ? Math.max(minLeftPct, Math.min(maxLeftPct, n)) : defaultLeftPct;
  });

  useEffect(() => {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem(storageKey, String(leftPct));
  }, [storageKey, leftPct]);

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    draggingRef.current = true;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }, []);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!draggingRef.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      if (rect.width <= 0) return;
      const pct = ((e.clientX - rect.left) / rect.width) * 100;
      const clamped = Math.max(minLeftPct, Math.min(maxLeftPct, pct));
      setLeftPct(clamped);
    };
    const onUp = () => {
      if (draggingRef.current) {
        draggingRef.current = false;
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
      }
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
  }, [minLeftPct, maxLeftPct]);

  return (
    <div
      ref={containerRef}
      style={{
        display: 'flex',
        flexDirection: 'row',
        flex: 1,
        minHeight: 0,
        minWidth: 0,
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          width: `${leftPct}%`,
          minWidth: 0,
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
        }}
      >
        {left}
      </div>
      <div
        onMouseDown={onMouseDown}
        title="drag to resize"
        style={{
          width: 4,
          flexShrink: 0,
          cursor: 'col-resize',
          background: 'var(--line)',
          position: 'relative',
        }}
      >
        {/* Wider hit-target around the visual line. */}
        <span
          style={{
            position: 'absolute',
            left: -4,
            right: -4,
            top: 0,
            bottom: 0,
          }}
        />
      </div>
      <div
        style={{
          flex: 1,
          minWidth: 0,
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
        }}
      >
        {right}
      </div>
    </div>
  );
}
