import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Loader2 } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { StateBadge } from "../components/StateBadge";
import { DataTable } from "../components/DataTable";
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
    <ErrorFallback error={error} resource="tasks" onRetry={() => void refetch()} />
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
          : (
          <DataTable<TaskRecord>
            data={filtered}
            columns={[
              { key: 'task_id',    header: 'Task ID',   render: r => <span className="font-mono text-xs text-zinc-300 whitespace-nowrap" title={r.task_id}>{shortId(r.task_id)}</span>,               sortValue: r => r.task_id },
              { key: 'run',        header: 'Run',        render: r => r.parent_run_id ? <span className="font-mono text-[11px] text-zinc-500 whitespace-nowrap" title={r.parent_run_id}>{shortId(r.parent_run_id)}</span> : <span className="text-zinc-700">—</span> },
              { key: 'state',      header: 'Status',     render: r => <StateBadge state={r.state as Parameters<typeof StateBadge>[0]["state"]} compact />, sortValue: r => r.state },
              { key: 'worker',     header: 'Worker',     render: r => r.lease_owner ? <span className="font-mono text-[11px] text-zinc-400 whitespace-nowrap">{shortId(r.lease_owner)}</span> : <span className="text-zinc-700">—</span> },
              { key: 'queued_at',  header: 'Queued',     render: r => <span className="text-[11px] text-zinc-500 tabular-nums whitespace-nowrap" title={fmtTime(r.created_at)}>{fmtRelative(r.created_at)}</span>,   sortValue: r => r.created_at },
              { key: 'started_at', header: 'Started At', render: r => r.lease_expires_at ? <span className="text-[11px] text-zinc-400 tabular-nums whitespace-nowrap">{fmtTime(r.updated_at)}</span> : <span className="text-zinc-700">—</span>, sortValue: r => r.updated_at },
              { key: 'actions',    header: '',            render: r => <RowActions task={r} /> },
            ]}
            filterFn={(r, q) => r.task_id.includes(q) || r.state.includes(q) || (r.parent_run_id ?? '').includes(q) || (r.lease_owner ?? '').includes(q)}
            csvRow={r => [r.task_id, r.parent_run_id ?? '', r.state, r.lease_owner ?? '', r.created_at, r.updated_at]}
            csvHeaders={['Task ID', 'Run ID', 'State', 'Worker', 'Queued At', 'Updated At']}
            filename="tasks"
            emptyText="No tasks match this filter"
          />
        )
        }
      </div>
    </div>
  );
}

export default TasksPage;
