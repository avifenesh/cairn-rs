import { WifiOff, ShieldOff, FileQuestion, AlertTriangle, RefreshCw } from 'lucide-react';
import { clsx } from 'clsx';

// ── Variant config ────────────────────────────────────────────────────────────

type Variant = 'network' | 'auth' | 'not_found' | 'generic';

function detectVariant(error: unknown): Variant {
  const msg = error instanceof Error ? error.message : String(error);
  if (/401|unauthorized|Unauthorized/.test(msg)) return 'auth';
  if (/404|not found|Not Found/.test(msg)) return 'not_found';
  if (/fetch|network|ECONNREFUSED|Failed to fetch|NetworkError/i.test(msg)) return 'network';
  return 'generic';
}

const VARIANT_META: Record<Variant, {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  iconClass: string;
  title: string;
  hint: string;
}> = {
  network: {
    icon: WifiOff,
    iconClass: 'text-amber-500',
    title: 'Server unreachable',
    hint: 'Check that cairn-app is running and the API URL is correct.',
  },
  auth: {
    icon: ShieldOff,
    iconClass: 'text-red-400',
    title: 'Authentication failed',
    hint: 'Your session may have expired. Sign out and sign in again.',
  },
  not_found: {
    icon: FileQuestion,
    iconClass: 'text-gray-400 dark:text-zinc-500',
    title: 'Resource not found',
    hint: 'The requested resource no longer exists or the URL is incorrect.',
  },
  generic: {
    icon: AlertTriangle,
    iconClass: 'text-red-400',
    title: 'Request failed',
    hint: 'An unexpected error occurred. Try refreshing the data.',
  },
};

// ── Component ─────────────────────────────────────────────────────────────────

interface ErrorFallbackProps {
  /** The error from TanStack Query or a manual throw. */
  error: unknown;
  /** Called when the user clicks Retry. */
  onRetry?: () => void;
  /** Override the auto-detected variant. */
  variant?: Variant;
  /** Optional context label, e.g. "runs" or "dashboard". */
  resource?: string;
  /** Use a compact single-line layout for inline use. */
  compact?: boolean;
}

/**
 * Drop-in error display for TanStack Query failure states.
 *
 * Detects the error type from the message and picks an appropriate
 * icon, title, and hint.  Provides a Retry button when `onRetry` is
 * supplied.
 */
export function ErrorFallback({
  error,
  onRetry,
  variant: variantProp,
  resource,
  compact = false,
}: ErrorFallbackProps) {
  const variant = variantProp ?? detectVariant(error);
  const { icon: Icon, iconClass, title, hint } = VARIANT_META[variant];
  const message = error instanceof Error ? error.message : String(error ?? 'Unknown error');

  if (compact) {
    return (
      <div className="flex items-center gap-2 px-4 py-3 rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 text-[12px]">
        <Icon size={13} className={clsx('shrink-0', iconClass)} />
        <span className="text-gray-500 dark:text-zinc-400 truncate">
          {resource ? `Failed to load ${resource}: ` : ''}{message}
        </span>
        {onRetry && (
          <button
            onClick={onRetry}
            className="ml-auto flex items-center gap-1 text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 transition-colors shrink-0"
          >
            <RefreshCw size={11} />
            Retry
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center justify-center min-h-48 gap-4 p-8 text-center">
      <div className="w-10 h-10 rounded-full bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 flex items-center justify-center">
        <Icon size={18} className={iconClass} />
      </div>

      <div className="space-y-1">
        <p className="text-[13px] font-medium text-gray-700 dark:text-zinc-300">
          {resource ? `Failed to load ${resource}` : title}
        </p>
        <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-sm">{hint}</p>
      </div>

      {/* Error detail */}
      <p className="text-[11px] font-mono text-gray-300 dark:text-zinc-700 bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800
                    rounded px-3 py-1.5 max-w-sm break-all">
        {message}
      </p>

      {onRetry && (
        <button
          onClick={onRetry}
          className="flex items-center gap-1.5 rounded bg-gray-100 dark:bg-zinc-800 hover:bg-gray-200 dark:hover:bg-zinc-700
                     border border-gray-200 dark:border-zinc-700 text-gray-700 dark:text-zinc-300 text-[12px] font-medium
                     px-3 py-1.5 transition-colors"
        >
          <RefreshCw size={12} />
          Retry
        </button>
      )}
    </div>
  );
}
