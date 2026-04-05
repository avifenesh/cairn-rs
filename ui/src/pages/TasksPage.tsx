import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Loader2, ServerCrash, Inbox } from "lucide-react";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import type { TaskRecord, TaskState } from "../lib/types";

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const ACTIVE_STATES: TaskState[] = [
  "queued", "leased", "running", "paused", "waiting_dependency",
];

const ALL_STATES: (TaskState | "all")[] = [
  "all", "queued", "leased", "running", "completed", "failed",
  "canceled", "paused", "waiting_dependency", "retryable_failed", "dead_lettered",
];

// ── Row actions ────────────────────────────────────────────────────────────────

function RowActions({ task }: { task: TaskRecord }) {
  const qc    = useQueryClient();
  const toast = useToast();

  const claim = useMutation({
    mutationFn: () => defaultApi.claimTask(task.task_id, "operator", 60_000),
    onSuccess: () => { toast.success("Task claimed."); void qc.invalidateQueries({ queryKey: ["tasks"] }); },
    onError:   () => toast.error("Failed to claim task."),
  });

  const release = useMutation({
    mutationFn: () => defaultApi.releaseLease(task.task_id),
    onSuccess: () => { toast.success("Lease released."); void qc.invalidateQueries({ queryKey: ["tasks"] }); },
    onError:   () => toast.error("Failed to release lease."),
  });

  const canClaim   = task.state === "queued";
  const canRelease = (task.state === "leased" || task.state === "running") && !!task.lease_owner;

  if (!canClaim && !canRelease) return null;

  return (
    <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
      {canClaim && (
        <button
          onClick={e => { e.stopPropagation(); claim.mutate(); }}
          disabled={claim.isPending}
          className="px-2 py-0.5 rounded text-[11px] font-medium bg-indigo-900/60 text-indigo-300
                     hover:bg-indigo-900 border border-indigo-800/50 transition-colors disabled:opacity-40"
        >
          {claim.isPending ? <Loader2 size={10} className="animate-spin inline" /> : "Claim"}
        </button>
      )}
      {canRelease && (
        <button
          onClick={e => { e.stopPropagation(); release.mutate(); }}
          disabled={release.isPending}
          className="px-2 py-0.5 rounded text-[11px] font-medium bg-zinc-800 text-zinc-400
                     hover:bg-zinc-700 border border-zinc-700 transition-colors disabled:opacity-40"
        >
          {release.isPending ? <Loader2 size={10} className="animate-spin inline" /> : "Release"}
        </button>
      )}
    </div>
  );
}

// ── Table ─────────────────────────────────────────────────────────────────────

const TH = ({ ch, right }: { ch: React.ReactNode; right?: boolean }) => (
  <th className={clsx(
    "px-3 py-2 text-[11px] font-medium text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-zinc-800",
    right ? "text-right" : "text-left",
  )}>
    {ch}
  </th>
);

function TasksTable({ tasks }: { tasks: TaskRecord[] }) {
  if (tasks.length === 0) return (
    <div className="flex flex-col items-center justify-center py-16 gap-2 text-zinc-700">
      <Inbox size={26} />
      <p className="text-[13px]">No tasks match this filter</p>
    </div>
  );

  return (
    <table className="min-w-full text-[13px]">
      <thead className="bg-zinc-900 sticky top-0 z-10">
        <tr>
          <TH ch="Task ID" />
          <TH ch="Run" />
          <TH ch="Status" />
          <TH ch="Worker" />
          <TH ch="Queued At" />
          <TH ch="Started At" />
          <TH ch="" right />
        </tr>
      </thead>
      <tbody className="divide-y divide-zinc-800/50">
        {tasks.map((task, i) => (
          <tr key={task.task_id}
            className={clsx(
              "group transition-colors",
              i % 2 === 0 ? "bg-zinc-900" : "bg-[#111113]",
              "hover:bg-zinc-800/70",
            )}>
            <td className="px-3 py-1.5 font-mono text-zinc-300 whitespace-nowrap">
              {shortId(task.task_id)}
            </td>
            <td className="px-3 py-1.5 font-mono text-zinc-500 whitespace-nowrap text-[12px]">
              {task.parent_run_id
                ? shortId(task.parent_run_id)
                : <span className="text-zinc-700">—</span>}
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap">
              <StateBadge state={task.state as Parameters<typeof StateBadge>[0]["state"]} compact />
            </td>
            <td className="px-3 py-1.5 font-mono text-[12px] whitespace-nowrap">
              {task.lease_owner
                ? <span className="text-zinc-400">{shortId(task.lease_owner)}</span>
                : <span className="text-zinc-700">—</span>}
            </td>
            <td className="px-3 py-1.5 text-zinc-500 whitespace-nowrap tabular-nums">
              {fmtTime(task.created_at)}
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap tabular-nums">
              {task.lease_expires_at
                ? <span className="text-zinc-400">{fmtTime(task.updated_at)}</span>
                : <span className="text-zinc-700">—</span>}
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap">
              <RowActions task={task} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function TasksPage() {
  const [filter, setFilter] = useState<TaskState | "all">("all");

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["tasks"],
    queryFn: () => defaultApi.getAllTasks({ limit: 500 }),
    refetchInterval: 15_000,
  });

  const tasks     = data ?? [];
  const filtered  = filter === "all" ? tasks : tasks.filter(t => t.state === filter);
  const activeCnt = tasks.filter(t => ACTIVE_STATES.includes(t.state)).length;

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-zinc-300 font-medium">Failed to load tasks</p>
      <p className="text-[12px] text-zinc-500">{error instanceof Error ? error.message : "Unknown"}</p>
      <button onClick={() => refetch()}
        className="mt-1 px-3 py-1.5 rounded bg-zinc-800 text-zinc-300 text-[12px] hover:bg-zinc-700 transition-colors">
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-zinc-800 shrink-0 bg-zinc-900">
        <span className="text-[13px] font-medium text-zinc-200">
          Tasks
          {!isLoading && (
            <span className="ml-2 text-[12px] text-zinc-500 font-normal">
              {filtered.length}
              {filter !== "all" && ` / ${tasks.length} total`}
              {activeCnt > 0 && filter === "all" && (
                <span className="ml-1.5 text-indigo-400">{activeCnt} active</span>
              )}
            </span>
          )}
        </span>

        <select
          value={filter}
          onChange={e => setFilter(e.target.value as TaskState | "all")}
          className="ml-auto rounded border border-zinc-700 bg-zinc-800 text-[12px] text-zinc-300
                     px-2 py-1 focus:outline-none focus:border-indigo-500 transition-colors"
        >
          {ALL_STATES.map(s => (
            <option key={s} value={s}>
              {s === "all" ? "All states" : s.replace(/_/g, " ")}
            </option>
          ))}
        </select>

        <button onClick={() => refetch()} disabled={isFetching}
          className="flex items-center gap-1 text-[12px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40 transition-colors">
          <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading
          ? <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-600">
              <Loader2 size={16} className="animate-spin" />
              <span className="text-[13px]">Loading…</span>
            </div>
          : <TasksTable tasks={filtered} />
        }
      </div>
    </div>
  );
}

export default TasksPage;
