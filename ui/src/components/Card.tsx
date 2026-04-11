/**
 * Card — base container component.
 *
 * Extracts the Panel pattern duplicated in DashboardPage, ProjectDashboardPage,
 * and others. Three variants cover all observed uses.
 */

import { clsx } from "clsx";

export type CardVariant = "default" | "shell" | "inner";

interface CardProps {
  children: React.ReactNode;
  /**
   * - "default": standard panel with padding (most common).
   * - "shell": no padding, for wrapping tables/lists that manage their own padding.
   * - "inner": elevated surface for nesting inside another card.
   */
  variant?: CardVariant;
  className?: string;
}

const VARIANT_CLASSES: Record<CardVariant, string> = {
  default: "bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg p-4",
  shell:   "bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden",
  inner:   "bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden",
};

export function Card({ children, variant = "default", className }: CardProps) {
  return (
    <div className={clsx(VARIANT_CLASSES[variant], className)}>
      {children}
    </div>
  );
}

// ── Card sub-components ──────────────────────────────────────────────────────

interface CardHeaderProps {
  children: React.ReactNode;
  /** Right-aligned actions. */
  actions?: React.ReactNode;
  className?: string;
}

/** Header row for shell-variant cards. */
export function CardHeader({ children, actions, className }: CardHeaderProps) {
  return (
    <div className={clsx("flex items-center justify-between px-4 h-9 border-b border-gray-200 dark:border-zinc-800", className)}>
      <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
        {children}
      </p>
      {actions}
    </div>
  );
}
