import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Shield,
  RefreshCw,
  ChevronDown,
  ChevronRight,
} from "lucide-react";
import { clsx } from "clsx";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";
import type { AuditOutcome } from "../lib/types";

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

// ── Main page ─────────────────────────────────────────────────────────────────

export function AuditLogPage() {
  const [actionFilter, setActionFilter]   = useState<string>("all");
  const [outcomeFilter, setOutcomeFilter] = useState<AuditOutcome | "all">("all");

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["audit-log"],
    queryFn:  () => defaultApi.getAuditLog(200),
    refetchInterval: 30_000,
  });

  const entries = data?.items ?? [];

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
      <div className="flex items-center gap-3 px-4 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
        <Shield size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Audit Log
          {!isLoading && (
            <span className="ml-2 text-[11px] text-gray-400 dark:text-zinc-600 font-normal">
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
          className="rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-gray-500 dark:text-zinc-400 text-[12px]
                     px-2 py-1 focus:outline-none focus:border-indigo-500"
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
          className="rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-gray-500 dark:text-zinc-400 text-[12px]
                     px-2 py-1 focus:outline-none focus:border-indigo-500"
        >
          <option value="all">All outcomes</option>
          <option value="success">Success</option>
          <option value="failure">Failure</option>
        </select>

        <button
          onClick={() => void refetch()}
          disabled={isFetching}
          className="ml-auto flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900
                     text-gray-400 dark:text-zinc-500 text-[12px] px-2.5 py-1 hover:text-gray-800 dark:hover:text-zinc-200 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800
                     disabled:opacity-40 transition-colors"
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
            <thead className="sticky top-0 z-10 bg-white dark:bg-zinc-950">
              <tr className="border-b border-gray-200 dark:border-zinc-800">
                {[
                  { label: "Timestamp",     cls: "text-left"  },
                  { label: "Actor",         cls: "text-left"  },
                  { label: "Action",        cls: "text-left"  },
                  { label: "Resource Type", cls: "text-left"  },
                  { label: "Resource ID",   cls: "text-left"  },
                  { label: "Outcome",       cls: "text-left"  },
                  { label: "Details",       cls: "text-left"  },
                ].map(({ label, cls }) => (
                  <th
                    key={label}
                    className={clsx(
                      "px-4 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap",
                      cls,
                    )}
                  >
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
                    "border-b border-gray-200/40 dark:border-zinc-800/40 h-9 transition-colors hover:bg-gray-50/50 dark:bg-zinc-900/50",
                    idx % 2 !== 0 && "bg-gray-50/20 dark:bg-zinc-900/20",
                  )}
                >
                  <td className="px-4 py-0 text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap font-mono">
                    {fmtTime(entry.occurred_at_ms)}
                  </td>
                  <td className="px-4 py-0 font-mono text-[12px] text-gray-700 dark:text-zinc-300 whitespace-nowrap">
                    {entry.actor_id}
                  </td>
                  <td className="px-4 py-0 text-[12px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">
                    {entry.action}
                  </td>
                  <td className="px-4 py-0 text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap">
                    {entry.resource_type || <span className="text-gray-300 dark:text-zinc-600">—</span>}
                  </td>
                  <td className="px-4 py-0 font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap">
                    {entry.resource_id ? shortId(entry.resource_id) : <span className="text-gray-300 dark:text-zinc-600">—</span>}
                  </td>
                  <td className="px-4 py-0 whitespace-nowrap">
                    <OutcomeBadge outcome={entry.outcome} />
                  </td>
                  <td className="px-4 py-1.5">
                    <DetailsCell metadata={entry.metadata ?? {}} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

export default AuditLogPage;
