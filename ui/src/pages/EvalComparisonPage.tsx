/**
 * EvalComparisonPage — side-by-side comparison of two eval runs.
 *
 * Route: #eval-compare/<leftId>/<rightId>
 * Accessed from EvalsPage by selecting two runs and clicking "Compare".
 */

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeft,
  FlaskConical,
  CheckCircle2,
  XCircle,
  TrendingUp,
  TrendingDown,
  Minus,
  Clock,
  AlertTriangle,
  Loader2,
} from "lucide-react";
import { clsx } from "clsx";
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

function durationMs(r: EvalRunRecord): number | null {
  return r.completed_at ? r.completed_at - r.started_at : null;
}

function deriveStatus(r: EvalRunRecord): EvalRunStatus {
  if (r.error_message)  return "failed";
  if (r.completed_at)   return r.success === false ? "failed" : "completed";
  if (!r.completed_at)  return "running";
  return "pending";
}

/** Numeric "score": 1.0 = passed, 0.0 = failed, 0.5 = unknown/pending */
function numericScore(r: EvalRunRecord): number | null {
  if (r.success === true)  return 1.0;
  if (r.success === false) return 0.0;
  return null; // still running / no data
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

// ── Delta indicator ───────────────────────────────────────────────────────────

/** Show a numeric delta with coloured arrow. positive = green (improvement). */
function Delta({ delta, unit = "", invert = false }: {
  delta: number | null;
  unit?: string;
  /** invert = lower is better (e.g. duration) */
  invert?: boolean;
}) {
  if (delta === null || delta === 0) {
    return <span className="text-[11px] text-gray-400 dark:text-zinc-600 flex items-center gap-0.5"><Minus size={10} /> tie</span>;
  }

  const isGood = invert ? delta < 0 : delta > 0;
  const sign   = delta > 0 ? "+" : "";
  const fmt    = Number.isInteger(delta) ? delta.toFixed(0) : delta.toFixed(2);

  return (
    <span className={clsx(
      "text-[11px] font-medium flex items-center gap-0.5",
      isGood ? "text-emerald-400" : "text-red-400",
    )}>
      {isGood
        ? <TrendingUp  size={11} className="shrink-0" />
        : <TrendingDown size={11} className="shrink-0" />
      }
      {sign}{fmt}{unit}
    </span>
  );
}

// ── Run panel ─────────────────────────────────────────────────────────────────

function RunPanel({
  run,
  side,
  highlight,
}: {
  run: EvalRunRecord;
  side: "left" | "right";
  /** "win" = this side is better, "lose" = other is better, "tie" */
  highlight: "win" | "lose" | "tie";
}) {
  const status = deriveStatus(run);
  const score  = numericScore(run);
  const dur    = durationMs(run);

  const borderColor =
    highlight === "win"  ? "border-emerald-500/40" :
    highlight === "lose" ? "border-red-500/30"      :
                           "border-gray-200 dark:border-zinc-700";

  const headerBg =
    highlight === "win"  ? "bg-emerald-950/30" :
    highlight === "lose" ? "bg-red-950/20"      :
                           "bg-gray-50 dark:bg-zinc-900";

  return (
    <div className={clsx(
      "flex flex-col rounded-xl border overflow-hidden",
      borderColor,
    )}>
      {/* Panel header */}
      <div className={clsx("px-4 py-3 border-b border-gray-200 dark:border-zinc-800", headerBg)}>
        <div className="flex items-center justify-between mb-1">
          <span className="text-[10px] font-semibold uppercase tracking-wider text-gray-400 dark:text-zinc-500">
            {side === "left" ? "← Left" : "Right →"}
          </span>
          {highlight === "win"  && <span className="text-[10px] text-emerald-400 font-medium">Better</span>}
          {highlight === "lose" && <span className="text-[10px] text-red-400 font-medium">Worse</span>}
        </div>
        <p className="text-[13px] font-mono text-gray-800 dark:text-zinc-200 truncate">{run.eval_run_id}</p>
      </div>

      {/* Metrics */}
      <div className="flex-1 p-4 space-y-3">
        {/* Score */}
        <div>
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Outcome</p>
          <div className="flex items-center gap-2">
            {score === 1.0 ? (
              <CheckCircle2 size={20} className="text-emerald-400" />
            ) : score === 0.0 ? (
              <XCircle size={20} className="text-red-400" />
            ) : (
              <Loader2 size={20} className="text-gray-400 dark:text-zinc-500 animate-spin" />
            )}
            <span className={clsx(
              "text-[22px] font-semibold tabular-nums",
              score === 1.0 ? "text-emerald-400" :
              score === 0.0 ? "text-red-400"      : "text-gray-400 dark:text-zinc-500",
            )}>
              {score !== null ? `${(score * 100).toFixed(0)}%` : "—"}
            </span>
          </div>
        </div>

        {/* Status */}
        <div>
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Status</p>
          <StatusBadge status={status} />
        </div>

        {/* Evaluator type */}
        <div>
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Evaluator</p>
          <p className="text-[12px] font-mono text-gray-700 dark:text-zinc-300">{run.evaluator_type}</p>
        </div>

        {/* Subject kind */}
        <div>
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Subject</p>
          <p className="text-[12px] text-gray-500 dark:text-zinc-400">{run.subject_kind}</p>
        </div>

        {/* Duration */}
        <div>
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Duration</p>
          <span className="flex items-center gap-1.5 text-[12px] text-gray-500 dark:text-zinc-400">
            <Clock size={11} className="text-gray-400 dark:text-zinc-600" />
            {fmtDuration(run.started_at, run.completed_at)}
            {dur !== null && (
              <span className="text-[10px] font-mono text-gray-300 dark:text-zinc-700">({dur.toLocaleString()}ms)</span>
            )}
          </span>
        </div>

        {/* Started */}
        <div>
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Started</p>
          <p className="text-[11px] font-mono text-gray-400 dark:text-zinc-500">{fmtTime(run.started_at)}</p>
        </div>

        {/* Error */}
        {run.error_message && (
          <div>
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Error</p>
            <p className="text-[11px] text-red-400 font-mono break-all">{run.error_message}</p>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Comparison table ──────────────────────────────────────────────────────────

interface ComparisonRow {
  dimension:  string;
  leftValue:  string;
  rightValue: string;
  delta:      number | null;
  unit?:      string;
  invert?:    boolean;
  match:      boolean;
}

function ComparisonTable({ rows }: { rows: ComparisonRow[] }) {
  return (
    <div className="rounded-xl border border-gray-200 dark:border-zinc-800 overflow-hidden">
      {/* Header */}
      <div className="grid grid-cols-[1fr_1fr_80px_1fr] gap-0 bg-white dark:bg-zinc-950 border-b border-gray-200 dark:border-zinc-800">
        <div className="px-4 py-2 text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Dimension</div>
        <div className="px-4 py-2 text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider border-l border-gray-200 dark:border-zinc-800">Left</div>
        <div className="px-4 py-2 text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider border-l border-gray-200 dark:border-zinc-800 text-center">Delta</div>
        <div className="px-4 py-2 text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider border-l border-gray-200 dark:border-zinc-800">Right</div>
      </div>

      {rows.map((row, i) => (
        <div
          key={row.dimension}
          className={clsx(
            "grid grid-cols-[1fr_1fr_80px_1fr] border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0",
            i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50",
            !row.match && "bg-amber-950/10",
          )}
        >
          <div className="px-4 py-2.5 text-[12px] text-gray-500 dark:text-zinc-400">{row.dimension}</div>
          <div className={clsx(
            "px-4 py-2.5 text-[12px] font-mono border-l border-gray-200 dark:border-zinc-800",
            row.delta !== null && row.delta > 0 && !row.invert ? "text-emerald-400" :
            row.delta !== null && row.delta < 0 && !row.invert ? "text-red-400"    :
            row.delta !== null && row.delta < 0 &&  row.invert ? "text-emerald-400":
            row.delta !== null && row.delta > 0 &&  row.invert ? "text-red-400"    :
            "text-gray-700 dark:text-zinc-300",
          )}>
            {row.leftValue}
          </div>
          <div className="px-2 py-2.5 border-l border-gray-200 dark:border-zinc-800 flex items-center justify-center">
            <Delta delta={row.delta} unit={row.unit} invert={row.invert} />
          </div>
          <div className={clsx(
            "px-4 py-2.5 text-[12px] font-mono border-l border-gray-200 dark:border-zinc-800",
            row.delta !== null && row.delta < 0 && !row.invert ? "text-emerald-400" :
            row.delta !== null && row.delta > 0 && !row.invert ? "text-red-400"    :
            row.delta !== null && row.delta > 0 &&  row.invert ? "text-emerald-400":
            row.delta !== null && row.delta < 0 &&  row.invert ? "text-red-400"    :
            "text-gray-700 dark:text-zinc-300",
          )}>
            {row.rightValue}
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export interface EvalComparisonPageProps {
  leftId:  string;
  rightId: string;
}

export function EvalComparisonPage({ leftId, rightId }: EvalComparisonPageProps) {
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ["evals-compare", leftId, rightId],
    queryFn:  () => defaultApi.getEvalRuns(500),
    staleTime: 15_000,
  });

  const runs = data?.items ?? [];
  const left  = runs.find((r) => r.eval_run_id === leftId);
  const right = runs.find((r) => r.eval_run_id === rightId);

  // Derived comparison metrics.
  const comparison = useMemo(() => {
    if (!left || !right) return null;

    const leftScore  = numericScore(left);
    const rightScore = numericScore(right);
    const scoreDelta = leftScore !== null && rightScore !== null
      ? leftScore - rightScore
      : null;

    const leftDur  = durationMs(left);
    const rightDur = durationMs(right);
    const durDelta = leftDur !== null && rightDur !== null
      ? leftDur - rightDur
      : null;

    const leftStatus  = deriveStatus(left);
    const rightStatus = deriveStatus(right);

    const highlight: { left: "win" | "lose" | "tie"; right: "win" | "lose" | "tie" } = (() => {
      if (scoreDelta === null) return { left: "tie", right: "tie" };
      if (scoreDelta > 0)  return { left: "win", right: "lose" };
      if (scoreDelta < 0)  return { left: "lose", right: "win" };
      // Same score — faster is better
      if (durDelta !== null && durDelta < 0) return { left: "win", right: "lose" };
      if (durDelta !== null && durDelta > 0) return { left: "lose", right: "win" };
      return { left: "tie", right: "tie" };
    })();

    const rows: ComparisonRow[] = [
      {
        dimension:  "Outcome",
        leftValue:  left.success === true  ? "Passed" : left.success === false ? "Failed" : "Pending",
        rightValue: right.success === true ? "Passed" : right.success === false ? "Failed" : "Pending",
        delta:      scoreDelta !== null ? scoreDelta * 100 : null,
        unit:       "%",
        match:      left.success === right.success,
      },
      {
        dimension:  "Evaluator Type",
        leftValue:  left.evaluator_type,
        rightValue: right.evaluator_type,
        delta:      null,
        match:      left.evaluator_type === right.evaluator_type,
      },
      {
        dimension:  "Subject Kind",
        leftValue:  left.subject_kind,
        rightValue: right.subject_kind,
        delta:      null,
        match:      left.subject_kind === right.subject_kind,
      },
      {
        dimension:  "Status",
        leftValue:  leftStatus,
        rightValue: rightStatus,
        delta:      null,
        match:      leftStatus === rightStatus,
      },
      {
        dimension:  "Duration",
        leftValue:  fmtDuration(left.started_at, left.completed_at),
        rightValue: fmtDuration(right.started_at, right.completed_at),
        delta:      durDelta,
        unit:       "ms",
        invert:     true,
        match:      true,
      },
      {
        dimension:  "Started At",
        leftValue:  fmtTime(left.started_at),
        rightValue: fmtTime(right.started_at),
        delta:      null,
        match:      true,
      },
    ];

    return { scoreDelta, durDelta, highlight, rows };
  }, [left, right]);

  // ── Render ──────────────────────────────────────────────────────────────────

  const handleBack = () => { window.location.hash = "evals"; };

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950 overflow-hidden">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <button
          onClick={handleBack}
          className="p-1 rounded text-gray-400 dark:text-zinc-500 hover:text-gray-800 dark:hover:text-zinc-200 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors"
          title="Back to Evaluations"
        >
          <ArrowLeft size={14} />
        </button>
        <FlaskConical size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">Eval Comparison</span>
        <span className="text-[11px] text-gray-400 dark:text-zinc-600 font-mono hidden sm:block">
          {leftId.slice(0, 12)}… vs {rightId.slice(0, 12)}…
        </span>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto px-4 py-5 space-y-5">
        {isLoading ? (
          <div className="flex items-center justify-center py-16 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading eval runs…</span>
          </div>
        ) : isError ? (
          <div className="flex flex-col items-center justify-center py-16 gap-2 text-center">
            <AlertTriangle size={24} className="text-red-500" />
            <p className="text-[13px] text-gray-700 dark:text-zinc-300">Failed to load eval runs</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-500">{error instanceof Error ? error.message : "Unknown error"}</p>
          </div>
        ) : !left || !right ? (
          <div className="flex flex-col items-center justify-center py-16 gap-2 text-center">
            <AlertTriangle size={24} className="text-amber-500" />
            <p className="text-[13px] text-gray-700 dark:text-zinc-300">
              {!left && !right ? "Neither run found" :
               !left           ? `Left run not found: ${leftId}` :
                                 `Right run not found: ${rightId}`}
            </p>
            <button onClick={handleBack} className="mt-2 text-[12px] text-indigo-400 hover:text-indigo-300 transition-colors">
              ← Back to Evaluations
            </button>
          </div>
        ) : (
          <>
            {/* Score delta headline */}
            {comparison && comparison.scoreDelta !== null && (
              <div className={clsx(
                "flex items-center justify-center gap-3 rounded-xl border py-4 px-6",
                comparison.scoreDelta > 0
                  ? "border-emerald-500/30 bg-emerald-950/20"
                  : comparison.scoreDelta < 0
                  ? "border-red-500/30 bg-red-950/20"
                  : "border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900",
              )}>
                {comparison.scoreDelta > 0 ? (
                  <TrendingUp size={20} className="text-emerald-400" />
                ) : comparison.scoreDelta < 0 ? (
                  <TrendingDown size={20} className="text-red-400" />
                ) : (
                  <Minus size={20} className="text-gray-400 dark:text-zinc-500" />
                )}
                <div className="text-center">
                  <p className={clsx(
                    "text-[28px] font-semibold tabular-nums leading-none",
                    comparison.scoreDelta > 0 ? "text-emerald-400" :
                    comparison.scoreDelta < 0 ? "text-red-400"     : "text-gray-500 dark:text-zinc-400",
                  )}>
                    {comparison.scoreDelta > 0 ? "+" : ""}
                    {(comparison.scoreDelta * 100).toFixed(0)}%
                  </p>
                  <p className="text-[11px] text-gray-400 dark:text-zinc-500 mt-1">
                    {comparison.scoreDelta > 0
                      ? "Left is better"
                      : comparison.scoreDelta < 0
                      ? "Right is better"
                      : "No difference in score"}
                  </p>
                </div>
              </div>
            )}

            {/* Side-by-side panels */}
            <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
              <RunPanel run={left}  side="left"  highlight={comparison?.highlight.left  ?? "tie"} />
              <RunPanel run={right} side="right" highlight={comparison?.highlight.right ?? "tie"} />
            </div>

            {/* Comparison table */}
            <div>
              <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-3">
                Detailed Comparison
              </p>
              {comparison && <ComparisonTable rows={comparison.rows} />}
            </div>

            {/* Mismatch callout */}
            {left.evaluator_type !== right.evaluator_type && (
              <div className="flex items-start gap-2.5 rounded-lg border border-amber-600/30 bg-amber-950/20 px-4 py-3">
                <AlertTriangle size={14} className="text-amber-400 mt-0.5 shrink-0" />
                <div>
                  <p className="text-[12px] font-medium text-amber-300">Evaluator type mismatch</p>
                  <p className="text-[11px] text-amber-600 mt-0.5">
                    Comparing <span className="font-mono">{left.evaluator_type}</span> vs{" "}
                    <span className="font-mono">{right.evaluator_type}</span> — results may not be directly comparable.
                  </p>
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default EvalComparisonPage;
