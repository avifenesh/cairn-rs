import { useState, useMemo } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  FlaskConical,
  RefreshCw,
  Plus,
  CheckCircle2,
  XCircle,
  Clock,
  Loader2,
  GitCompare,
  Trophy,
  X,
} from "lucide-react";
import { clsx } from "clsx";
import { ErrorFallback } from "../components/ErrorFallback";
import { MiniChart } from "../components/MiniChart";
import { BarChart } from "../components/BarChart";
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
  pending:   "bg-gray-100/80 dark:bg-zinc-800/80 text-gray-500 dark:text-zinc-400",
  running:   "bg-indigo-500/10 text-indigo-400",
  completed: "bg-emerald-500/10 text-emerald-400",
  failed:    "bg-red-500/10 text-red-400",
  canceled:  "bg-gray-100/60 dark:bg-zinc-800/60 text-gray-400 dark:text-zinc-500",
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
    <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 border-l-indigo-500 rounded-lg p-4">
      <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">{label}</p>
      <p className={clsx("text-xl font-semibold tabular-nums", accent ? "text-indigo-400" : "text-gray-900 dark:text-zinc-100")}>
        {value}
      </p>
      {sub && <p className="mt-1 text-[11px] text-gray-400 dark:text-zinc-600">{sub}</p>}
    </div>
  );
}

// ── Score trend sparklines ────────────────────────────────────────────────────

/**
 * For each evaluator type, compute a time-ordered array of 0/1 (fail/pass)
 * values and render a MiniChart showing the trend.
 */
function ScoreTrends({ runs }: { runs: (EvalRunRecord & { _status: EvalRunStatus })[] }) {
  const byType = useMemo(() => {
    const map: Record<string, number[]> = {};
    const sorted = [...runs].sort((a, b) => a.started_at - b.started_at);
    for (const r of sorted) {
      if (r._status !== "completed" && r._status !== "failed") continue;
      const score = r.success === true ? 1 : 0;
      map[r.evaluator_type] = [...(map[r.evaluator_type] ?? []), score];
    }
    return Object.entries(map)
      .filter(([, scores]) => scores.length >= 2)
      .sort(([, a], [, b]) => b.length - a.length)
      .slice(0, 6);
  }, [runs]);

  if (byType.length === 0) return null;

  return (
    <div className="px-4 pb-4 border-b border-gray-200 dark:border-zinc-800">
      <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider pt-3 pb-2">
        Score Trend by Evaluator
      </p>
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
        {byType.map(([type, scores]) => {
          const passRate = scores.filter(Boolean).length / scores.length;
          const color = passRate >= 0.8 ? "#10b981" : passRate >= 0.5 ? "#f59e0b" : "#ef4444";
          return (
            <div key={type} className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg p-3">
              <div className="flex items-center justify-between mb-2">
                <p className="text-[11px] font-mono text-gray-500 dark:text-zinc-400 truncate max-w-[120px]" title={type}>
                  {type}
                </p>
                <span className={clsx(
                  "text-[11px] font-semibold tabular-nums",
                  passRate >= 0.8 ? "text-emerald-400" :
                  passRate >= 0.5 ? "text-amber-400"   : "text-red-400",
                )}>
                  {(passRate * 100).toFixed(0)}%
                </span>
              </div>
              <MiniChart data={scores} height={28} color={color} className="w-full" />
              <p className="text-[10px] text-gray-300 dark:text-zinc-700 mt-1">{scores.length} runs</p>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Evaluator ranking ─────────────────────────────────────────────────────────

function EvaluatorRanking({ runs }: { runs: (EvalRunRecord & { _status: EvalRunStatus })[] }) {
  const items = useMemo(() => {
    const map: Record<string, { pass: number; fail: number }> = {};
    for (const r of runs) {
      if (r._status !== "completed" && r._status !== "failed") continue;
      const entry = map[r.evaluator_type] ?? { pass: 0, fail: 0 };
      if (r.success === true) entry.pass++;
      else                    entry.fail++;
      map[r.evaluator_type] = entry;
    }
    return Object.entries(map)
      .map(([label, { pass, fail }]) => {
        const total = pass + fail;
        const rate  = total > 0 ? pass / total : 0;
        const color = rate >= 0.8 ? "#10b981" : rate >= 0.5 ? "#f59e0b" : "#ef4444";
        return { label, value: pass, total, rate, color };
      })
      .sort((a, b) => b.rate - a.rate);
  }, [runs]);

  if (items.length === 0) return null;

  return (
    <div className="grid grid-cols-1 gap-4 px-4 pb-4 border-b border-gray-200 dark:border-zinc-800 lg:grid-cols-2">
      {/* Best performers */}
      <div>
        <div className="flex items-center gap-1.5 pt-3 pb-2">
          <Trophy size={12} className="text-amber-400" />
          <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Evaluator Rankings</p>
        </div>
        <BarChart
          items={items.map((i) => ({
            label:    i.label,
            value:    Math.round(i.rate * 100),
            color:    i.color,
            sublabel: `(${i.value}/${i.total})`,
          }))}
          formatValue={(v) => `${v}%`}
          maxItems={6}
          barHeight={6}
          rowGap={8}
        />
      </div>

      {/* Best / worst summary */}
      <div className="flex flex-col gap-3 pt-3">
        {items.length > 0 && (
          <div className="rounded-lg border border-emerald-500/20 bg-emerald-950/10 px-4 py-3">
            <div className="flex items-center gap-1.5 mb-1">
              <CheckCircle2 size={12} className="text-emerald-400" />
              <span className="text-[10px] text-emerald-500 uppercase tracking-wider font-medium">Top performer</span>
            </div>
            <p className="text-[13px] font-mono text-emerald-300 truncate">{items[0].label}</p>
            <p className="text-[11px] text-emerald-600 mt-0.5">
              {(items[0].rate * 100).toFixed(0)}% pass rate · {items[0].value}/{items[0].total} runs
            </p>
          </div>
        )}
        {items.length > 1 && (
          <div className="rounded-lg border border-red-500/20 bg-red-950/10 px-4 py-3">
            <div className="flex items-center gap-1.5 mb-1">
              <XCircle size={12} className="text-red-400" />
              <span className="text-[10px] text-red-500 uppercase tracking-wider font-medium">Needs attention</span>
            </div>
            <p className="text-[13px] font-mono text-red-300 truncate">{items[items.length - 1].label}</p>
            <p className="text-[11px] text-red-600 mt-0.5">
              {(items[items.length - 1].rate * 100).toFixed(0)}% pass rate · {items[items.length - 1].value}/{items[items.length - 1].total} runs
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Empty + skeleton ──────────────────────────────────────────────────────────

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center py-20 gap-3 text-center">
      <div className="w-10 h-10 rounded-full bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 flex items-center justify-center">
        <FlaskConical size={18} className="text-gray-400 dark:text-zinc-600" />
      </div>
      <div>
        <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No eval runs yet</p>
        <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-1 max-w-xs">
          Click <strong className="text-gray-500 dark:text-zinc-400">New Eval Run</strong> above to create one,
          or eval runs will appear here automatically after orchestration completes.
        </p>
      </div>
    </div>
  );
}

function SkeletonRows() {
  return (
    <div className="divide-y divide-gray-200 dark:divide-zinc-800/40">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className="flex items-center gap-4 px-4 h-9 animate-pulse">
          <div className="h-3.5 w-3.5 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-2.5 w-32 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-2.5 w-28 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-2.5 w-24 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="h-4 w-20 rounded bg-gray-100 dark:bg-zinc-800" />
          <div className="ml-auto h-2.5 w-20 rounded bg-gray-100 dark:bg-zinc-800" />
        </div>
      ))}
    </div>
  );
}

// ── Compare selection banner ──────────────────────────────────────────────────

function CompareBanner({
  selected,
  onClear,
  onCompare,
}: {
  selected: string[];
  onClear: () => void;
  onCompare: () => void;
}) {
  const ready = selected.length === 2;
  return (
    <div className={clsx(
      "flex items-center gap-3 px-4 py-2 border-b border-gray-200 dark:border-zinc-800 shrink-0 transition-colors",
      ready ? "bg-indigo-950/30 border-indigo-800/40" : "bg-gray-50/60 dark:bg-zinc-900/60",
    )}>
      <span className="text-[12px] text-gray-500 dark:text-zinc-400">
        <span className={clsx("font-semibold", ready ? "text-indigo-300" : "text-gray-700 dark:text-zinc-300")}>
          {selected.length}
        </span>
        {" / 2 runs selected for comparison"}
      </span>
      {selected.length > 0 && (
        <button
          onClick={onClear}
          className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
        >
          <X size={10} /> Clear
        </button>
      )}
      <button
        onClick={onCompare}
        disabled={!ready}
        className={clsx(
          "ml-auto flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium transition-colors",
          ready
            ? "bg-indigo-600 hover:bg-indigo-500 text-white"
            : "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-600 cursor-not-allowed",
        )}
      >
        <GitCompare size={12} />
        Compare
      </button>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

const EVALUATOR_TYPES = ["accuracy", "relevance", "coherence", "safety", "custom"] as const;
const SUBJECT_KINDS   = ["prompt_release", "agent_template", "run_output", "custom"] as const;

export function EvalsPage() {
  const [statusFilter, setStatusFilter] = useState<EvalRunStatus | "all">("all");
  const [selected, setSelected]         = useState<Set<string>>(new Set());
  const [showNewForm, setShowNewForm]   = useState(false);
  const [newEvalType, setNewEvalType]   = useState<string>(EVALUATOR_TYPES[0]);
  const [newSubject, setNewSubject]     = useState<string>(SUBJECT_KINDS[0]);
  const qc = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["evals"],
    queryFn:  () => defaultApi.getEvalRuns(200),
    refetchInterval: 20_000,
  });

  const createEval = useMutation({
    mutationFn: () => {
      const id = `eval_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`;
      return defaultApi.createEvalRun({
        eval_run_id: id,
        subject_kind: newSubject,
        evaluator_type: newEvalType,
      });
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["evals"] });
      setShowNewForm(false);
    },
  });

  const runs = data?.items ?? [];
  const annotated = useMemo(
    () => runs.map((r) => ({ ...r, _status: deriveStatus(r) })),
    [runs],
  );

  const filtered = statusFilter === "all"
    ? annotated
    : annotated.filter((r) => r._status === statusFilter);

  // Summary stats
  const total     = runs.length;
  const completed = annotated.filter((r) => r._status === "completed").length;
  const failed    = annotated.filter((r) => r._status === "failed").length;
  const passRate  = total > 0 ? Math.round((completed / total) * 100) : 0;
  const evalTypes = [...new Set(runs.map((r) => r.evaluator_type))].length;

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else if (next.size < 2) {
        next.add(id);
      }
      return next;
    });
  }

  function handleCompare() {
    const [left, right] = Array.from(selected);
    if (left && right) {
      window.location.hash = `eval-compare/${left}/${right}`;
    }
  }

  const selectedArr = Array.from(selected);
  const showBanner  = selectedArr.length > 0;

  if (isError) {
    return <ErrorFallback error={error} resource="eval runs" onRetry={() => void refetch()} />;
  }

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
        <FlaskConical size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Evaluations
          {!isLoading && (
            <span className="ml-2 text-[11px] text-gray-400 dark:text-zinc-600 font-normal">
              {filtered.length}{statusFilter !== "all" ? ` / ${total} total` : ""}
            </span>
          )}
        </span>

        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as EvalRunStatus | "all")}
          className="rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-gray-500 dark:text-zinc-400 text-[12px]
                     px-2 py-1 focus:outline-none focus:border-indigo-500"
        >
          <option value="all">All statuses</option>
          <option value="completed">Completed</option>
          <option value="running">Running</option>
          <option value="failed">Failed</option>
          <option value="pending">Pending</option>
        </select>

        <div className="ml-auto relative">
          <button
            onClick={() => setShowNewForm(v => !v)}
            className="flex items-center gap-1.5 rounded bg-indigo-600 hover:bg-indigo-500
                       text-white text-[12px] font-medium px-3 py-1.5 transition-colors"
            title="Create a new eval run"
          >
            <Plus size={12} />
            New Eval Run
          </button>

          {showNewForm && (
            <>
              <div className="fixed inset-0 z-30" onClick={() => setShowNewForm(false)} />
              <div className="absolute right-0 top-full mt-1 z-40 w-64 bg-white dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg shadow-xl p-3 space-y-3">
                <label className="block">
                  <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Evaluator Type</span>
                  <select
                    value={newEvalType}
                    onChange={e => setNewEvalType(e.target.value)}
                    className="mt-1 w-full rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-950 text-gray-700 dark:text-zinc-300 text-[12px] px-2 py-1.5 focus:outline-none focus:border-indigo-500"
                  >
                    {EVALUATOR_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
                  </select>
                </label>
                <label className="block">
                  <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Subject Kind</span>
                  <select
                    value={newSubject}
                    onChange={e => setNewSubject(e.target.value)}
                    className="mt-1 w-full rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-950 text-gray-700 dark:text-zinc-300 text-[12px] px-2 py-1.5 focus:outline-none focus:border-indigo-500"
                  >
                    {SUBJECT_KINDS.map(s => <option key={s} value={s}>{s}</option>)}
                  </select>
                </label>
                <button
                  onClick={() => createEval.mutate()}
                  disabled={createEval.isPending}
                  className="w-full flex items-center justify-center gap-1.5 rounded bg-indigo-600 hover:bg-indigo-500
                             text-white text-[12px] font-medium px-3 py-1.5 transition-colors disabled:opacity-50"
                >
                  {createEval.isPending ? <Loader2 size={12} className="animate-spin" /> : <Plus size={12} />}
                  {createEval.isPending ? "Creating…" : "Create"}
                </button>
              </div>
            </>
          )}
        </div>

        <button
          onClick={() => void refetch()}
          disabled={isFetching}
          className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900
                     text-gray-400 dark:text-zinc-500 text-[12px] px-2.5 py-1 hover:text-gray-800 dark:hover:text-zinc-200 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800
                     disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={clsx(isFetching && "animate-spin")} />
          Refresh
        </button>
      </div>

      {/* Compare selection banner */}
      {showBanner && (
        <CompareBanner
          selected={selectedArr}
          onClear={() => setSelected(new Set())}
          onCompare={handleCompare}
        />
      )}

      {/* Stat cards */}
      {!isLoading && total > 0 && (
        <div className="grid grid-cols-2 gap-3 px-4 py-4 border-b border-gray-200 dark:border-zinc-800 lg:grid-cols-4 shrink-0">
          <StatCard label="Total Eval Runs"  value={total}          sub="all time" />
          <StatCard label="Pass Rate"        value={`${passRate}%`} sub={`${completed} passed`} accent />
          <StatCard label="Failed"           value={failed}         sub={failed > 0 ? "needs attention" : "none"} />
          <StatCard label="Evaluator Types"  value={evalTypes}      sub="distinct types" />
        </div>
      )}

      {/* Score trend sparklines */}
      {!isLoading && annotated.length >= 2 && (
        <ScoreTrends runs={annotated} />
      )}

      {/* Evaluator rankings */}
      {!isLoading && annotated.length >= 2 && (
        <EvaluatorRanking runs={annotated} />
      )}

      {/* Table hint */}
      {!isLoading && filtered.length > 0 && (
        <div className="px-4 py-1.5 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <p className="text-[10px] text-gray-300 dark:text-zinc-700">
            Select up to 2 runs using the checkboxes, then click <strong className="text-gray-400 dark:text-zinc-600">Compare</strong>.
          </p>
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
            <thead className="sticky top-0 z-10 bg-white dark:bg-zinc-950">
              <tr className="border-b border-gray-200 dark:border-zinc-800">
                <th className="px-4 py-2 text-left w-8">
                  {/* checkbox column header — intentionally blank */}
                </th>
                {[
                  { label: "Run ID",     cls: "text-left"  },
                  { label: "Eval Suite", cls: "text-left"  },
                  { label: "Subject",    cls: "text-left"  },
                  { label: "Status",     cls: "text-left"  },
                  { label: "Duration",   cls: "text-right" },
                  { label: "Created",    cls: "text-right" },
                ].map(({ label, cls }) => (
                  <th key={label} className={clsx(
                    "px-4 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap",
                    cls,
                  )}>
                    {label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {filtered.map((run, idx) => {
                const isSelected = selected.has(run.eval_run_id);
                const isDisabled = selected.size >= 2 && !isSelected;
                return (
                  <tr
                    key={run.eval_run_id}
                    className={clsx(
                      "border-b border-gray-200/40 dark:border-zinc-800/40 h-9 transition-colors",
                      isSelected
                        ? "bg-indigo-950/30"
                        : idx % 2 !== 0
                        ? "bg-gray-50/20 dark:bg-zinc-900/20 hover:bg-gray-50/50 dark:bg-zinc-900/50"
                        : "hover:bg-gray-50/50 dark:bg-zinc-900/50",
                    )}
                  >
                    {/* Checkbox */}
                    <td className="px-4 py-0 w-8">
                      <input
                        type="checkbox"
                        checked={isSelected}
                        disabled={isDisabled}
                        onChange={() => toggleSelect(run.eval_run_id)}
                        className="accent-indigo-500 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                        title={isDisabled ? "Deselect another run first" : "Select for comparison"}
                      />
                    </td>
                    <td className="px-4 py-0 font-mono text-[12px] text-gray-700 dark:text-zinc-300 whitespace-nowrap">
                      {shortId(run.eval_run_id)}
                    </td>
                    <td className="px-4 py-0 text-[12px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">
                      {run.evaluator_type}
                    </td>
                    <td className="px-4 py-0 text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap">
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
                      <span className="flex items-center justify-end gap-1 text-[11px] text-gray-400 dark:text-zinc-500">
                        <Clock size={10} className="text-gray-300 dark:text-zinc-700" />
                        {fmtDuration(run.started_at, run.completed_at)}
                      </span>
                    </td>
                    <td className="px-4 py-0 text-[11px] text-gray-400 dark:text-zinc-600 whitespace-nowrap text-right font-mono">
                      {fmtTime(run.started_at)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

export default EvalsPage;
