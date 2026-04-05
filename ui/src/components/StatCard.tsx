import { clsx } from "clsx";

export type StatCardVariant = "default" | "success" | "warning" | "danger" | "info";

interface StatCardProps {
  label: string;
  value: number | string;
  description?: string;
  variant?: StatCardVariant;
  loading?: boolean;
  icon?: unknown;
}

const ACCENT: Record<StatCardVariant, string> = {
  default: "border-l-zinc-700",
  success: "border-l-emerald-500",
  warning: "border-l-amber-500",
  danger:  "border-l-red-500",
  info:    "border-l-indigo-500",
};

const VALUE: Record<StatCardVariant, string> = {
  default: "text-zinc-100",
  success: "text-emerald-400",
  warning: "text-amber-400",
  danger:  "text-red-400",
  info:    "text-indigo-400",
};

export function StatCard({ label, value, description, variant = "default", loading = false }: StatCardProps) {
  if (loading) {
    return (
      <div className="bg-zinc-900 border border-zinc-800 border-l-2 border-l-zinc-700 rounded-lg p-4 animate-pulse">
        <div className="h-2 w-18 rounded bg-zinc-800 mb-3" />
        <div className="h-6 w-12 rounded bg-zinc-800" />
      </div>
    );
  }

  return (
    <div className={clsx(
      "bg-zinc-900 border border-zinc-800 border-l-2 rounded-lg p-4",
      ACCENT[variant],
    )}>
      <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2 truncate">
        {label}
      </p>
      <p className={clsx("text-xl font-semibold tabular-nums leading-none", VALUE[variant])}>
        {value}
      </p>
      {description && (
        <p className="mt-1.5 text-[11px] text-zinc-600 truncate">{description}</p>
      )}
    </div>
  );
}
