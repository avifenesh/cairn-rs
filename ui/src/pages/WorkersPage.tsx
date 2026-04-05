/**
 * WorkersPage — monitor connected runtime workers.
 *
 * Workers are not a first-class API resource; their identity is derived from
 * lease_owner on TaskRecords.  This page aggregates GET /v1/tasks to build
 * per-worker summaries: current task, completed count, avg duration, heartbeat.
 */

import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  RefreshCw, Loader2, Users, ChevronDown, ChevronRight,
  Activity, CheckCircle2, Clock, AlertTriangle, Cpu,
} from "lucide-react";
import { clsx } from "clsx";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";
import type { TaskRecord, TaskState } from "../lib/types";

// ── Types ─────────────────────────────────────────────────────────────────────

interface WorkerSummary {
  worker_id:       string;
  /** Tasks currently held (leased / running). */
  active_tasks:    TaskRecord[];
  /** Tasks completed or failed (historical). */
  history:         TaskRecord[];
  completed_count: number;
  failed_count:    number;
  /** Average duration of completed tasks in ms (null if none). */
  avg_duration_ms: number | null;
  /** Most recent updated_at across all known tasks for this worker. */
  last_seen_ms:    number;
  status:          "active" | "idle";
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const ACTIVE_STATES = new Set<TaskState>(["leased", "running"]);
const DONE_STATES   = new Set<TaskState>(["completed", "failed", "canceled", "dead_lettered"]);

function fmtDuration(ms: number): string {
  if (ms < 1_000)  return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1_000)}s`;
}

function fmtRelative(ms: number): string {
  const d = Date.now() - ms;
  if (d < 30_000)       return "just now";
  if (d < 60_000)       return `${Math.floor(d / 1_000)}s ago`;
  if (d < 3_600_000)    return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)   return `${Math.floor(d / 3_600_000)}h ago`;
  return `${Math.floor(d / 86_400_000)}d ago`;
}

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
}

const shortId = (id: string) =>
  id.length > 24 ? `${id.slice(0, 12)}…${id.slice(-6)}` : id;

/** Derive worker summaries from a flat list of task records. */
function buildWorkerSummaries(tasks: TaskRecord[]): WorkerSummary[] {
  const map = new Map<string, {
    active:   TaskRecord[];
    history:  TaskRecord[];
    lastSeen: number;
  }>();

  for (const t of tasks) {
    if (!t.lease_owner) continue;

    const w = t.lease_owner;
    if (!map.has(w)) {
      map.set(w, { active: [], history: [], lastSeen: 0 });
    }
    const entry = map.get(w)!;

    if (ACTIVE_STATES.has(t.state)) {
      entry.active.push(t);
    } else if (DONE_STATES.has(t.state)) {
      entry.history.push(t);
    }

    if (t.updated_at > entry.lastSeen) entry.lastSeen = t.updated_at;
  }

  return Array.from(map.entries())
    .map(([worker_id, { active, history, lastSeen }]) => {
      const completed = history.filter(t => t.state === "completed");
      const failed    = history.filter(t => t.state === "failed");

      // Average duration: completed tasks where we can approximate duration.
      // We use (updated_at - created_at) as a proxy for wall-clock duration.
      const durations = completed
        .map(t => t.updated_at - t.created_at)
        .filter(d => d > 0);
      const avg_duration_ms = durations.length > 0
        ? durations.reduce((s, d) => s + d, 0) / durations.length
        : null;

      return {
        worker_id,
        active_tasks:    active,
        history:         history.sort((a, b) => b.updated_at - a.updated_at).slice(0, 30),
        completed_count: completed.length,
        failed_count:    failed.length,
        avg_duration_ms,
        last_seen_ms:    lastSeen,
        status:          active.length > 0 ? "active" as const : "idle" as const,
      };
    })
    .sort((a, b) => {
      // Active first, then by last_seen descending.
      if (a.status !== b.status) return a.status === "active" ? -1 : 1;
      return b.last_seen_ms - a.last_seen_ms;
    });
}

// ── Stat card ─────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub, color = "indigo" }: {
  label: string;
  value: string | number;
  sub?: string;
  color?: "indigo" | "emerald" | "zinc" | "amber";
}) {
  const border = {
    indigo:  "border-l-indigo-500",
    emerald: "border-l-emerald-500",
    zinc:    "border-l-zinc-600",
    amber:   "border-l-amber-500",
  }[color];
  const valueColor = {
    indigo:  "text-indigo-400",
    emerald: "text-emerald-400",
    zinc:    "text-zinc-300",
    amber:   "text-amber-400",
  }[color];

  return (
    <div className={clsx("bg-zinc-900 border border-zinc-800 border-l-2 rounded-lg p-4", border)}>
      <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2 truncate">
        {label}
      </p>
      <p className={clsx("text-xl font-semibold tabular-nums leading-none", valueColor)}>{value}</p>
      {sub && <p className="mt-1.5 text-[11px] text-zinc-600 truncate">{sub}</p>}
    </div>
  );
}

// ── Task state badge (inline) ─────────────────────────────────────────────────

const STATE_PILL: Partial<Record<TaskState, string>> = {
  leased:    "text-indigo-400 bg-indigo-400/10",
  running:   "text-blue-400 bg-blue-400/10",
  completed: "text-emerald-400 bg-emerald-400/10",
  failed:    "text-red-400 bg-red-400/10",
  canceled:  "text-zinc-500 bg-zinc-800",
  paused:    "text-amber-400 bg-amber-400/10",
};

function StatePill({ state }: { state: TaskState }) {
  const cls = STATE_PILL[state] ?? "text-zinc-500 bg-zinc-800";
  return (
    <span className={clsx("text-[10px] font-medium rounded px-1.5 py-0.5 font-mono", cls)}>
      {state}
    </span>
  );
}

// ── Worker history panel ──────────────────────────────────────────────────────

function WorkerHistory({ tasks }: { tasks: TaskRecord[] }) {
  if (tasks.length === 0) {
    return (
      <p className="text-[12px] text-zinc-600 italic px-4 py-3">
        No completed tasks recorded for this worker yet.
      </p>
    );
  }

  return (
    <div className="divide-y divide-zinc-800/50">
      {/* Column headers */}
      <div className="grid grid-cols-[1fr_80px_80px_96px] gap-2 px-4 py-1.5 bg-zinc-950">
        {["Task ID", "State", "Duration", "Completed"].map(h => (
          <span key={h} className="text-[10px] text-zinc-600 uppercase tracking-wider">{h}</span>
        ))}
      </div>
      {tasks.map((t, i) => {
        const dur = t.updated_at - t.created_at;
        return (
          <div
            key={t.task_id}
            className={clsx(
              "grid grid-cols-[1fr_80px_80px_96px] gap-2 items-center px-4 py-2",
              i % 2 === 0 ? "bg-zinc-950/30" : "",
            )}
          >
            <span
              className="font-mono text-[11px] text-zinc-400 truncate"
              title={t.task_id}
            >
              {shortId(t.task_id)}
            </span>
            <StatePill state={t.state} />
            <span className="text-[11px] font-mono text-zinc-500 tabular-nums">
              {dur > 0 ? fmtDuration(dur) : "—"}
            </span>
            <span className="text-[10px] text-zinc-600 tabular-nums" title={fmtTime(t.updated_at)}>
              {fmtRelative(t.updated_at)}
            </span>
          </div>
        );
      })}
    </div>
  );
}

// ── Worker row ────────────────────────────────────────────────────────────────

function WorkerRow({
  worker,
  expanded,
  onToggle,
  even,
}: {
  worker: WorkerSummary;
  expanded: boolean;
  onToggle: () => void;
  even: boolean;
}) {
  const isActive   = worker.status === "active";
  const staleSecs  = (Date.now() - worker.last_seen_ms) / 1_000;
  const isStale    = !isActive && staleSecs > 300; // >5 min → stale

  return (
    <div className={clsx(
      "border-b border-zinc-800/50 last:border-0",
      even ? "bg-zinc-900" : "bg-zinc-900/50",
    )}>
      {/* Main row */}
      <div
        className="flex items-center gap-0 h-11 cursor-pointer hover:bg-white/[0.02] transition-colors select-none"
        onClick={onToggle}
      >
        {/* Expand chevron */}
        <div className="w-9 shrink-0 flex justify-center">
          {expanded
            ? <ChevronDown  size={12} className="text-zinc-600" />
            : <ChevronRight size={12} className="text-zinc-600" />
          }
        </div>

        {/* Worker ID */}
        <div className="flex-1 min-w-0 flex items-center gap-2 pr-3">
          <div className={clsx(
            "flex h-6 w-6 shrink-0 items-center justify-center rounded-full",
            isActive ? "bg-emerald-500/15" : "bg-zinc-800",
          )}>
            <Cpu size={11} className={isActive ? "text-emerald-400" : "text-zinc-600"} />
          </div>
          <span className="text-[12px] font-mono text-zinc-200 truncate" title={worker.worker_id}>
            {shortId(worker.worker_id)}
          </span>
        </div>

        {/* Status badge */}
        <div className="w-24 shrink-0 px-2">
          <span className={clsx(
            "inline-flex items-center gap-1.5 text-[11px] font-medium rounded-full px-2 py-0.5",
            isActive
              ? "text-emerald-400 bg-emerald-400/10"
              : isStale
              ? "text-zinc-600 bg-zinc-800/60"
              : "text-zinc-400 bg-zinc-800",
          )}>
            <span className={clsx(
              "w-1.5 h-1.5 rounded-full shrink-0",
              isActive ? "bg-emerald-400 animate-pulse" :
              isStale  ? "bg-zinc-700" : "bg-zinc-500",
            )} />
            {isActive ? "active" : isStale ? "stale" : "idle"}
          </span>
        </div>

        {/* Current task */}
        <div className="w-44 shrink-0 px-2">
          {worker.active_tasks.length > 0 ? (
            <span
              className="text-[11px] font-mono text-indigo-400 truncate block"
              title={worker.active_tasks[0].task_id}
            >
              {shortId(worker.active_tasks[0].task_id)}
              {worker.active_tasks.length > 1 && (
                <span className="text-zinc-600 ml-1">+{worker.active_tasks.length - 1}</span>
              )}
            </span>
          ) : (
            <span className="text-[11px] text-zinc-700">—</span>
          )}
        </div>

        {/* Completed */}
        <div className="w-24 shrink-0 px-2 flex items-center gap-1">
          <CheckCircle2 size={10} className="text-emerald-600 shrink-0" />
          <span className="text-[12px] tabular-nums text-zinc-300">
            {worker.completed_count}
          </span>
          {worker.failed_count > 0 && (
            <span className="ml-1 text-[10px] text-red-500 tabular-nums">
              / {worker.failed_count} failed
            </span>
          )}
        </div>

        {/* Avg duration */}
        <div className="w-24 shrink-0 px-2">
          <span className="text-[11px] font-mono tabular-nums text-zinc-500">
            {worker.avg_duration_ms !== null ? fmtDuration(worker.avg_duration_ms) : "—"}
          </span>
        </div>

        {/* Last heartbeat */}
        <div className="w-28 shrink-0 px-2 flex items-center gap-1">
          <Clock size={10} className="text-zinc-700 shrink-0" />
          <span
            className={clsx(
              "text-[11px] tabular-nums",
              isStale ? "text-amber-600" : "text-zinc-500",
            )}
            title={fmtTime(worker.last_seen_ms)}
          >
            {fmtRelative(worker.last_seen_ms)}
          </span>
        </div>
      </div>

      {/* Expanded history */}
      {expanded && (
        <div className="border-t border-zinc-800/60 bg-zinc-950/30">
          <div className="px-4 py-2 flex items-center gap-2">
            <Activity size={11} className="text-zinc-600" />
            <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
              Task History
            </span>
            <span className="text-[10px] text-zinc-700">
              ({worker.history.length} records)
            </span>
          </div>
          <WorkerHistory tasks={worker.history} />
        </div>
      )}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function WorkersPage() {
  const [expanded, setExpanded] = useState<string | null>(null);
  const [filter,   setFilter]   = useState<"all" | "active" | "idle">("all");

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey:        ["tasks-for-workers"],
    queryFn:         () => defaultApi.getAllTasks({ limit: 1000 }),
    refetchInterval: 10_000,
  });

  const workers = useMemo(
    () => buildWorkerSummaries(data ?? []),
    [data],
  );

  const visible = filter === "all"
    ? workers
    : workers.filter(w => w.status === filter);

  const totalWorkers  = workers.length;
  const activeWorkers = workers.filter(w => w.status === "active").length;
  const idleWorkers   = totalWorkers - activeWorkers;
  const avgTaskTime   = (() => {
    const all = workers.flatMap(w =>
      w.avg_duration_ms !== null ? [w.avg_duration_ms] : [],
    );
    return all.length > 0
      ? all.reduce((s, d) => s + d, 0) / all.length
      : null;
  })();

  if (isError) return (
    <ErrorFallback error={error} resource="workers" onRetry={() => void refetch()} />
  );

  return (
    <div className="flex flex-col h-full bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-zinc-800 shrink-0 bg-zinc-900">
        <Users size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[13px] font-medium text-zinc-200">
          Workers
          {!isLoading && (
            <span className="ml-2 text-[12px] text-zinc-500 font-normal">
              {visible.length}
              {filter !== "all" && ` / ${totalWorkers} total`}
            </span>
          )}
        </span>

        {/* Filter */}
        <div className="flex items-center rounded border border-zinc-700 overflow-hidden ml-2">
          {(["all", "active", "idle"] as const).map(f => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={clsx(
                "px-2.5 py-1 text-[11px] capitalize transition-colors",
                f !== "all" && "border-l border-zinc-700",
                filter === f
                  ? "bg-zinc-700 text-zinc-200"
                  : "text-zinc-500 hover:text-zinc-300",
              )}
            >
              {f}
            </button>
          ))}
        </div>

        <button
          onClick={() => refetch()}
          disabled={isFetching}
          className="ml-auto flex items-center gap-1 text-[12px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {/* Stat cards */}
      {!isLoading && totalWorkers > 0 && (
        <div className="grid grid-cols-2 gap-3 px-4 py-4 border-b border-zinc-800 shrink-0 lg:grid-cols-4">
          <StatCard
            label="Total Workers"
            value={totalWorkers}
            sub="seen via task leases"
            color="indigo"
          />
          <StatCard
            label="Active"
            value={activeWorkers}
            sub={activeWorkers > 0 ? "holding tasks now" : "none running"}
            color="emerald"
          />
          <StatCard
            label="Idle"
            value={idleWorkers}
            sub="no current task"
            color="zinc"
          />
          <StatCard
            label="Avg Task Time"
            value={avgTaskTime !== null ? fmtDuration(avgTaskTime) : "—"}
            sub="across completed tasks"
            color="amber"
          />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Aggregating worker data…</span>
          </div>
        ) : visible.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-zinc-800 border border-zinc-700">
              <Users size={24} className="text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-zinc-400">
              {totalWorkers === 0 ? "No workers seen yet" : `No ${filter} workers`}
            </p>
            <p className="text-[12px] text-zinc-600 max-w-xs">
              {totalWorkers === 0
                ? "Workers appear here once they claim a task. Use POST /v1/tasks/:id/claim to register a worker."
                : "Try switching the filter to 'all'."}
            </p>
          </div>
        ) : (
          <div className="min-w-[720px]">
            {/* Column headers */}
            <div className="flex items-center h-8 border-b border-zinc-800 bg-zinc-950 sticky top-0">
              <div className="w-9 shrink-0" />
              <div className="flex-1 px-2">
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Worker ID</span>
              </div>
              <div className="w-24 shrink-0 px-2">
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Status</span>
              </div>
              <div className="w-44 shrink-0 px-2">
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Current Task</span>
              </div>
              <div className="w-24 shrink-0 px-2">
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Completed</span>
              </div>
              <div className="w-24 shrink-0 px-2">
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Avg Duration</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Last Heartbeat</span>
              </div>
            </div>

            {visible.map((worker, i) => (
              <WorkerRow
                key={worker.worker_id}
                worker={worker}
                even={i % 2 === 0}
                expanded={expanded === worker.worker_id}
                onToggle={() => setExpanded(v => v === worker.worker_id ? null : worker.worker_id)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Data source note */}
      {!isLoading && totalWorkers > 0 && (
        <div className="flex items-center gap-1.5 px-4 py-2 border-t border-zinc-800 shrink-0">
          <AlertTriangle size={10} className="text-zinc-700 shrink-0" />
          <span className="text-[10px] text-zinc-700">
            Worker data derived from task lease records — refreshes every 10 s.
            Only workers that have claimed at least one task appear here.
          </span>
        </div>
      )}
    </div>
  );
}

export default WorkersPage;
