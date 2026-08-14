'use client';

import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';

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

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 10 : 2;
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      setLeftPct((current) => Math.max(minLeftPct, current - step));
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      setLeftPct((current) => Math.min(maxLeftPct, current + step));
    } else if (event.key === 'Home') {
      event.preventDefault();
      setLeftPct(minLeftPct);
    } else if (event.key === 'End') {
      event.preventDefault();
      setLeftPct(maxLeftPct);
    }
  }, [maxLeftPct, minLeftPct]);

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
    <div ref={containerRef} className="h-splitter">
      <div
        className="h-splitter-pane h-splitter-pane-left"
        style={{
          width: `${leftPct}%`,
        }}
      >
        {left}
      </div>
      <div
        onMouseDown={onMouseDown}
        onKeyDown={onKeyDown}
        className="h-splitter-handle"
        role="separator"
        aria-label="Resize overview and root shell panes"
        aria-orientation="vertical"
        aria-valuemin={minLeftPct}
        aria-valuemax={maxLeftPct}
        aria-valuenow={Math.round(leftPct)}
        tabIndex={0}
        title="drag to resize"
      >
        {/* Wider hit-target around the visual line. */}
        <span />
      </div>
      <div className="h-splitter-pane h-splitter-pane-right">
        {right}
      </div>
    </div>
  );
}
