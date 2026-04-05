/**
 * Minimal toast notification system — no external deps.
 *
 * Usage:
 *   import { useToast, Toaster } from '../components/Toast';
 *
 *   // 1. Mount <Toaster /> once near the root (Layout or App).
 *   // 2. Call `const toast = useToast()` in any component.
 *   // 3. toast.success('Saved!') / toast.error('Failed.')
 */

import {
  useState,
  useCallback,
  useRef,
  createContext,
  useContext,
  type ReactNode,
} from 'react';
import { CheckCircle2, XCircle, X } from 'lucide-react';
import { clsx } from 'clsx';

// ── Types ─────────────────────────────────────────────────────────────────────

type ToastVariant = 'success' | 'error';

interface ToastItem {
  id: number;
  message: string;
  variant: ToastVariant;
}

interface ToastAPI {
  success: (message: string) => void;
  error:   (message: string) => void;
}

// ── Context ───────────────────────────────────────────────────────────────────

const ToastContext = createContext<ToastAPI | null>(null);

// ── Provider + Toaster ────────────────────────────────────────────────────────

let _nextId = 0;
const AUTO_DISMISS_MS = 3_000;

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
    setToasts((prev) => [...prev, { id, message, variant }]);
    const timer = setTimeout(() => dismiss(id), AUTO_DISMISS_MS);
    timers.current.set(id, timer);
  }, [dismiss]);

  const api: ToastAPI = {
    success: (msg) => add(msg, 'success'),
    error:   (msg) => add(msg, 'error'),
  };

  return (
    <ToastContext.Provider value={api}>
      {children}
      {/* Portal-style fixed overlay */}
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

// ── Individual card ───────────────────────────────────────────────────────────

const VARIANT_STYLES: Record<ToastVariant, string> = {
  success: 'bg-emerald-950 ring-emerald-800 text-emerald-200',
  error:   'bg-red-950     ring-red-800     text-red-200',
};

const VARIANT_ICON: Record<ToastVariant, typeof CheckCircle2> = {
  success: CheckCircle2,
  error:   XCircle,
};

const ICON_COLOR: Record<ToastVariant, string> = {
  success: 'text-emerald-400',
  error:   'text-red-400',
};

interface ToastCardProps {
  toast: ToastItem;
  onDismiss: () => void;
}

function ToastCard({ toast, onDismiss }: ToastCardProps) {
  const Icon = VARIANT_ICON[toast.variant];

  return (
    <div
      role="alert"
      className={clsx(
        'pointer-events-auto flex items-start gap-3 rounded-xl px-4 py-3',
        'ring-1 shadow-xl shadow-black/40',
        'animate-in slide-in-from-right-4 fade-in duration-200',
        VARIANT_STYLES[toast.variant],
      )}
    >
      <Icon size={16} className={clsx('mt-0.5 shrink-0', ICON_COLOR[toast.variant])} />
      <p className="flex-1 text-sm leading-snug">{toast.message}</p>
      <button
        onClick={onDismiss}
        className="shrink-0 mt-0.5 opacity-60 hover:opacity-100 transition-opacity"
        aria-label="Dismiss"
      >
        <X size={13} />
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
