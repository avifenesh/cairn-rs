import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  ListChecks,
  RefreshCw,
  ServerCrash,
  Cpu,
  ChevronRight,
  X,
  Play,
  Unlock,
  Clock,
  AlertCircle,
} from "lucide-react";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { defaultApi } from "../lib/api";
import type { TaskRecord, TaskState, RunState } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
}

function shortId(id: string): string {
  return id.length > 20 ? `${id.slice(0, 8)}\u2026${id.slice(-5)}` : id;
}

// Reuse StateBadge's colour set for TaskState — the states overlap enough
// that we can cast safely.
function TaskStateBadge({ state, compact }: { state: TaskState; compact?: boolean }) {
  // Map task-only states to the closest RunState analogue.
  const mapped: Record<TaskState, RunState> = {
    queued:               "pending",
    leased:               "running",
    running:              "running",
    completed:            "completed",
    failed:               "failed",
    canceled:             "canceled",
    paused:               "paused",
    waiting_dependency:   "waiting_dependency" as RunState,
    retryable_failed:     "failed",
    dead_lettered:        "failed",
  };
  return <StateBadge state={mapped[state] ?? ("pending" as RunState)} compact={compact} />;
}

// ── State filter options ──────────────────────────────────────────────────────

const ALL_TASK_STATES: TaskState[] = [
  "queued", "leased", "running", "completed",
  "failed", "canceled", "paused",
  "waiting_dependency", "retryable_failed", "dead_lettered",
];
const TASK_STATE_LABELS: Record<TaskState, string> = {
  queued:              "Queued",
  leased:              "Leased",
  running:             "Running",
  completed:           "Completed",
  failed:              "Failed",
  canceled:            "Canceled",
  paused:              "Paused",
  waiting_dependency:  "Waiting",
  retryable_failed:    "Retryable Failed",
  dead_lettered:       "Dead Letter",
};

// ── Detail panel ──────────────────────────────────────────────────────────────

interface DetailPanelProps {
  task: TaskRecord;
  onClose: () => void;
  onClaim: (taskId: string) => void;
  onRelease: (taskId: string) => void;
  claiming: boolean;
  releasing: boolean;
}

function DetailPanel({ task, onClose, onClaim, onRelease, claiming, releasing }: DetailPanelProps) {
  const [workerId, setWorkerId] = useState("operator-1");
  const canClaim   = task.state === "queued";
  const canRelease = task.state === "leased";

  return (
    <aside className="flex flex-col w-88 shrink-0 border-l border-zinc-800 bg-zinc-900 h-full overflow-y-auto">
      {/* Header */}
      <div className="flex items-center gap-2 px-5 py-4 border-b border-zinc-800 sticky top-0 bg-zinc-900 z-10">
        <ChevronRight size={14} className="text-zinc-500 shrink-0" />
        <span className="text-sm font-semibold font-mono text-zinc-100 truncate flex-1">
          {shortId(task.task_id)}
        </span>
        <button
          onClick={onClose}
          className="rounded p-1 text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
          aria-label="Close"
        >
          <X size={15} />
        </button>
      </div>

      <div className="flex-1 p-5 space-y-5">
        {/* State */}
        <div>
          <p className="text-[10px] text-zinc-500 uppercase tracking-widest mb-1.5">State</p>
          <TaskStateBadge state={task.state} />
        </div>

        {/* Actions */}
        {(canClaim || canRelease) && (
          <div className="space-y-3">
            <p className="text-[10px] text-zinc-500 uppercase tracking-widest">Actions</p>

            {canClaim && (
              <div className="space-y-2">
                <input
                  value={workerId}
                  onChange={(e) => setWorkerId(e.target.value)}
                  placeholder="Worker ID"
                  className="w-full rounded-md bg-zinc-800 border border-zinc-700 text-zinc-200 text-xs px-3 py-2 focus:outline-none focus:ring-1 focus:ring-indigo-500 placeholder-zinc-600"
                />
                <button
                  onClick={() => onClaim(task.task_id)}
                  disabled={claiming || !workerId.trim()}
                  className="w-full flex items-center justify-center gap-2 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-white text-xs px-3 py-2 transition-colors"
                >
                  {claiming ? (
                    <RefreshCw size={12} className="animate-spin" />
                  ) : (
                    <Play size={12} />
                  )}
                  Claim task
                </button>
              </div>
            )}

            {canRelease && (
              <button
                onClick={() => onRelease(task.task_id)}
                disabled={releasing}
                className="w-full flex items-center justify-center gap-2 rounded-md bg-amber-700/60 hover:bg-amber-700 disabled:opacity-40 text-amber-200 text-xs px-3 py-2 transition-colors ring-1 ring-amber-700"
              >
                {releasing ? (
                  <RefreshCw size={12} className="animate-spin" />
                ) : (
                  <Unlock size={12} />
                )}
                Release lease
              </button>
            )}
          </div>
        )}

        {/* IDs */}
        <Section title="Identifiers">
          <Field label="Task ID"       value={task.task_id}           mono />
          {task.parent_run_id  && <Field label="Run ID"       value={task.parent_run_id}  mono />}
          {task.parent_task_id && <Field label="Parent Task"  value={task.parent_task_id} mono />}
        </Section>

        {/* Project */}
        <Section title="Project">
          <Field label="Tenant"    value={task.project.tenant_id} />
          <Field label="Workspace" value={task.project.workspace_id} />
          <Field label="Project"   value={task.project.project_id} />
        </Section>

        {/* Lease */}
        {(task.lease_owner || task.lease_expires_at) && (
          <Section title="Lease">
            {task.lease_owner && (
              <Field label="Held by" value={task.lease_owner} mono />
            )}
            {task.lease_expires_at && (
              <Field label="Expires" value={fmtTime(task.lease_expires_at)} />
            )}
          </Section>
        )}

        {/* Failure */}
        {task.failure_class && (
          <Section title="Failure">
            <Field label="Class" value={task.failure_class} />
          </Section>
        )}

        {/* Timestamps */}
        <Section title="Timestamps">
          <Field label="Created" value={fmtTime(task.created_at)} />
          <Field label="Updated" value={fmtTime(task.updated_at)} />
          <Field label="Version" value={String(task.version)} />
        </Section>
      </div>
    </aside>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[10px] text-zinc-500 uppercase tracking-widest mb-2">{title}</p>
      <div className="rounded-lg bg-zinc-800/50 ring-1 ring-zinc-700/50 divide-y divide-zinc-700/40">
        {children}
      </div>
    </div>
  );
}

function Field({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-start justify-between px-3 py-2 gap-3">
      <span className="text-xs text-zinc-500 shrink-0 pt-0.5">{label}</span>
      <span className={clsx("text-xs text-zinc-300 text-right break-all", mono && "font-mono")}>
        {value}
      </span>
    </div>
  );
}

// ── Tasks table ───────────────────────────────────────────────────────────────

interface TableProps {
  tasks: TaskRecord[];
  selectedId: string | null;
  onSelect: (t: TaskRecord) => void;
}

function TasksTable({ tasks, selectedId, onSelect }: TableProps) {
  if (tasks.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-24 gap-3 text-center">
        <Cpu size={36} className="text-zinc-700" />
        <p className="text-sm text-zinc-400">No tasks match this filter</p>
        <p className="text-xs text-zinc-600">Select a different state or clear the filter</p>
      </div>
    );
  }

  return (
    <table className="min-w-full text-sm">
      <thead className="sticky top-0 z-10 bg-zinc-950">
        <tr className="border-b border-zinc-800">
          {[
            { label: "Task ID",     cls: "text-left"  },
            { label: "Run ID",      cls: "text-left"  },
            { label: "State",       cls: "text-left"  },
            { label: "Lease Owner", cls: "text-left"  },
            { label: "Created",     cls: "text-right" },
          ].map(({ label, cls }) => (
            <th key={label} className={clsx(
              "px-4 py-3 text-xs font-medium text-zinc-500 uppercase tracking-widest whitespace-nowrap",
              cls,
            )}>
              {label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody className="divide-y divide-zinc-800/60">
        {tasks.map((task) => {
          const selected = task.task_id === selectedId;
          const leaseExpired = task.lease_expires_at != null && task.lease_expires_at < Date.now();
          return (
            <tr
              key={task.task_id}
              onClick={() => onSelect(task)}
              className={clsx(
                "cursor-pointer transition-colors",
                selected ? "bg-zinc-800" : "hover:bg-zinc-900/60",
              )}
            >
              <td className="px-4 py-3 font-mono text-zinc-300 whitespace-nowrap text-xs">
                <span className="flex items-center gap-1.5">
                  {selected && <ChevronRight size={11} className="text-indigo-400 shrink-0" />}
                  {shortId(task.task_id)}
                </span>
              </td>
              <td className="px-4 py-3 font-mono text-zinc-500 whitespace-nowrap text-xs">
                {task.parent_run_id ? shortId(task.parent_run_id) : <span className="text-zinc-700">—</span>}
              </td>
              <td className="px-4 py-3 whitespace-nowrap">
                <TaskStateBadge state={task.state} compact />
              </td>
              <td className="px-4 py-3 whitespace-nowrap">
                {task.lease_owner ? (
                  <span className={clsx(
                    "inline-flex items-center gap-1.5 text-xs font-mono",
                    leaseExpired ? "text-red-400" : "text-zinc-400",
                  )}>
                    {leaseExpired && <AlertCircle size={11} />}
                    {task.lease_owner}
                  </span>
                ) : (
                  <span className="text-zinc-700 text-xs">—</span>
                )}
              </td>
              <td className="px-4 py-3 text-zinc-500 text-xs whitespace-nowrap text-right">
                <span className="flex items-center justify-end gap-1">
                  <Clock size={11} className="text-zinc-700" />
                  {fmtTime(task.created_at)}
                </span>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

// ── Skeleton ──────────────────────────────────────────────────────────────────

function SkeletonRows() {
  return (
    <div className="divide-y divide-zinc-800/60">
      {Array.from({ length: 10 }).map((_, i) => (
        <div key={i} className="flex items-center gap-4 px-4 py-3.5 animate-pulse">
          <div className="h-3 w-36 rounded bg-zinc-800" />
          <div className="h-3 w-28 rounded bg-zinc-800" />
          <div className="h-5 w-20 rounded-full bg-zinc-800" />
          <div className="h-3 w-24 rounded bg-zinc-800" />
          <div className="ml-auto h-3 w-32 rounded bg-zinc-800" />
        </div>
      ))}
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function TasksPage() {
  const qc = useQueryClient();
  const [stateFilter, setStateFilter] = useState<TaskState | "all">("all");
  const [selectedTask, setSelectedTask] = useState<TaskRecord | null>(null);

  // Fetch all tasks
  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["tasks"],
    queryFn: () => defaultApi.getAllTasks({ limit: 500 }),
    refetchInterval: 15_000,
    select: (tasks) =>
      [...tasks].sort((a, b) => b.created_at - a.created_at),
  });

  // Claim mutation
  const [claimWorker] = useState("operator-1");
  const claimMut = useMutation({
    mutationFn: ({ taskId, workerId }: { taskId: string; workerId: string }) =>
      defaultApi.claimTask(taskId, workerId),
    onSuccess: (updated) => {
      void qc.invalidateQueries({ queryKey: ["tasks"] });
      setSelectedTask(updated);
    },
  });

  // Release mutation
  const releaseMut = useMutation({
    mutationFn: (taskId: string) => defaultApi.releaseLease(taskId),
    onSuccess: (updated) => {
      void qc.invalidateQueries({ queryKey: ["tasks"] });
      setSelectedTask(updated);
    },
  });

  const tasks = data ?? [];
  const filtered =
    stateFilter === "all" ? tasks : tasks.filter((t) => t.state === stateFilter);

  const leasedCount = tasks.filter((t) => t.state === "leased").length;
  const queuedCount = tasks.filter((t) => t.state === "queued").length;

  function handleSelect(t: TaskRecord) {
    setSelectedTask((prev) => (prev?.task_id === t.task_id ? null : t));
  }

  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center p-8">
        <ServerCrash size={40} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load tasks</p>
        <p className="text-sm text-zinc-500">
          {error instanceof Error ? error.message : "Unknown error"}
        </p>
        <button
          onClick={() => void refetch()}
          className="mt-2 px-4 py-2 rounded-lg bg-zinc-800 text-zinc-300 text-sm hover:bg-zinc-700 transition-colors"
        >
          Retry
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* ── Toolbar ───────────────────────────────────────────────────── */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-800 shrink-0">
        <ListChecks size={15} className="text-indigo-400 shrink-0" />
        <h2 className="text-sm font-semibold text-zinc-200">
          Tasks
          {!isLoading && (
            <span className="ml-2 text-xs text-zinc-500 font-normal">
              {filtered.length}
              {stateFilter !== "all" ? ` / ${tasks.length} total` : ""}
            </span>
          )}
        </h2>

        {/* Quick-stat pills */}
        {!isLoading && (
          <>
            {queuedCount > 0 && (
              <button
                onClick={() => setStateFilter("queued")}
                className="inline-flex items-center gap-1 rounded-full bg-zinc-800 ring-1 ring-zinc-700 text-zinc-400 text-[10px] px-2 py-0.5 hover:ring-indigo-600 hover:text-indigo-300 transition-colors"
              >
                {queuedCount} queued
              </button>
            )}
            {leasedCount > 0 && (
              <button
                onClick={() => setStateFilter("leased")}
                className="inline-flex items-center gap-1 rounded-full bg-blue-950 ring-1 ring-blue-800 text-blue-300 text-[10px] px-2 py-0.5 hover:ring-blue-500 transition-colors"
              >
                {leasedCount} leased
              </button>
            )}
          </>
        )}

        {/* State filter */}
        <select
          value={stateFilter}
          onChange={(e) => setStateFilter(e.target.value as TaskState | "all")}
          className="rounded-md bg-zinc-800 border border-zinc-700 text-zinc-300 text-xs px-2.5 py-1.5 focus:outline-none focus:ring-1 focus:ring-indigo-500"
        >
          <option value="all">All states</option>
          {ALL_TASK_STATES.map((s) => (
            <option key={s} value={s}>{TASK_STATE_LABELS[s]}</option>
          ))}
        </select>

        <button
          onClick={() => void refetch()}
          disabled={isFetching}
          className="ml-auto flex items-center gap-1.5 rounded-md bg-zinc-800 border border-zinc-700 text-zinc-400 text-xs px-2.5 py-1.5 hover:text-zinc-200 hover:bg-zinc-700 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={12} className={clsx(isFetching && "animate-spin")} />
          Refresh
        </button>
      </div>

      {/* ── Content: table + detail panel ─────────────────────────────── */}
      <div className="flex flex-1 overflow-hidden">
        {/* Table */}
        <div className={clsx("flex-1 overflow-y-auto", selectedTask && "border-r border-zinc-800")}>
          {isLoading
            ? <SkeletonRows />
            : <TasksTable tasks={filtered} selectedId={selectedTask?.task_id ?? null} onSelect={handleSelect} />
          }
        </div>

        {/* Detail panel */}
        {selectedTask && (
          <DetailPanel
            task={selectedTask}
            onClose={() => setSelectedTask(null)}
            onClaim={(id) => claimMut.mutate({ taskId: id, workerId: claimWorker })}
            onRelease={(id) => releaseMut.mutate(id)}
            claiming={claimMut.isPending}
            releasing={releaseMut.isPending}
          />
        )}
      </div>
    </div>
  );
}

export default TasksPage;
