/**
 * PageHeader — standardized page header with title, subtitle, and actions.
 *
 * Extracts the best pattern from DashboardPage, MetricsPage, ProjectDashboardPage.
 * Supports back navigation, status indicators, and right-aligned action slots.
 */

import { ArrowLeft } from "lucide-react";
import { clsx } from "clsx";

interface PageHeaderProps {
  /** Page title (e.g. "Runs", "Overview"). */
  title: string;
  /** Optional subtitle below the title. */
  subtitle?: string;
  /** Section label above the title (uppercase, small). */
  sectionLabel?: string;
  /** Back navigation — renders an ArrowLeft link. */
  onBack?: () => void;
  /** Back link text (default: "Back"). */
  backLabel?: string;
  /** Right-aligned action buttons / controls. */
  actions?: React.ReactNode;
  /** Status indicator (e.g. health badge) rendered between title and actions. */
  status?: React.ReactNode;
  className?: string;
}

export function PageHeader({
  title,
  subtitle,
  sectionLabel,
  onBack,
  backLabel = "Back",
  actions,
  status,
  className,
}: PageHeaderProps) {
  return (
    <div className={clsx("space-y-3", className)}>
      {onBack && (
        <button
          onClick={onBack}
          className="flex items-center gap-1.5 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 focus-visible:ring-offset-1"
        >
          <ArrowLeft size={13} /> {backLabel}
        </button>
      )}

      <div className="flex items-start justify-between gap-4 flex-wrap">
        <div className="min-w-0">
          {sectionLabel && (
            <p className="text-[11px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">
              {sectionLabel}
            </p>
          )}
          <h2 className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
            {title}
          </h2>
          {subtitle && (
            <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">
              {subtitle}
            </p>
          )}
        </div>

        {(status || actions) && (
          <div className="flex items-center gap-3 shrink-0">
            {status}
            {actions}
          </div>
        )}
      </div>
    </div>
  );
}
