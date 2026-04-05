import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  FlaskConical,
  RefreshCw,
  Plus,
  CheckCircle2,
  XCircle,
  Clock,
  Loader2,
} from "lucide-react";
import { clsx } from "clsx";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";
import type { EvalRunRecord, EvalRunStatus } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
}

function fmtDuration(startMs: number, endMs: number | null): string {
  if (!endMs) return "—";
  const d = endMs - startMs;
  if (d < 1000)  return `${d}ms`;
  if (d < 60000) return `${(d / 1000).toFixed(1)}s`;
  return `${Math.floor(d / 60000)}m ${Math.floor((d % 60000) / 1000)}s`;
}

function shortId(id: string): string {
  return id.length > 20 ? `${id.slice(0, 10)}\u2026${id.slice(-6)}` : id;
}

/** Derive a status from EvalRunRecord fields. */
function deriveStatus(r: EvalRunRecord): EvalRunStatus {
  if (r.error_message)     return "failed";
  if (r.completed_at) {
    return r.success === false ? "failed" : "completed";
  }
  if (r.started_at && !r.completed_at) return "running";
  return "pending";
}

// ── Status badge ──────────────────────────────────────────────────────────────

const STATUS_STYLES: Record<EvalRunStatus, string> = {
  pending:   "bg-zinc-800/80 text-zinc-400",
  running:   "bg-indigo-500/10 text-indigo-400",
  completed: "bg-emerald-500/10 text-emerald-400",
  failed:    "bg-red-500/10 text-red-400",
  canceled:  "bg-zinc-800/60 text-zinc-500",
};
const STATUS_DOT: Record<EvalRunStatus, string> = {
  pending:   "bg-zinc-500",
  running:   "bg-indigo-400 animate-pulse",
  completed: "bg-emerald-500",
  failed:    "bg-red-500",
  canceled:  "bg-zinc-600",
};

function StatusBadge({ status }: { status: EvalRunStatus }) {
  return (
    <span className={clsx(
      "inline-flex items-center gap-1.5 rounded text-[11px] font-medium px-1.5 py-0.5",
      STATUS_STYLES[status],
    )}>
      <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0", STATUS_DOT[status])} />
      {status.charAt(0).toUpperCase() + status.slice(1)}
    </span>
  );
}

// ── Stat card ─────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub, accent = false }: {
  label: string; value: string | number; sub?: string; accent?: boolean;
}) {
  return (
    <div className="bg-zinc-900 border border-zinc-800 border-l-2 border-l-indigo-500 rounded-lg p-4">
      <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2">{label}</p>
      <p className={clsx("text-xl font-semibold tabular-nums", accent ? "text-indigo-400" : "text-zinc-100")}>
        {value}
      </p>
      {sub && <p className="mt-1 text-[11px] text-zinc-600">{sub}</p>}
    </div>
  );
}

// ── Empty state ───────────────────────────────────────────────────────────────

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center py-20 gap-3 text-center">
      <div className="w-10 h-10 rounded-full bg-zinc-900 border border-zinc-800 flex items-center justify-center">
        <FlaskConical size={18} className="text-zinc-600" />
      </div>
      <div>
        <p className="text-[13px] font-medium text-zinc-400">No eval runs yet</p>
        <p className="text-[11px] text-zinc-600 mt-1 max-w-xs">
          Use <code className="text-zinc-500 bg-zinc-800 rounded px-1">POST /v1/evals/runs</code> to
          start evaluating LLM outputs against prompts and rubrics.
        </p>
      </div>
    </div>
  );
}

// ── Skeleton ──────────────────────────────────────────────────────────────────

function SkeletonRows() {
  return (
    <div className="divide-y divide-zinc-800/40">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className="flex items-center gap-4 px-4 h-9 animate-pulse">
          <div className="h-2.5 w-32 rounded bg-zinc-800" />
          <div className="h-2.5 w-28 rounded bg-zinc-800" />
          <div className="h-2.5 w-24 rounded bg-zinc-800" />
          <div className="h-4 w-20 rounded bg-zinc-800" />
          <div className="ml-auto h-2.5 w-20 rounded bg-zinc-800" />
        </div>
      ))}
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function EvalsPage() {
  const [statusFilter, setStatusFilter] = useState<EvalRunStatus | "all">("all");

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["evals"],
    queryFn:  () => defaultApi.getEvalRuns(200),
    refetchInterval: 20_000,
  });

  const runs = data?.items ?? [];

  // Attach derived status to each record.
  const annotated = runs.map((r) => ({ ...r, _status: deriveStatus(r) }));

  const filtered = statusFilter === "all"
    ? annotated
    : annotated.filter((r) => r._status === statusFilter);

  // ── Summary stats ─────────────────────────────────────────────────────────
  const total      = runs.length;
  const completed  = annotated.filter((r) => r._status === "completed").length;
  const failed     = annotated.filter((r) => r._status === "failed").length;
  const passRate   = total > 0 ? Math.round((completed / total) * 100) : 0;
  const evalTypes  = [...new Set(runs.map((r) => r.evaluator_type))].length;

  if (isError) {
    return <ErrorFallback error={error} resource="eval runs" onRetry={() => void refetch()} />;
  }

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-11 border-b border-zinc-800 shrink-0 bg-zinc-950">
        <FlaskConical size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[13px] font-medium text-zinc-200">
          Evaluations
          {!isLoading && (
            <span className="ml-2 text-[11px] text-zinc-600 font-normal">
              {filtered.length}{statusFilter !== "all" ? ` / ${total} total` : ""}
            </span>
          )}
        </span>

        {/* Status filter */}
        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as EvalRunStatus | "all")}
          className="rounded border border-zinc-800 bg-zinc-900 text-zinc-400 text-[12px]
                     px-2 py-1 focus:outline-none focus:border-indigo-500"
        >
          <option value="all">All statuses</option>
          <option value="completed">Completed</option>
          <option value="running">Running</option>
          <option value="failed">Failed</option>
          <option value="pending">Pending</option>
        </select>

        {/* New Eval Run — placeholder, wires to the API form */}
        <button
          className="ml-auto flex items-center gap-1.5 rounded bg-indigo-600 hover:bg-indigo-500
                     text-white text-[12px] font-medium px-3 py-1.5 transition-colors"
          title="POST /v1/evals/runs to create an eval run programmatically"
        >
          <Plus size={12} />
          New Eval Run
        </button>

        <button
          onClick={() => void refetch()}
          disabled={isFetching}
          className="flex items-center gap-1.5 rounded border border-zinc-800 bg-zinc-900
                     text-zinc-500 text-[12px] px-2.5 py-1 hover:text-zinc-200 hover:bg-zinc-800
                     disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={clsx(isFetching && "animate-spin")} />
          Refresh
        </button>
      </div>

      {/* Stat cards */}
      {!isLoading && total > 0 && (
        <div className="grid grid-cols-2 gap-3 px-4 py-4 border-b border-zinc-800 lg:grid-cols-4 shrink-0">
          <StatCard label="Total Eval Runs"  value={total}         sub="all time" />
          <StatCard label="Pass Rate"        value={`${passRate}%`} sub={`${completed} passed`} accent />
          <StatCard label="Failed"           value={failed}        sub={failed > 0 ? "needs attention" : "none"} />
          <StatCard label="Evaluator Types"  value={evalTypes}     sub="distinct types" />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <SkeletonRows />
        ) : filtered.length === 0 ? (
          <EmptyState />
        ) : (
          <table className="min-w-full">
            <thead className="sticky top-0 z-10 bg-zinc-950">
              <tr className="border-b border-zinc-800">
                {[
                  { label: "Run ID",         cls: "text-left"  },
                  { label: "Eval Suite",     cls: "text-left"  },
                  { label: "Subject",        cls: "text-left"  },
                  { label: "Status",         cls: "text-left"  },
                  { label: "Duration",       cls: "text-right" },
                  { label: "Created",        cls: "text-right" },
                ].map(({ label, cls }) => (
                  <th key={label} className={clsx(
                    "px-4 py-2 text-[11px] font-medium text-zinc-500 uppercase tracking-wider whitespace-nowrap",
                    cls,
                  )}>
                    {label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {filtered.map((run, idx) => (
                <tr
                  key={run.eval_run_id}
                  className={clsx(
                    "border-b border-zinc-800/40 h-9 hover:bg-zinc-900/50 transition-colors",
                    idx % 2 !== 0 && "bg-zinc-900/20",
                  )}
                >
                  <td className="px-4 py-0 font-mono text-[12px] text-zinc-300 whitespace-nowrap">
                    {shortId(run.eval_run_id)}
                  </td>
                  <td className="px-4 py-0 text-[12px] text-zinc-400 whitespace-nowrap">
                    {run.evaluator_type}
                  </td>
                  <td className="px-4 py-0 text-[11px] text-zinc-500 whitespace-nowrap">
                    {run.subject_kind}
                  </td>
                  <td className="px-4 py-0 whitespace-nowrap">
                    <div className="flex items-center gap-2">
                      <StatusBadge status={run._status} />
                      {run._status === "completed" && run.success === false && (
                        <XCircle size={11} className="text-red-400 shrink-0" />
                      )}
                      {run._status === "completed" && run.success === true && (
                        <CheckCircle2 size={11} className="text-emerald-500 shrink-0" />
                      )}
                      {run._status === "running" && (
                        <Loader2 size={11} className="text-indigo-400 animate-spin shrink-0" />
                      )}
                    </div>
                  </td>
                  <td className="px-4 py-0 text-right">
                    <span className="flex items-center justify-end gap-1 text-[11px] text-zinc-500">
                      <Clock size={10} className="text-zinc-700" />
                      {fmtDuration(run.started_at, run.completed_at)}
                    </span>
                  </td>
                  <td className="px-4 py-0 text-[11px] text-zinc-600 whitespace-nowrap text-right font-mono">
                    {fmtTime(run.started_at)}
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

export default EvalsPage;
