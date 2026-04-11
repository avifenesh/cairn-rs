/**
 * Badge — generic status pill with optional dot indicator.
 *
 * Extracts the best pattern from StateBadge, SessionPill, DecisionBadge, etc.
 * For run/task state-specific badges, continue using StateBadge.
 * Use Badge for general-purpose status indicators.
 */

import { clsx } from "clsx";

export type BadgeVariant =
  | "success" | "warning" | "danger" | "info"
  | "purple" | "sky" | "neutral" | "muted";

interface BadgeProps {
  /** Badge text. */
  children: React.ReactNode;
  /** Color variant. */
  variant?: BadgeVariant;
  /** Show a leading dot indicator. */
  dot?: boolean;
  /** Animate the dot (pulse). */
  dotPulse?: boolean;
  /** Use bordered/outlined style instead of filled. */
  outlined?: boolean;
  /** Compact sizing. */
  compact?: boolean;
  className?: string;
}

const VARIANT_FILLED: Record<BadgeVariant, string> = {
  success: "text-emerald-400 bg-emerald-500/10",
  warning: "text-amber-400 bg-amber-500/10",
  danger:  "text-red-400 bg-red-500/10",
  info:    "text-indigo-400 bg-indigo-500/10",
  purple:  "text-violet-400 bg-violet-500/10",
  sky:     "text-sky-400 bg-sky-500/10",
  neutral: "text-gray-500 dark:text-zinc-400 bg-gray-100/80 dark:bg-zinc-800/80",
  muted:   "text-gray-400 dark:text-zinc-500 bg-gray-100/60 dark:bg-zinc-800/60",
};

const VARIANT_OUTLINED: Record<BadgeVariant, string> = {
  success: "text-emerald-400 bg-emerald-500/10 border-emerald-500/20",
  warning: "text-amber-400 bg-amber-400/10 border-amber-400/20",
  danger:  "text-red-400 bg-red-500/10 border-red-500/20",
  info:    "text-indigo-400 bg-indigo-500/10 border-indigo-500/20",
  purple:  "text-violet-400 bg-violet-500/10 border-violet-500/20",
  sky:     "text-sky-400 bg-sky-500/10 border-sky-500/20",
  neutral: "text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700",
  muted:   "text-gray-400 dark:text-zinc-500 bg-gray-100/60 dark:bg-zinc-800/60 border-gray-200 dark:border-zinc-700",
};

const DOT_COLORS: Record<BadgeVariant, string> = {
  success: "bg-emerald-500",
  warning: "bg-amber-500",
  danger:  "bg-red-500",
  info:    "bg-indigo-400",
  purple:  "bg-violet-400",
  sky:     "bg-sky-400",
  neutral: "bg-zinc-500",
  muted:   "bg-zinc-600",
};

export function Badge({
  children,
  variant = "neutral",
  dot = false,
  dotPulse = false,
  outlined = false,
  compact = false,
  className,
}: BadgeProps) {
  const colors = outlined ? VARIANT_OUTLINED[variant] : VARIANT_FILLED[variant];

  return (
    <span
      className={clsx(
        "inline-flex items-center gap-1.5 rounded font-medium whitespace-nowrap",
        outlined && "border",
        compact ? "px-1.5 py-0.5 text-[10px]" : "px-1.5 py-0.5 text-[11px]",
        colors,
        className,
      )}
    >
      {dot && (
        <span
          className={clsx(
            "w-1.5 h-1.5 rounded-full shrink-0",
            DOT_COLORS[variant],
            dotPulse && "animate-pulse",
          )}
        />
      )}
      {children}
    </span>
  );
}
