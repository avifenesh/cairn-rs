import { Fragment } from 'react';
import { clsx } from 'clsx';

export interface BreadcrumbItem {
  label: string;
  /** If set, renders as a clickable link navigating to this hash. */
  href?: string;
}

interface BreadcrumbProps {
  items: BreadcrumbItem[];
  className?: string;
}

export function Breadcrumb({ items, className }: BreadcrumbProps) {
  if (items.length === 0) return null;

  return (
    <nav aria-label="breadcrumb" className={clsx('flex items-center gap-1 text-[12px] min-w-0', className)}>
      {items.map((item, i) => {
        const isLast = i === items.length - 1;
        return (
          <Fragment key={i}>
            {i > 0 && (
              <span className="text-gray-300 dark:text-zinc-600 select-none shrink-0" aria-hidden>
                /
              </span>
            )}
            {item.href && !isLast ? (
              <a
                href={item.href}
                onClick={(e) => {
                  e.preventDefault();
                  window.location.hash = item.href!.replace(/^#/, '');
                }}
                className="shrink-0 text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-gray-700 dark:hover:text-zinc-300 transition-colors whitespace-nowrap"
              >
                {item.label}
              </a>
            ) : isLast ? (
              <span
                className="text-gray-900 dark:text-zinc-200 font-medium truncate"
                aria-current="page"
              >
                {item.label}
              </span>
            ) : (
              <span className="shrink-0 text-gray-400 dark:text-zinc-500 whitespace-nowrap">
                {item.label}
              </span>
            )}
          </Fragment>
        );
      })}
    </nav>
  );
}
