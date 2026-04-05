import { clsx } from "clsx";

export type StatCardVariant = "default" | "success" | "warning" | "danger" | "info";

interface StatCardProps {
  label: string;
  value: number | string;
  description?: string;
  variant?: StatCardVariant;
  loading?: boolean;
  // icon prop kept for API compat but not rendered
  icon?: unknown;
}

/** 2-px left border colour per variant */
const BORDER_COLOR: Record<StatCardVariant, string> = {
  default: "border-l-zinc-700",
  success: "border-l-emerald-500",
  warning: "border-l-amber-500",
  danger:  "border-l-red-500",
  info:    "border-l-blue-500",
};

/** Number colour per variant */
const VALUE_COLOR: Record<StatCardVariant, string> = {
  default: "text-zinc-100",
  success: "text-emerald-400",
  warning: "text-amber-400",
  danger:  "text-red-400",
  info:    "text-blue-400",
};

export function StatCard({
  label,
  value,
  description,
  variant = "default",
  loading = false,
}: StatCardProps) {
  if (loading) {
    return (
      <div className="bg-zinc-900 border border-zinc-800 border-l-2 border-l-zinc-700 rounded-lg p-4 animate-pulse">
        <div className="h-2.5 w-20 rounded bg-zinc-800 mb-3" />
        <div className="h-7 w-14 rounded bg-zinc-700" />
      </div>
    );
  }

  return (
    <div
      className={clsx(
        "bg-zinc-900 border border-zinc-800 border-l-2 rounded-lg p-4",
        "transition-colors hover:border-zinc-700 hover:border-l-current",
        BORDER_COLOR[variant]
      )}
    >
      <p className="text-xs font-medium text-zinc-500 uppercase tracking-wider mb-2 truncate">
        {label}
      </p>
      <p className={clsx("text-2xl font-semibold tabular-nums", VALUE_COLOR[variant])}>
        {value}
      </p>
      {description && (
        <p className="mt-1.5 text-xs text-zinc-600 truncate">{description}</p>
      )}
    </div>
  );
}
