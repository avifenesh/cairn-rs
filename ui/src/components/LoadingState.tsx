/**
 * Loading and empty-state components.
 *
 * - TableSkeleton   — animated pulse rows matching compact h-9 table rows
 * - CardSkeleton    — animated pulse for stat-card grids
 * - Spinner         — inline loading spinner
 * - EmptyState      — zero-data placeholder with title + optional sub-text
 */

import { Loader2 } from 'lucide-react';
import { clsx } from 'clsx';

// ── Table skeleton ────────────────────────────────────────────────────────────

interface TableSkeletonProps {
  /** Number of placeholder rows (default 6). */
  rows?: number;
  /** Number of columns (default 5). */
  cols?: number;
  className?: string;
}

export function TableSkeleton({ rows = 6, cols = 5, className }: TableSkeletonProps) {
  // Column width hints cycle: wide, medium, small, medium, small
  const widths = ['w-40', 'w-28', 'w-16', 'w-32', 'w-20'];

  return (
    <div className={clsx('animate-pulse', className)}>
      {Array.from({ length: rows }).map((_, ri) => (
        <div
          key={ri}
          className={clsx(
            'flex items-center gap-4 px-4 h-9 border-b border-zinc-800/50',
            ri % 2 === 0 ? 'bg-zinc-900' : 'bg-zinc-900/50',
          )}
        >
          {Array.from({ length: cols }).map((_, ci) => (
            <div
              key={ci}
              className={clsx(
                'h-2 rounded-sm bg-zinc-800',
                widths[ci % widths.length],
                ci === cols - 1 && 'ml-auto', // last col right-aligned
              )}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

// ── Card skeleton ─────────────────────────────────────────────────────────────

interface CardSkeletonProps {
  count?: number;   // number of cards in the grid
  className?: string;
}

export function CardSkeleton({ count = 4, className }: CardSkeletonProps) {
  return (
    <div className={clsx('grid gap-3', count === 3 ? 'grid-cols-3' : 'grid-cols-2 lg:grid-cols-4', className)}>
      {Array.from({ length: count }).map((_, i) => (
        <div
          key={i}
          className="bg-zinc-900 border border-zinc-800 border-l-2 border-l-zinc-700 rounded-lg p-4 animate-pulse"
        >
          <div className="h-2.5 w-20 rounded-sm bg-zinc-800 mb-3" />
          <div className="h-6 w-14 rounded-sm bg-zinc-800" />
        </div>
      ))}
    </div>
  );
}

// ── Spinner ───────────────────────────────────────────────────────────────────

interface SpinnerProps {
  size?: number;
  className?: string;
  label?: string;
}

export function Spinner({ size = 16, className, label }: SpinnerProps) {
  return (
    <span className={clsx('inline-flex items-center gap-2 text-zinc-500', className)}>
      <Loader2 size={size} className="animate-spin shrink-0" />
      {label && <span className="text-xs">{label}</span>}
    </span>
  );
}

// ── Empty state ───────────────────────────────────────────────────────────────

interface EmptyStateProps {
  title: string;
  sub?: string;
  className?: string;
}

export function EmptyState({ title, sub, className }: EmptyStateProps) {
  return (
    <div className={clsx(
      'flex flex-col items-center justify-center py-16 text-center gap-1.5',
      className,
    )}>
      {/* Minimal geometric illustration */}
      <div className="w-8 h-8 rounded-lg border-2 border-zinc-800 border-dashed mb-2 opacity-50" />
      <p className="text-sm font-medium text-zinc-500">{title}</p>
      {sub && <p className="text-[11px] text-zinc-600 max-w-xs">{sub}</p>}
    </div>
  );
}

// ── Page-level loading wrapper ────────────────────────────────────────────────

interface PageLoadingProps {
  label?: string;
}

export function PageLoading({ label = 'Loading…' }: PageLoadingProps) {
  return (
    <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-600 text-xs">
      <Loader2 size={14} className="animate-spin" />
      {label}
    </div>
  );
}
