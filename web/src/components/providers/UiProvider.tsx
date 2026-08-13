'use client';

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

type ToastKind = 'success' | 'error' | 'info';

interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

interface ConfirmOptions {
  title: string;
  description?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
}

interface UiContextValue {
  toast: (kind: ToastKind, text: string) => void;
  confirm: (opts: ConfirmOptions) => Promise<boolean>;
}

const Ctx = createContext<UiContextValue | null>(null);

export function UiProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [pendingConfirm, setPendingConfirm] = useState<{
    opts: ConfirmOptions;
    resolve: (b: boolean) => void;
  } | null>(null);
  const idRef = useRef(0);

  const toast = useCallback((kind: ToastKind, text: string) => {
    const id = ++idRef.current;
    setToasts((prev) => [...prev, { id, kind, text }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 4500);
  }, []);

  const confirm = useCallback((opts: ConfirmOptions) => {
    return new Promise<boolean>((resolve) => {
      setPendingConfirm({ opts, resolve });
    });
  }, []);

  const value: UiContextValue = useMemo(() => ({ toast, confirm }), [toast, confirm]);

  useEffect(() => {
    if (!pendingConfirm) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        pendingConfirm.resolve(false);
        setPendingConfirm(null);
      } else if (e.key === 'Enter' && !pendingConfirm.opts.destructive) {
        // Destructive confirms require an explicit click — a stray Enter
        // (a common reflex) must not force-remove a container/service/backup.
        pendingConfirm.resolve(true);
        setPendingConfirm(null);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [pendingConfirm]);

  return (
    <Ctx.Provider value={value}>
      {children}

      <div className="toasts">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={`toast ${t.kind === 'error' ? 'err' : t.kind === 'info' ? '' : ''}`}
            style={{
              borderLeftColor:
                t.kind === 'success'
                  ? 'var(--accent)'
                  : t.kind === 'error'
                    ? 'var(--err)'
                    : 'var(--info)',
            }}
          >
            <div className="row between" style={{ alignItems: 'flex-start' }}>
              <div style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word', minWidth: 0 }}>
                <span
                  style={{
                    color:
                      t.kind === 'success'
                        ? 'var(--accent)'
                        : t.kind === 'error'
                          ? 'var(--err)'
                          : 'var(--info)',
                    marginRight: 6,
                  }}
                >
                  {t.kind === 'success' ? '✓' : t.kind === 'error' ? '×' : 'i'}
                </span>
                {t.text}
              </div>
              <button
                type="button"
                onClick={() => setToasts((prev) => prev.filter((x) => x.id !== t.id))}
                className="icon-btn"
                style={{ width: 18, height: 18 }}
                aria-label="Dismiss"
              >
                ×
              </button>
            </div>
          </div>
        ))}
      </div>

      {pendingConfirm && (
        <div
          className="modal-overlay"
          onClick={() => {
            pendingConfirm.resolve(false);
            setPendingConfirm(null);
          }}
        >
          <div
            className={`modal ${pendingConfirm.opts.destructive ? 'danger' : ''}`}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="panel-head">
              <div
                className="panel-title"
                style={{
                  color: pendingConfirm.opts.destructive ? 'var(--err)' : 'var(--accent)',
                }}
              >
                {pendingConfirm.opts.destructive ? '⚠' : '?'} {pendingConfirm.opts.title}
              </div>
            </div>
            {pendingConfirm.opts.description && (
              <div
                className="panel-body"
                style={{
                  color: 'var(--fg-1)',
                  fontSize: 12,
                  whiteSpace: 'pre-wrap',
                }}
              >
                {pendingConfirm.opts.description}
              </div>
            )}
            <div className="modal-foot">
              <button
                type="button"
                className="btn"
                onClick={() => {
                  pendingConfirm.resolve(false);
                  setPendingConfirm(null);
                }}
              >
                {pendingConfirm.opts.cancelLabel ?? 'cancel'}
              </button>
              <button
                type="button"
                autoFocus
                className={`btn ${pendingConfirm.opts.destructive ? 'danger' : 'primary'}`}
                onClick={() => {
                  pendingConfirm.resolve(true);
                  setPendingConfirm(null);
                }}
              >
                {pendingConfirm.opts.confirmLabel ??
                  (pendingConfirm.opts.destructive ? 'confirm' : 'ok')}
              </button>
            </div>
          </div>
        </div>
      )}
    </Ctx.Provider>
  );
}

export function useUi() {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error('useUi must be used within UiProvider');
  return ctx;
}
