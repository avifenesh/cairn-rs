/**
 * Toast notification system — no external deps.
 *
 * Variants: success · error · warning · info
 * Design:   zinc-900 bg, 2px left border accent, bottom-right stack
 * Dismiss:  auto after 4 s, or manual X button
 *
 * Usage:
 *   // 1. <ToastProvider> wraps the app (already in main.tsx).
 *   // 2. const toast = useToast();
 *   // 3. toast.success('Saved!') / toast.error('Failed.') / toast.info(...) / toast.warning(...)
 */

import {
  useState, useCallback, useRef,
  createContext, useContext,
  type ReactNode,
} from 'react';
import { CheckCircle2, XCircle, AlertTriangle, Info, X } from 'lucide-react';
import { clsx } from 'clsx';

// ── Types ─────────────────────────────────────────────────────────────────────

export type ToastVariant = 'success' | 'error' | 'warning' | 'info';

interface ToastItem {
  id:      number;
  message: string;
  variant: ToastVariant;
}

export interface ToastAPI {
  success: (message: string) => void;
  error:   (message: string) => void;
  warning: (message: string) => void;
  info:    (message: string) => void;
}

// ── Variant config ────────────────────────────────────────────────────────────

const VARIANT: Record<ToastVariant, {
  border: string; icon: typeof CheckCircle2; iconCls: string; textCls: string;
}> = {
  success: { border: 'border-l-emerald-500', icon: CheckCircle2,  iconCls: 'text-emerald-400', textCls: 'text-gray-800 dark:text-zinc-200' },
  error:   { border: 'border-l-red-500',     icon: XCircle,        iconCls: 'text-red-400',     textCls: 'text-gray-800 dark:text-zinc-200' },
  warning: { border: 'border-l-amber-500',   icon: AlertTriangle,  iconCls: 'text-amber-400',   textCls: 'text-gray-800 dark:text-zinc-200' },
  info:    { border: 'border-l-blue-500',    icon: Info,           iconCls: 'text-blue-400',    textCls: 'text-gray-700 dark:text-zinc-300' },
};

// ── Context ───────────────────────────────────────────────────────────────────

const ToastContext = createContext<ToastAPI | null>(null);

// ── Provider ──────────────────────────────────────────────────────────────────

let _nextId = 0;
const DISMISS_MS = 4_000;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const timers = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const dismiss = useCallback((id: number) => {
    clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const add = useCallback((message: string, variant: ToastVariant) => {
    const id = ++_nextId;
    setToasts((prev) => [...prev.slice(-4), { id, message, variant }]); // max 5 visible
    const timer = setTimeout(() => dismiss(id), DISMISS_MS);
    timers.current.set(id, timer);
  }, [dismiss]);

  const api: ToastAPI = {
    success: (m) => add(m, 'success'),
    error:   (m) => add(m, 'error'),
    warning: (m) => add(m, 'warning'),
    info:    (m) => add(m, 'info'),
  };

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div
        aria-live="polite"
        className="fixed bottom-5 right-5 z-50 flex flex-col gap-2 pointer-events-none"
        style={{ maxWidth: '22rem' }}
      >
        {toasts.map((t) => (
          <ToastCard key={t.id} toast={t} onDismiss={() => dismiss(t.id)} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

// ── Toast card ────────────────────────────────────────────────────────────────

function ToastCard({ toast, onDismiss }: { toast: ToastItem; onDismiss: () => void }) {
  const v = VARIANT[toast.variant];
  const Icon = v.icon;

  return (
    <div
      role="alert"
      className={clsx(
        // base
        'pointer-events-auto flex items-start gap-3',
        'bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 rounded-lg px-4 py-3',
        'shadow-lg shadow-black/40',
        // slide-in from right (Tailwind v4 animation or fallback)
        'translate-x-0 opacity-100',
        v.border,
      )}
      style={{
        animation: 'toastIn 180ms cubic-bezier(.16,1,.3,1) both',
      }}
    >
      <Icon size={14} className={clsx('mt-0.5 shrink-0', v.iconCls)} />
      <p className={clsx('flex-1 text-xs leading-relaxed', v.textCls)}>
        {toast.message}
      </p>
      <button
        onClick={onDismiss}
        className="shrink-0 mt-0.5 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors"
        aria-label="Dismiss"
      >
        <X size={12} />
      </button>
    </div>
  );
}

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useToast(): ToastAPI {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error('useToast must be used inside <ToastProvider>');
  return ctx;
}
