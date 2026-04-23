import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Shield,
  RefreshCw,
  ChevronDown,
  ChevronRight,
  ChevronLeft,
  ChevronsLeft,
} from "lucide-react";
import { clsx } from "clsx";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";
import type { AuditOutcome } from "../lib/types";
import { ds } from "../lib/design-system";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
}

function shortId(id: string): string {
  return id.length > 22 ? `${id.slice(0, 10)}\u2026${id.slice(-7)}` : id;
}

// ── Expandable details cell ───────────────────────────────────────────────────

function DetailsCell({ metadata }: { metadata: Record<string, unknown> }) {
  const [open, setOpen] = useState(false);
  const isEmpty = Object.keys(metadata).length === 0;

  if (isEmpty) {
    return <span className="text-gray-300 dark:text-zinc-600 text-[11px]">—</span>;
  }

  return (
    <div>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
      >
        {open
          ? <ChevronDown size={11} className="shrink-0" />
          : <ChevronRight size={11} className="shrink-0" />
        }
        {open ? "hide" : "show"}
      </button>
      {open && (
        <pre className="mt-1.5 text-[10px] font-mono text-gray-500 dark:text-zinc-400 bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800
                        rounded px-2.5 py-1.5 overflow-x-auto max-w-xs whitespace-pre-wrap break-all">
          {JSON.stringify(metadata, null, 2)}
        </pre>
      )}
    </div>
  );
}

// ── Outcome badge ─────────────────────────────────────────────────────────────

function OutcomeBadge({ outcome }: { outcome: AuditOutcome }) {
  return (
    <span className={clsx(
      "inline-flex items-center gap-1 rounded text-[11px] font-medium px-1.5 py-0.5",
      outcome === "success"
        ? "bg-emerald-500/10 text-emerald-400"
        : "bg-red-500/10 text-red-400",
    )}>
      <span className={clsx(
        "w-1.5 h-1.5 rounded-full shrink-0",
        outcome === "success" ? "bg-emerald-500" : "bg-red-500",
      )} />
      {outcome}
    </span>
  );
}

// ── Skeleton ──────────────────────────────────────────────────────────────────

function SkeletonRows() {
  return (
    <div className="divide-y divide-gray-200 dark:divide-zinc-800/40">
      {Array.from({ length: 8 }).map((_, i) => (
        <div key={i} className="flex items-center gap-3 px-4 h-9 animate-pulse">
          <div className="h-2.5 w-28 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-2.5 w-20 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-2.5 w-24 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-2.5 w-16 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-4 w-16 rounded bg-gray-100 dark:bg-zinc-800 ml-auto" />
        </div>
      ))}
    </div>
  );
}

// ── Empty state ───────────────────────────────────────────────────────────────

function EmptyState({ filtered }: { filtered: boolean }) {
  return (
    <div className="flex flex-col items-center justify-center py-20 gap-2 text-gray-300 dark:text-zinc-600">
      <Shield size={28} className="text-gray-300 dark:text-zinc-600" />
      <p className="text-[13px]">
        {filtered ? "No entries match this filter" : "No audit log entries yet"}
      </p>
      {!filtered && (
        <p className="text-[11px] text-gray-300 dark:text-zinc-600">
          Entries appear when actions like approvals, credential changes, or task cancellations occur.
        </p>
      )}
    </div>
  );
}

// ── Time-range + page-size controls ───────────────────────────────────────────

type TimeRange = "1h" | "24h" | "7d" | "30d" | "all";

const TIME_RANGE_MS: Record<Exclude<TimeRange, "all">, number> = {
  "1h":       60 * 60 * 1000,
  "24h":  24 * 60 * 60 * 1000,
  "7d":    7 * 24 * 60 * 60 * 1000,
  "30d":  30 * 24 * 60 * 60 * 1000,
};

const PAGE_SIZES = [50, 100, 250, 500] as const;
type PageSize = typeof PAGE_SIZES[number];

// ── Main page ─────────────────────────────────────────────────────────────────

export function AuditLogPage() {
  const [actionFilter, setActionFilter]   = useState<string>("all");
  const [outcomeFilter, setOutcomeFilter] = useState<AuditOutcome | "all">("all");
  const [pageSize,  setPageSize]  = useState<PageSize>(100);
  const [timeRange, setTimeRange] = useState<TimeRange>("24h");
  // Cursor stack for prev/next — each element is the `before_ms` that
  // returned the page. Empty stack = newest page.
  const [cursorStack, setCursorStack] = useState<number[]>([]);
  const beforeMs = cursorStack[cursorStack.length - 1];

  // Compute since_ms from the selected range (10s bucket for query stability).
  const sinceMs = timeRange === "all"
    ? undefined
    : Math.floor((Date.now() - TIME_RANGE_MS[timeRange]) / 10_000) * 10_000;

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["audit-log", pageSize, timeRange, beforeMs ?? null],
    queryFn:  () => defaultApi.getAuditLog({
      limit:     pageSize,
      since_ms:  sinceMs,
      before_ms: beforeMs,
    }),
    refetchInterval: cursorStack.length === 0 ? 30_000 : false,
  });

  const entries = data?.items ?? [];
  const hasMore = Boolean(data?.has_more);
  const atNewest = cursorStack.length === 0;

  // Reset paging when filters that change the result set flip.
  function resetPaging() {
    setCursorStack([]);
  }

  function goOlder() {
    const last = entries[entries.length - 1];
    if (last) setCursorStack(stack => [...stack, last.occurred_at_ms]);
  }

  function goNewer() {
    setCursorStack(stack => stack.slice(0, -1));
  }

  function goNewest() {
    setCursorStack([]);
  }

  // Derive unique action names for the filter dropdown.
  const allActions = [...new Set(entries.map((e) => e.action))].sort();

  const filtered = entries.filter((e) => {
    if (actionFilter  !== "all" && e.action  !== actionFilter)  return false;
    if (outcomeFilter !== "all" && e.outcome !== outcomeFilter)  return false;
    return true;
  });

  if (isError) {
    return <ErrorFallback error={error} resource="audit log" onRetry={() => void refetch()} />;
  }

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className={clsx(ds.toolbar.base, "h-11", ds.surface.elevated)}>
        <Shield size={13} className="text-indigo-400 shrink-0" />
        <span className={ds.toolbar.title}>
          Audit Log
          {!isLoading && (
            <span className={ds.toolbar.count}>
              {filtered.length}
              {(actionFilter !== "all" || outcomeFilter !== "all")
                ? ` / ${entries.length} total`
                : ""}
            </span>
          )}
        </span>

        {/* Action filter */}
        <select
          value={actionFilter}
          onChange={(e) => setActionFilter(e.target.value)}
          className={ds.input.select}
        >
          <option value="all">All actions</option>
          {allActions.map((a) => (
            <option key={a} value={a}>{a}</option>
          ))}
        </select>

        {/* Outcome filter */}
        <select
          value={outcomeFilter}
          onChange={(e) => setOutcomeFilter(e.target.value as AuditOutcome | "all")}
          className={ds.input.select}
        >
          <option value="all">All outcomes</option>
          <option value="success">Success</option>
          <option value="failure">Failure</option>
        </select>

        {/* Time-range filter */}
        <select
          value={timeRange}
          onChange={(e) => { setTimeRange(e.target.value as TimeRange); resetPaging(); }}
          aria-label="Time range"
          className={ds.input.select}
        >
          <option value="1h">Last hour</option>
          <option value="24h">Last 24h</option>
          <option value="7d">Last 7 days</option>
          <option value="30d">Last 30 days</option>
          <option value="all">All time</option>
        </select>

        {/* Page-size dropdown */}
        <select
          value={pageSize}
          onChange={(e) => { setPageSize(Number(e.target.value) as PageSize); resetPaging(); }}
          aria-label="Page size"
          className={ds.input.select}
        >
          {PAGE_SIZES.map((n) => (
            <option key={n} value={n}>{n}/page</option>
          ))}
        </select>

        <button
          onClick={() => void refetch()}
          disabled={isFetching}
          className={clsx(ds.btn.secondary, "ml-auto")}
        >
          <RefreshCw size={11} className={clsx(isFetching && "animate-spin")} />
          Refresh
        </button>
      </div>

      {/* Table */}
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <SkeletonRows />
        ) : filtered.length === 0 ? (
          <EmptyState filtered={actionFilter !== "all" || outcomeFilter !== "all"} />
        ) : (
          <table className="min-w-full">
            <thead className={clsx("sticky top-0 z-10", ds.table.headBg)}>
              <tr>
                {["Timestamp", "Actor", "Action", "Resource Type", "Resource ID", "Outcome", "Details"].map((label) => (
                  <th key={label} className={ds.table.th}>
                    {label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {filtered.map((entry, idx) => (
                <tr
                  key={entry.entry_id}
                  className={clsx(
                    ds.table.rowBorder, ds.table.rowHover, "h-9",
                    idx % 2 === 0 ? ds.table.rowEven : ds.table.rowOdd,
                  )}
                >
                  <td className={clsx(ds.table.td, "text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap font-mono")}>
                    {fmtTime(entry.occurred_at_ms)}
                  </td>
                  <td className={clsx(ds.table.td, "font-mono text-[12px] text-gray-700 dark:text-zinc-300 whitespace-nowrap")}>
                    {entry.actor_id}
                  </td>
                  <td className={clsx(ds.table.td, "text-[12px] text-gray-500 dark:text-zinc-400 whitespace-nowrap")}>
                    {entry.action}
                  </td>
                  <td className={clsx(ds.table.td, "text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap")}>
                    {entry.resource_type || <span className="text-gray-300 dark:text-zinc-600">—</span>}
                  </td>
                  <td className={clsx(ds.table.td, "font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap")}>
                    {entry.resource_id ? shortId(entry.resource_id) : <span className="text-gray-300 dark:text-zinc-600">—</span>}
                  </td>
                  <td className={clsx(ds.table.td, "whitespace-nowrap")}>
                    <OutcomeBadge outcome={entry.outcome} />
                  </td>
                  <td className={clsx(ds.table.td)}>
                    <DetailsCell metadata={entry.metadata ?? {}} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* Pagination footer */}
      <div className="flex items-center gap-2 px-3 h-9 border-t border-gray-200 dark:border-zinc-800 shrink-0
                      bg-gray-50 dark:bg-zinc-900 text-[11px] text-gray-500 dark:text-zinc-400 tabular-nums">
        <span>
          {atNewest ? "Newest page" : `Page ${cursorStack.length + 1}`}
          {entries.length > 0 && ` • ${entries.length} entries`}
        </span>
        <div className="ml-auto flex items-center gap-1">
          <button
            onClick={goNewest}
            disabled={atNewest || isFetching}
            aria-label="Jump to newest"
            className={clsx(
              "flex items-center gap-1 h-6 px-2 rounded border transition-colors",
              atNewest
                ? "text-gray-300 dark:text-zinc-700 border-gray-100 dark:border-zinc-800 cursor-not-allowed"
                : "text-gray-500 dark:text-zinc-400 border-gray-200 dark:border-zinc-800 hover:bg-gray-100 dark:hover:bg-zinc-800",
            )}
          >
            <ChevronsLeft size={12} />
            Newest
          </button>
          <button
            onClick={goNewer}
            disabled={atNewest || isFetching}
            className={clsx(
              "flex items-center gap-1 h-6 px-2 rounded border transition-colors",
              atNewest
                ? "text-gray-300 dark:text-zinc-700 border-gray-100 dark:border-zinc-800 cursor-not-allowed"
                : "text-gray-500 dark:text-zinc-400 border-gray-200 dark:border-zinc-800 hover:bg-gray-100 dark:hover:bg-zinc-800",
            )}
          >
            <ChevronLeft size={12} />
            Newer
          </button>
          <button
            onClick={goOlder}
            disabled={!hasMore || entries.length === 0 || isFetching}
            className={clsx(
              "flex items-center gap-1 h-6 px-2 rounded border transition-colors",
              !hasMore || entries.length === 0
                ? "text-gray-300 dark:text-zinc-700 border-gray-100 dark:border-zinc-800 cursor-not-allowed"
                : "text-gray-500 dark:text-zinc-400 border-gray-200 dark:border-zinc-800 hover:bg-gray-100 dark:hover:bg-zinc-800",
            )}
          >
            Older
            <ChevronRight size={12} />
          </button>
        </div>
      </div>
    </div>
  );
}

export default AuditLogPage;
