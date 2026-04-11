import type { ReactNode } from "react";
import { clsx } from "clsx";

/**
 * Consistent empty state for feature pages that need operator action.
 * Shows: icon + title + description + optional action link.
 */
export function FeatureEmptyState({
  icon,
  title,
  description,
  actionLabel,
  actionHref,
  className,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  actionLabel?: string;
  actionHref?: string;
  className?: string;
}) {
  return (
    <div className={clsx("flex flex-col items-center justify-center py-14 px-6 gap-4 text-center", className)}>
      <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
        {icon}
      </div>
      <div className="max-w-sm">
        <p className="text-[13px] font-medium text-gray-600 dark:text-zinc-300">{title}</p>
        <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-1.5 leading-relaxed">
          {description}
        </p>
      </div>
      {actionLabel && actionHref && (
        <a
          href={actionHref}
          className={clsx(
            "inline-flex items-center gap-1.5 rounded-md px-4 py-2 text-[12px] font-medium transition-colors",
            "bg-indigo-600 hover:bg-indigo-500 text-white",
          )}
        >
          {actionLabel}
        </a>
      )}
    </div>
  );
}
