import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  RefreshCw, Loader2, Unlock, LayoutList, LayoutGrid,
  ChevronDown, ChevronRight, XCircle, ListChecks,
} from "lucide-react";
import { clsx } from "clsx";
import { useTableKeyboard } from "../hooks/useTableKeyboard";
import { ErrorFallback } from "../components/ErrorFallback";
import { HelpTooltip } from "../components/HelpTooltip";
import { StateBadge } from "../components/StateBadge";
import { DataTable } from "../components/DataTable";
import { useToast } from "../components/Toast";
import { CopyButton } from "../components/CopyButton";
import { defaultApi } from "../lib/api";
import { card as cardPreset } from "../lib/design-system";
import type { TaskRecord, TaskState } from "../lib/types";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";

// ── Helpers ───────────────────────────────────────────────────────────────────

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const fmtAge = (ms: number): string => {
  const d = Date.now() - ms;
  if (d < 60_000)      return `${Math.floor(d / 1_000)}s`;
  if (d < 3_600_000)   return `${Math.floor(d / 60_000)}m`;
  if (d < 86_400_000)  return `${Math.floor(d / 3_600_000)}h`;
  return `${Math.floor(d / 86_400_000)}d`;
};

const fmtRelative = (ms: number): string => {
  const d = Date.now() - ms;
  if (d < 60_000)      return "just now";
  if (d < 3_600_000)   return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)  return `${Math.floor(d / 3_600_000)}h ago`;
  if (d < 604_800_000) return `${Math.floor(d / 86_400_000)}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
};

const ACTIVE_STATES: TaskState[] = [
  "queued", "leased", "running", "paused", "waiting_dependency",
];

const ALL_STATES: (TaskState | "all")[] = [
  "all", "queued", "leased", "running", "completed", "failed",
  "canceled", "paused", "waiting_dependency", "retryable_failed", "dead_lettered",
];

// ── State visual config ───────────────────────────────────────────────────────

interface StateConfig {
  label:      string;
  dot:        string;
  badge:      string;
  cardBorder: string;
  cardBg:     string;
  headBg:     string;
  headText:   string;
}

const STATE_CONFIG: Partial<Record<TaskState, StateConfig>> = {
  queued: {
    label: "Queued", dot: "bg-amber-400",
    badge: "text-amber-400 bg-amber-400/10",
    cardBorder: "border-amber-800/40", cardBg: "bg-gray-50 dark:bg-zinc-900",
    headBg: "bg-amber-950/30", headText: "text-amber-400",
  },
  leased: {
    label: "Claimed", dot: "bg-indigo-400",
    badge: "text-indigo-400 bg-indigo-400/10",
    cardBorder: "border-indigo-800/40", cardBg: "bg-gray-50 dark:bg-zinc-900",
    headBg: "bg-indigo-950/30", headText: "text-indigo-400",
  },
  running: {
    label: "Running", dot: "bg-blue-400 animate-pulse",
    badge: "text-blue-400 bg-blue-400/10",
    cardBorder: "border-blue-800/40", cardBg: "bg-gray-50 dark:bg-zinc-900",
    headBg: "bg-blue-950/30", headText: "text-blue-400",
  },
  paused: {
    label: "Paused", dot: "bg-zinc-500",
    badge: "text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800",
    cardBorder: "border-gray-200 dark:border-zinc-700/40", cardBg: "bg-gray-50/60 dark:bg-zinc-900/60",
    headBg: "bg-gray-100/50 dark:bg-zinc-800/50", headText: "text-gray-500 dark:text-zinc-400",
  },
  waiting_dependency: {
    label: "Waiting", dot: "bg-purple-400",
    badge: "text-purple-400 bg-purple-400/10",
    cardBorder: "border-purple-800/40", cardBg: "bg-gray-50 dark:bg-zinc-900",
    headBg: "bg-purple-950/30", headText: "text-purple-400",
  },
  completed: {
    label: "Completed", dot: "bg-emerald-500",
    badge: "text-emerald-400 bg-emerald-400/10",
    cardBorder: "border-emerald-900/40", cardBg: "bg-gray-50/60 dark:bg-zinc-900/60",
    headBg: "bg-emerald-950/20", headText: "text-emerald-400",
  },
  failed: {
    label: "Failed", dot: "bg-red-500",
    badge: "text-red-400 bg-red-400/10",
    cardBorder: "border-red-900/40", cardBg: "bg-gray-50/60 dark:bg-zinc-900/60",
    headBg: "bg-red-950/20", headText: "text-red-400",
  },
  canceled: {
    label: "Cancelled", dot: "bg-zinc-600",
    badge: "text-gray-400 dark:text-zinc-500 bg-gray-100/60 dark:bg-zinc-800/60",
    cardBorder: "border-gray-200/30 dark:border-zinc-800/30", cardBg: "bg-gray-50/40 dark:bg-zinc-900/40",
    headBg: "bg-gray-100/30 dark:bg-zinc-800/30", headText: "text-gray-400 dark:text-zinc-500",
  },
};

const BOARD_COLUMNS: TaskState[] = [
  "queued", "leased", "running", "paused", "waiting_dependency",
  "completed", "failed", "canceled",
];

// ── Lifecycle diagram (pure SVG) ──────────────────────────────────────────────

function LifecycleDiagram() {
  const W = 640, H = 110;
  // Node layout: x-center, y-center, label, color
  const nodes: { x: number; y: number; label: string; color: string; state: string }[] = [
    { x:  56, y: 38, label: "Queued",    color: "#f59e0b", state: "queued"    },
    { x: 176, y: 38, label: "Claimed",   color: "#818cf8", state: "leased"    },
    { x: 296, y: 38, label: "Running",   color: "#60a5fa", state: "running"   },
    { x: 432, y: 20, label: "Completed", color: "#34d399", state: "completed" },
    { x: 432, y: 58, label: "Failed",    color: "#f87171", state: "failed"    },
    { x: 576, y: 38, label: "Cancelled", color: "#71717a", state: "canceled"  },
  ];

  const nW = 72, nH = 22, r = 5;

  // Edge definitions: [fromIdx, toIdx, label?, dashed?]
  const edges: [number, number, string?, boolean?][] = [
    [0, 1],                       // queued → claimed
    [1, 2],                       // claimed → running
    [2, 3],                       // running → completed
    [2, 4],                       // running → failed
    [1, 0, "release", true],      // claimed → queued (release)
    [4, 0, "retry", true],        // failed → queued (retry)
    [3, 5],                       // completed → cancelled (cancel)
    [4, 5],                       // failed → cancelled
  ];

  function nodeCenter(idx: number): [number, number] {
    const n = nodes[idx];
    return [n.x, n.y];
  }

  function edgePath(from: number, to: number): string {
    const [fx, fy] = nodeCenter(from);
    const [tx, ty] = nodeCenter(to);
    const dx = tx - fx;
    const dy = ty - fy;

    // straight right
    if (Math.abs(dy) < 4) {
      const sx = fx + nW / 2;
      const ex = tx - nW / 2;
      return `M ${sx} ${fy} L ${ex} ${ty}`;
    }
    // curved (going down or up between nodes)
    const sx = fx + (dx > 0 ? nW / 2 : -nW / 2);
    const ex = tx + (dx > 0 ? -nW / 2 : nW / 2);
    const cy = (fy + ty) / 2;
    return `M ${sx} ${fy} C ${sx} ${cy}, ${ex} ${cy}, ${ex} ${ty}`;
  }

  return (
    <svg
      width="100%"
      viewBox={`0 0 ${W} ${H}`}
      className="overflow-visible"
      aria-label="Task lifecycle state machine"
    >
      <defs>
        <marker id="arrow" markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto">
          <path d="M0,0 L0,6 L6,3 z" fill="#52525b" />
        </marker>
        <marker id="arrow-retry" markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto">
          <path d="M0,0 L0,6 L6,3 z" fill="#3f3f46" />
        </marker>
      </defs>

      {/* Edges */}
      {edges.map(([f, t, label, dashed], i) => {
        const path = edgePath(f, t);
        const stroke = dashed ? "#3f3f46" : "#52525b";
        return (
          <g key={i}>
            <path
              d={path}
              fill="none"
              stroke={stroke}
              strokeWidth={1.2}
              strokeDasharray={dashed ? "3 2" : undefined}
              markerEnd={`url(#${dashed ? "arrow-retry" : "arrow"})`}
            />
            {label && (() => {
              // Label midpoint — rough center of path
              const [fx, fy] = nodeCenter(f);
              const [tx, ty] = nodeCenter(t);
              const mx = (fx + tx) / 2;
              const my = (fy + ty) / 2 - 5;
              return (
                <text
                  x={mx} y={my}
                  textAnchor="middle"
                  fontSize="8"
                  fill="#52525b"
                  fontFamily="monospace"
                >
                  {label}
                </text>
              );
            })()}
          </g>
        );
      })}

      {/* Nodes */}
      {nodes.map((n, i) => (
        <g key={i}>
          <rect
            x={n.x - nW / 2}
            y={n.y - nH / 2}
            width={nW}
            height={nH}
            rx={r}
            fill={n.color + "18"}
            stroke={n.color + "60"}
            strokeWidth={1}
          />
          <circle cx={n.x - nW / 2 + 10} cy={n.y} r={3} fill={n.color} opacity={0.9} />
          <text
            x={n.x + 2}
            y={n.y + 4}
            textAnchor="middle"
            fontSize="9.5"
            fontFamily="ui-monospace, monospace"
            fill={n.color}
            fontWeight="500"
          >
            {n.label}
          </text>
        </g>
      ))}
    </svg>
  );
}

function LifecycleBanner() {
  const [open, setOpen] = useState(false);
  return (
    <div className="border-b border-gray-200 dark:border-zinc-800 shrink-0">
      <button
        type="button"
        onClick={() => setOpen(v => !v)}
        aria-expanded={open}
        aria-controls="lifecycle-diagram"
        className="w-full flex items-center gap-2 px-4 py-2 text-left hover:bg-gray-50/40 dark:bg-zinc-900/40 transition-colors"
      >
        {open
          ? <ChevronDown  size={11} className="text-gray-400 dark:text-zinc-600 shrink-0" />
          : <ChevronRight size={11} className="text-gray-400 dark:text-zinc-600 shrink-0" />
        }
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">
          Task Lifecycle
        </span>
        {!open && (
          <span className="text-[10px] text-gray-300 dark:text-zinc-600 ml-1">
            queued → claimed → running → completed / failed
          </span>
        )}
      </button>
      {open && (
        <div id="lifecycle-diagram" className="px-4 pb-3 bg-white dark:bg-zinc-950/40">
          <LifecycleDiagram />
        </div>
      )}
    </div>
  );
}

// ── Kanban task card ──────────────────────────────────────────────────────────

function TaskCard({ task, cfg }: { task: TaskRecord; cfg: StateConfig }) {
  function handleClick() {
    if (task.parent_run_id) {
      window.location.hash = `run/${task.parent_run_id}`;
    }
  }

  return (
    <div
      onClick={task.parent_run_id ? handleClick : undefined}
      className={clsx(
        "rounded-md border px-2.5 py-2 space-y-1.5 select-none",
        cfg.cardBorder, cfg.cardBg,
        task.parent_run_id ? "cursor-pointer hover:brightness-110 transition-all" : "",
      )}
    >
      {/* Task ID */}
      <p className="text-[11px] font-mono text-gray-700 dark:text-zinc-300 truncate" title={task.task_id}>
        {shortId(task.task_id)}
      </p>

      {/* Run ID link */}
      {task.parent_run_id && (
        <p className="text-[10px] font-mono text-gray-400 dark:text-zinc-600 truncate" title={task.parent_run_id}>
          run: {shortId(task.parent_run_id)}
        </p>
      )}

      {/* Worker */}
      {task.lease_owner && (
        <p className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 truncate" title={task.lease_owner}>
          ◎ {shortId(task.lease_owner)}
        </p>
      )}

      {/* Age */}
      <div className="flex items-center justify-between">
        <span className="text-[10px] text-gray-300 dark:text-zinc-600 tabular-nums">
          {fmtAge(task.created_at)} old
        </span>
        {task.failure_class && (
          <span className="text-[9px] font-mono text-red-600 truncate max-w-[72px]" title={task.failure_class}>
            {task.failure_class}
          </span>
        )}
      </div>
    </div>
  );
}

// ── Kanban column ─────────────────────────────────────────────────────────────

const MAX_CARDS = 20;

function KanbanColumn({ state, tasks }: { state: TaskState; tasks: TaskRecord[] }) {
  const cfg = STATE_CONFIG[state];
  if (!cfg) return null;

  const shown  = tasks.slice(0, MAX_CARDS);
  const hidden = tasks.length - shown.length;

  return (
    <div className={clsx(cardPreset.shell, "flex flex-col min-w-[180px] max-w-[200px] shrink-0")}>
      {/* Column header */}
      <div className={clsx("flex items-center gap-2 px-3 py-2 shrink-0", cfg.headBg)}>
        <span className={clsx("w-2 h-2 rounded-full shrink-0", cfg.dot)} />
        <span className={clsx("text-[11px] font-medium flex-1", cfg.headText)}>
          {cfg.label}
        </span>
        <span className={clsx(
          "text-[10px] font-mono tabular-nums rounded-full px-1.5 py-0.5 min-w-[20px] text-center",
          cfg.badge,
        )}>
          {tasks.length}
        </span>
      </div>

      {/* Cards */}
      <div className="flex-1 overflow-y-auto p-2 space-y-1.5 bg-white dark:bg-zinc-950/30 min-h-[80px]">
        {shown.length === 0 ? (
          <div className="flex items-center justify-center h-12">
            <span className="text-[10px] text-gray-400 dark:text-zinc-600">empty</span>
          </div>
        ) : (
          <>
            {shown.map(t => (
              <TaskCard key={t.task_id} task={t} cfg={cfg} />
            ))}
            {hidden > 0 && (
              <p className="text-[10px] text-gray-300 dark:text-zinc-600 text-center py-1">
                +{hidden} more
              </p>
            )}
          </>
        )}
      </div>
    </div>
  );
}

// ── Board view ────────────────────────────────────────────────────────────────

function BoardView({ tasks }: { tasks: TaskRecord[] }) {
  const byState: Partial<Record<TaskState, TaskRecord[]>> = {};
  for (const col of BOARD_COLUMNS) byState[col] = [];

  for (const t of tasks) {
    if (BOARD_COLUMNS.includes(t.state as TaskState)) {
      byState[t.state as TaskState]!.push(t);
    }
  }

  // Sort within each column: most recent first for terminal states, oldest first for active
  for (const [state, col] of Object.entries(byState) as [TaskState, TaskRecord[]][]) {
    const terminal = state === "completed" || state === "failed" || state === "canceled";
    col.sort((a, b) => terminal
      ? b.updated_at - a.updated_at
      : a.created_at - b.created_at,
    );
  }

  const totalVisible = BOARD_COLUMNS.reduce((s, c) => s + (byState[c]?.length ?? 0), 0);

  return (
    <div className="flex-1 overflow-x-auto overflow-y-hidden p-3">
      {totalVisible === 0 ? (
        <div className="flex flex-col items-center justify-center h-full gap-2 text-center px-6">
          <ListChecks size={28} className="text-gray-300 dark:text-zinc-600" />
          <p className="text-[13px] text-gray-400 dark:text-zinc-600 font-medium">No tasks yet</p>
          <p className="text-[11px] text-gray-300 dark:text-zinc-600 max-w-xs">
            Tasks are created automatically when a run starts executing work.
            Start a run in the <a href="#runs" onClick={() => { window.location.hash = "runs"; }} className="text-indigo-500 hover:text-indigo-400">Runs</a> page to see tasks appear here.
          </p>
        </div>
      ) : (
        <div className="flex gap-2.5 h-full">
          {BOARD_COLUMNS.map(state => (
            <KanbanColumn
              key={state}
              state={state}
              tasks={byState[state] ?? []}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Row actions ───────────────────────────────────────────────────────────────

function RowActions({ task }: { task: TaskRecord }) {
  const qc    = useQueryClient();
  const toast = useToast();

  // Also invalidate `run-tasks` so RunDetailPage's per-run task list reflects
  // the new worker/lease state immediately instead of polling stale values.
  const claim = useMutation({
    mutationFn: () => defaultApi.claimTask(task.task_id, "operator", 60_000),
    onSuccess: () => {
      toast.success("Task claimed.");
      void qc.invalidateQueries({ queryKey: ["tasks"] });
      void qc.invalidateQueries({ queryKey: ["run-tasks"] });
    },
    onError:   () => toast.error("Failed to claim task."),
  });

  const release = useMutation({
    mutationFn: () => defaultApi.releaseLease(task.task_id),
    onSuccess: () => {
      toast.success("Lease released.");
      void qc.invalidateQueries({ queryKey: ["tasks"] });
      void qc.invalidateQueries({ queryKey: ["run-tasks"] });
    },
    onError:   () => toast.error("Failed to release lease."),
  });

  const canClaim   = task.state === "queued";
  const canRelease = (task.state === "leased" || task.state === "running") && !!task.lease_owner;

  if (!canClaim && !canRelease) return null;

  return (
    <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
      {canClaim && (
        <>
          <HelpTooltip text="Claim: assign this queued task to yourself as the worker. Sets state to 'leased' with a 60-second expiry." placement="left" />
          <button
            onClick={e => { e.stopPropagation(); claim.mutate(); }}
            disabled={claim.isPending}
            className="px-2 py-0.5 rounded text-[11px] font-medium bg-indigo-900/60 text-indigo-300
                       hover:bg-indigo-900 border border-indigo-800/50 transition-colors disabled:opacity-40"
          >
            {claim.isPending ? <Loader2 size={10} className="animate-spin inline" /> : "Claim"}
          </button>
        </>
      )}
      {canRelease && (
        <>
          <HelpTooltip text="Release: return this leased task to the queue so another worker can pick it up." placement="left" />
          <button
            onClick={e => { e.stopPropagation(); release.mutate(); }}
            disabled={release.isPending}
            className="px-2 py-0.5 rounded text-[11px] font-medium bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400
                       hover:bg-gray-200 dark:hover:bg-zinc-700 border border-gray-200 dark:border-zinc-700 transition-colors disabled:opacity-40"
          >
            {release.isPending ? <Loader2 size={10} className="animate-spin inline" /> : "Release"}
          </button>
        </>
      )}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

type ViewMode = "table" | "board";

export function TasksPage() {
  const { ms: refreshMs, setOption: setRefreshOption, interval: refreshInterval } = useAutoRefresh("tasks", "15s");

  const [filter,   setFilter]   = useState<TaskState | "all">("all");
  const [viewMode, setViewMode] = useState<ViewMode>("table");
  const qc    = useQueryClient();
  const toast = useToast();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["tasks"],
    queryFn:  () => defaultApi.getAllTasks({ limit: 500 }),
    refetchInterval: refreshMs,
  });

  const tasks     = data ?? [];
  const filtered  = filter === "all" ? tasks : tasks.filter(t => t.state === filter);
  const activeCnt = tasks.filter(t => ACTIVE_STATES.includes(t.state)).length;

  const kbd = useTableKeyboard({
    items:  filtered,
    getKey: t => t.task_id,
  });

  const releaseSelected = useMutation({
    mutationFn: async () => {
      const toRelease = filtered.filter(t =>
        kbd.selectedKeys.has(t.task_id) &&
        (t.state === "leased" || t.state === "running") &&
        t.lease_owner,
      );
      await Promise.all(toRelease.map(t => defaultApi.releaseLease(t.task_id)));
      return toRelease.length;
    },
    onSuccess: n => {
      toast.success(`Released ${n} task lease${n !== 1 ? "s" : ""}.`);
      kbd.clearSelection();
      void qc.invalidateQueries({ queryKey: ["tasks"] });
    },
    onError: () => toast.error("Failed to release some leases."),
  });

  const cancelSelected = useMutation({
    mutationFn: async () => {
      const ids = filtered
        .filter(t => kbd.selectedKeys.has(t.task_id) && !["completed","failed","canceled","dead_lettered"].includes(t.state))
        .map(t => t.task_id);
      if (ids.length === 0) return { cancelled: 0, failed: [] };
      return defaultApi.batchCancelTasks(ids);
    },
    onSuccess: result => {
      const { cancelled, failed } = result;
      if (cancelled > 0) toast.success(`Cancelled ${cancelled} task${cancelled !== 1 ? "s" : ""}.`);
      if (failed.length > 0) toast.error(`${failed.length} task${failed.length !== 1 ? "s" : ""} could not be cancelled.`);
      kbd.clearSelection();
      void qc.invalidateQueries({ queryKey: ["tasks"] });
      void qc.invalidateQueries({ queryKey: ["run-tasks"] });
    },
    onError: () => toast.error("Batch cancel failed."),
  });

  if (isError) return (
    <ErrorFallback error={error} resource="tasks" onRetry={() => void refetch()} />
  );

  const selCount   = kbd.selectedKeys.size;
  const releasable = filtered.filter(t =>
    kbd.selectedKeys.has(t.task_id) &&
    (t.state === "leased" || t.state === "running") &&
    t.lease_owner,
  ).length;

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Tasks
          {!isLoading && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal">
              {filtered.length}
              {filter !== "all" && ` / ${tasks.length} total`}
              {activeCnt > 0 && filter === "all" && (
                <span className="ml-1.5 text-indigo-400">{activeCnt} active</span>
              )}
            </span>
          )}
        </span>

        {selCount > 0 && (
          <span className="text-[11px] text-indigo-400 font-medium">{selCount} selected</span>
        )}

        {/* View toggle */}
        <div className="flex items-center rounded border border-gray-200 dark:border-zinc-700 overflow-hidden ml-2">
          <button
            onClick={() => setViewMode("table")}
            title="Table view"
            className={clsx(
              "flex items-center gap-1 px-2.5 py-1 text-[11px] transition-colors",
              viewMode === "table"
                ? "bg-gray-200 dark:bg-zinc-700 text-gray-800 dark:text-zinc-200"
                : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300",
            )}
          >
            <LayoutList size={12} /> Table
          </button>
          <button
            onClick={() => setViewMode("board")}
            title="Board view"
            className={clsx(
              "flex items-center gap-1 px-2.5 py-1 text-[11px] border-l border-gray-200 dark:border-zinc-700 transition-colors",
              viewMode === "board"
                ? "bg-gray-200 dark:bg-zinc-700 text-gray-800 dark:text-zinc-200"
                : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300",
            )}
          >
            <LayoutGrid size={12} /> Board
          </button>
        </div>

        {/* State filter — only shown in table mode */}
        {viewMode === "table" && (
          <select
            value={filter}
            onChange={e => setFilter(e.target.value as TaskState | "all")}
            className="ml-1 rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800 text-[12px] text-gray-700 dark:text-zinc-300
                       px-2 py-1 focus:outline-none focus:border-indigo-500 transition-colors"
          >
            {ALL_STATES.map(s => (
              <option key={s} value={s}>
                {s === "all" ? "All states" : s.replace(/_/g, " ")}
              </option>
            ))}
          </select>
        )}

        <div className="ml-auto flex items-center gap-2">
          {releasable > 0 && viewMode === "table" && (
            <button
              onClick={() => releaseSelected.mutate()}
              disabled={releaseSelected.isPending}
              className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900
                         text-gray-500 dark:text-zinc-400 text-[12px] px-2.5 py-1 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600
                         disabled:opacity-40 transition-colors"
            >
              <Unlock size={11} />
              Release {releasable}
            </button>
          )}
          {selCount > 0 && viewMode === "table" && (
            <button
              onClick={() => {
                const cancelable = filtered.filter(t =>
                  kbd.selectedKeys.has(t.task_id) &&
                  !["completed","failed","canceled","dead_lettered"].includes(t.state)
                ).length;
                if (cancelable === 0) {
                  toast.info("Selected tasks are already in a terminal state.");
                  return;
                }
                if (!window.confirm(
                  `Cancel ${cancelable} task${cancelable !== 1 ? "s" : ""}?\n\nThis will stop them from being executed. Tasks already running may complete their current step before stopping.`
                )) return;
                cancelSelected.mutate();
              }}
              disabled={cancelSelected.isPending}
              title="Cancel selected non-terminal tasks"
              className="flex items-center gap-1.5 rounded border border-red-900/60 bg-red-950/30
                         text-red-400 text-[12px] px-2.5 py-1 hover:bg-red-950/60 hover:border-red-800
                         disabled:opacity-40 transition-colors"
            >
              <XCircle size={11} />
              Cancel {selCount}
            </button>
          )}
          {selCount > 0 && viewMode === "table" && (
            <button
              onClick={kbd.clearSelection}
              className="text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
            >
              Clear
            </button>
          )}
        </div>
        <div className="flex items-center gap-1">
          <div className="relative">
            <select value={refreshInterval.option} onChange={e => setRefreshOption(e.target.value as import('../hooks/useAutoRefresh').RefreshOption)}
              className="appearance-none rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] font-mono pl-5 pr-2 h-7 text-gray-500 dark:text-zinc-400 focus:outline-none focus:border-indigo-500 transition-colors">
              {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
            </select>
            <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none">
              <RefreshCw size={9} className={isFetching ? "animate-spin text-indigo-400" : "text-gray-400 dark:text-zinc-600"} />
            </span>
          </div>
          <button onClick={() => refetch()} disabled={isFetching}
            className="flex items-center gap-1 h-7 px-2 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600 disabled:opacity-40 transition-colors">
            <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
            <span className="hidden sm:inline">Refresh</span>
          </button>
        </div>
      </div>

      {/* Lifecycle diagram */}
      <LifecycleBanner />

      {/* Content */}
      {isLoading ? (
        /* Skeleton rows — gives users a sense of the layout while loading */
        <div className="flex-1 overflow-hidden">
          <div className="divide-y divide-gray-200 dark:divide-zinc-800/40">
            {Array.from({ length: 8 }).map((_, i) => (
              <div key={i} className="flex items-center gap-4 px-4 h-9 animate-pulse">
                <div className="h-2.5 w-28 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="h-2.5 w-20 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="h-4 w-16 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="h-2.5 w-20 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="ml-auto h-2.5 w-16 rounded bg-gray-100 dark:bg-zinc-800" />
              </div>
            ))}
          </div>
        </div>
      ) : viewMode === "board" ? (
        <BoardView tasks={tasks} />
      ) : (
        <div
          {...kbd.containerProps}
          className={`flex-1 overflow-x-auto overflow-y-auto ${kbd.containerProps.className}`}
        >
          <DataTable<TaskRecord>
            data={filtered}
            activeIndex={kbd.activeIndex}
            selectedIds={kbd.selectedKeys}
            getRowId={t => t.task_id}
            columns={[
              { key: "task_id",    header: "Task ID",    render: r => <span className="flex items-center gap-1 font-mono text-xs text-gray-700 dark:text-zinc-300 whitespace-nowrap group/id" title={r.task_id}>{shortId(r.task_id)}<CopyButton text={r.task_id} label="Copy task ID" size={10} className="opacity-0 group-hover/id:opacity-100" /></span>,                sortValue: r => r.task_id },
              { key: "run",        header: "Run",         render: r => r.parent_run_id ? <span className="font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap" title={r.parent_run_id}>{shortId(r.parent_run_id)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span> },
              { key: "state",      header: "Status",      render: r => <StateBadge state={r.state as Parameters<typeof StateBadge>[0]["state"]} compact />, sortValue: r => r.state },
              { key: "worker",     header: "Worker",      render: r => r.lease_owner ? <span className="font-mono text-[11px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">{shortId(r.lease_owner)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span> },
              { key: "queued_at",  header: "Queued",      render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums whitespace-nowrap" title={fmtTime(r.created_at)}>{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at },
              { key: "started_at", header: "Started At",  render: r => r.lease_expires_at ? <span className="text-[11px] text-gray-500 dark:text-zinc-400 tabular-nums whitespace-nowrap">{fmtTime(r.updated_at)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span>, sortValue: r => r.updated_at },
              { key: "actions",    header: "",             render: r => <RowActions task={r} /> },
            ]}
            filterFn={(r, q) => r.task_id.includes(q) || r.state.includes(q) || (r.parent_run_id ?? "").includes(q) || (r.lease_owner ?? "").includes(q)}
            csvRow={r => [r.task_id, r.parent_run_id ?? "", r.state, r.lease_owner ?? "", r.created_at, r.updated_at]}
            csvHeaders={["Task ID", "Run ID", "State", "Worker", "Queued At", "Updated At"]}
            filename="tasks"
            emptyText="No tasks match this filter — try a different state or clear the search"
          />
        </div>
      )}
    </div>
  );
}

export default TasksPage;
