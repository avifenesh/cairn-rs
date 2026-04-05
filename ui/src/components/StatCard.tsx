import type { LucideIcon } from "lucide-react";
import { clsx } from "clsx";

export type StatCardVariant = "default" | "success" | "warning" | "danger" | "info";

interface StatCardProps {
  /** Metric label shown below the number */
  label: string;
  /** The primary numeric value */
  value: number | string;
  /** Optional sub-label / context line */
  description?: string;
  /** Lucide icon component */
  icon?: LucideIcon;
  /** Colour variant — controls the icon background and indicator dot */
  variant?: StatCardVariant;
  /** Show a loading skeleton when true */
  loading?: boolean;
}

const variantStyles: Record<StatCardVariant, { dot: string; icon: string; ring: string }> = {
  default: {
    dot:  "bg-zinc-400",
    icon: "bg-zinc-800 text-zinc-300",
    ring: "ring-zinc-700",
  },
  success: {
    dot:  "bg-emerald-400",
    icon: "bg-emerald-950 text-emerald-400",
    ring: "ring-emerald-900/50",
  },
  warning: {
    dot:  "bg-amber-400",
    icon: "bg-amber-950 text-amber-400",
    ring: "ring-amber-900/50",
  },
  danger: {
    dot:  "bg-red-400",
    icon: "bg-red-950 text-red-400",
    ring: "ring-red-900/50",
  },
  info: {
    dot:  "bg-blue-400",
    icon: "bg-blue-950 text-blue-400",
    ring: "ring-blue-900/50",
  },
};

export function StatCard({
  label,
  value,
  description,
  icon: Icon,
  variant = "default",
  loading = false,
}: StatCardProps) {
  const styles = variantStyles[variant];

  if (loading) {
    return (
      <div className="rounded-xl bg-zinc-900 p-5 ring-1 ring-zinc-800 animate-pulse">
        <div className="flex items-start justify-between">
          <div className="space-y-2">
            <div className="h-3 w-24 rounded bg-zinc-700" />
            <div className="h-8 w-16 rounded bg-zinc-700" />
          </div>
          <div className="h-9 w-9 rounded-lg bg-zinc-800" />
        </div>
        <div className="mt-3 h-3 w-32 rounded bg-zinc-800" />
      </div>
    );
  }

  return (
    <div
      className={clsx(
        "rounded-xl bg-zinc-900 p-5 ring-1 ring-zinc-800 transition-all",
        "hover:ring-zinc-700 hover:shadow-lg hover:shadow-black/30"
      )}
    >
      <div className="flex items-start justify-between gap-3">
        {/* Value + label */}
        <div className="min-w-0">
          <p className="text-sm font-medium text-zinc-400 truncate">{label}</p>
          <p className="mt-1 text-3xl font-semibold tracking-tight text-zinc-50">
            {value}
          </p>
        </div>

        {/* Icon badge */}
        {Icon && (
          <div
            className={clsx(
              "flex h-10 w-10 shrink-0 items-center justify-center rounded-lg",
              styles.icon
            )}
          >
            <Icon size={18} strokeWidth={2} />
          </div>
        )}
      </div>

      {/* Description row with coloured indicator dot */}
      {description && (
        <div className="mt-3 flex items-center gap-1.5">
          <span className={clsx("h-2 w-2 shrink-0 rounded-full", styles.dot)} />
          <p className="text-xs text-zinc-500 truncate">{description}</p>
        </div>
      )}
    </div>
  );
}
