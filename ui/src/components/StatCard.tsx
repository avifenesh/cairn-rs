import { clsx } from "clsx";
import { HelpTooltip } from "./HelpTooltip";

export type StatCardVariant = "default" | "success" | "warning" | "danger" | "info";

interface StatCardProps {
  label: string;
  value: number | string;
  description?: string;
  variant?: StatCardVariant;
  loading?: boolean;
  /**
   * Compact mode: renders as an inline border-l-2 stat without card background.
   * Used inside stat strips (RunDetailPage, SessionDetailPage, etc.).
   */
  compact?: boolean;
  /** Reserved for future icon support — not yet rendered. */
  icon?: React.ReactNode;
  /** Optional inline help text shown as a (?) tooltip next to the label. */
  help?: string;
  className?: string;
}

const ACCENT: Record<StatCardVariant, string> = {
  default: "border-l-gray-300 dark:border-l-zinc-700",
  success: "border-l-emerald-500",
  warning: "border-l-amber-500",
  danger:  "border-l-red-500",
  info:    "border-l-indigo-500",
};

const VALUE: Record<StatCardVariant, string> = {
  default: "text-gray-900 dark:text-zinc-100",
  success: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  danger:  "text-red-600 dark:text-red-400",
  info:    "text-indigo-600 dark:text-indigo-400",
};

export function StatCard({ label, value, description, variant = "default", loading = false, compact = false, help, className }: StatCardProps) {
  if (loading) {
    return compact ? (
      <div data-stat-card className={clsx("border-l-2 pl-3 py-0.5 animate-pulse", ACCENT[variant], className)}>
        <div className="h-2 w-16 rounded bg-gray-200 dark:bg-zinc-800 mb-2" />
        <div className="h-5 w-10 rounded bg-gray-200 dark:bg-zinc-800" />
      </div>
    ) : (
      <div data-stat-card className={clsx("bg-white dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 border-l-gray-300 dark:border-l-zinc-700 rounded-lg p-4 animate-pulse", className)}>
        <div className="h-2 w-18 rounded bg-gray-200 dark:bg-zinc-800 mb-3" />
        <div className="h-6 w-12 rounded bg-gray-200 dark:bg-zinc-800" />
      </div>
    );
  }

  if (compact) {
    return (
      <div data-stat-card className={clsx("border-l-2 pl-3 py-0.5", ACCENT[variant], className)}>
        <p data-stat-label className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
          {label}
        </p>
        <p data-stat-value className={clsx("text-[20px] font-semibold tabular-nums leading-tight", VALUE[variant])}>
          {value}
        </p>
        {description && (
          <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{description}</p>
        )}
      </div>
    );
  }

  return (
    <div
      data-stat-card
      className={clsx(
        "bg-white dark:bg-zinc-900",
        "border border-gray-200 dark:border-zinc-800",
        "border-l-2 rounded-lg p-4",
        ACCENT[variant],
        className,
      )}
    >
      <p data-stat-label className="text-[11px] font-medium text-gray-500 dark:text-zinc-500 uppercase tracking-wider mb-2 flex items-center gap-1.5 truncate">
        <span className="truncate">{label}</span>
        {help && <HelpTooltip text={help} placement="top" className="shrink-0" />}
      </p>
      <p data-stat-value className={clsx("text-[20px] font-semibold tabular-nums leading-none", VALUE[variant])}>
        {value}
      </p>
      {description && (
        <p className="mt-1.5 text-[11px] text-gray-400 dark:text-zinc-600 truncate">{description}</p>
      )}
    </div>
  );
}
