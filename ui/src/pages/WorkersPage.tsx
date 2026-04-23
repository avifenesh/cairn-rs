/**
 * WorkersPage — operator view of the registered coder-agent fleet.
 *
 * Reads the real worker registry (`GET /v1/workers`) and fleet aggregate
 * (`GET /v1/fleet`).  Replaces the earlier approximation that synthesised
 * "workers" by grouping tasks by `lease_owner` — which silently reported
 * zero workers whenever no task was currently leased, even if a dozen
 * workers were registered and heartbeating.
 */

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  RefreshCw, Loader2, Users, Activity, Clock, Cpu, Pause, Play,
} from "lucide-react";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { ErrorFallback } from "../components/ErrorFallback";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import type { WorkerRecord, FleetReport, WorkerStatus } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtRelative(ms: number): string {
  if (!ms) return "never";
  const d = Date.now() - ms;
  if (d < 30_000)    return "just now";
  if (d < 60_000)    return `${Math.floor(d / 1_000)}s ago`;
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)}h ago`;
  return `${Math.floor(d / 86_400_000)}d ago`;
}

function fmtTime(ms: number): string {
  if (!ms) return "never";
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
}

const shortId = (id: string) =>
  id.length > 28 ? `${id.slice(0, 14)}…${id.slice(-8)}` : id;

function statusColor(status: WorkerStatus, isAlive: boolean): string {
  if (status === "suspended") return "text-amber-400 bg-amber-400/10";
  if (status === "offline")   return "text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800";
  // active
  return isAlive
    ? "text-emerald-400 bg-emerald-400/10"
    : "text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800";
}

function statusLabel(status: WorkerStatus, isAlive: boolean): string {
  if (status === "suspended") return "suspended";
  if (status === "offline")   return "offline";
  return isAlive ? "online" : "stale";
}

// ── Worker row ────────────────────────────────────────────────────────────────

function WorkerRow({
  worker,
  even,
  onSuspend,
  onReactivate,
  busy,
}: {
  worker: WorkerRecord;
  even: boolean;
  onSuspend: (id: string) => void;
  onReactivate: (id: string) => void;
  busy: boolean;
}) {
  const isAlive = worker.health?.is_alive ?? false;
  const status  = worker.status;
  const pill    = statusColor(status, isAlive);
  const label   = statusLabel(status, isAlive);
  const lastHb  = worker.health?.last_heartbeat_ms ?? 0;
  const active  = worker.health?.active_task_count ?? 0;

  return (
    <div className={clsx(
      "flex items-center gap-0 h-11 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0",
      even ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50",
    )}>
      {/* Worker id + display name */}
      <div className="flex-1 min-w-0 flex items-center gap-2 pl-4 pr-3">
        <div className={clsx(
          "flex h-6 w-6 shrink-0 items-center justify-center rounded-full",
          status === "active" && isAlive ? "bg-emerald-500/15" : "bg-gray-100 dark:bg-zinc-800",
        )}>
          <Cpu size={11} className={status === "active" && isAlive ? "text-emerald-400" : "text-gray-400 dark:text-zinc-600"} />
        </div>
        <div className="min-w-0">
          <div className="text-[12px] font-mono text-gray-800 dark:text-zinc-200 truncate" title={worker.worker_id}>
            {shortId(worker.worker_id)}
          </div>
          {worker.display_name && worker.display_name !== worker.worker_id && (
            <div className="text-[10px] text-gray-400 dark:text-zinc-500 truncate" title={worker.display_name}>
              {worker.display_name}
            </div>
          )}
        </div>
      </div>

      {/* Tenant */}
      <div className="w-36 shrink-0 px-2">
        <span className="text-[11px] font-mono text-gray-500 dark:text-zinc-400 truncate block" title={worker.tenant_id}>
          {worker.tenant_id}
        </span>
      </div>

      {/* Status pill */}
      <div className="w-28 shrink-0 px-2">
        <span className={clsx(
          "inline-flex items-center gap-1.5 text-[11px] font-medium rounded-full px-2 py-0.5",
          pill,
        )}>
          <span className={clsx(
            "w-1.5 h-1.5 rounded-full shrink-0",
            status === "active" && isAlive ? "bg-emerald-400 animate-pulse" :
            status === "suspended"         ? "bg-amber-400" :
            status === "offline"           ? "bg-zinc-500" : "bg-zinc-500",
          )} />
          {label}
        </span>
      </div>

      {/* Active tasks */}
      <div className="w-20 shrink-0 px-2">
        <span className="text-[12px] tabular-nums text-gray-700 dark:text-zinc-300">
          {active}
        </span>
      </div>

      {/* Last heartbeat */}
      <div className="w-32 shrink-0 px-2 flex items-center gap-1">
        <Clock size={10} className="text-gray-300 dark:text-zinc-600 shrink-0" />
        <span
          className={clsx(
            "text-[11px] tabular-nums",
            !isAlive && lastHb > 0 ? "text-amber-600" : "text-gray-400 dark:text-zinc-500",
          )}
          title={fmtTime(lastHb)}
        >
          {fmtRelative(lastHb)}
        </span>
      </div>

      {/* Registered */}
      <div className="w-28 shrink-0 px-2">
        <span
          className="text-[11px] tabular-nums text-gray-400 dark:text-zinc-500"
          title={fmtTime(worker.registered_at)}
        >
          {fmtRelative(worker.registered_at)}
        </span>
      </div>

      {/* Actions */}
      <div className="w-44 shrink-0 px-2 flex items-center gap-1.5 justify-end pr-4">
        <a
          href={`#worker/${encodeURIComponent(worker.worker_id)}`}
          className="text-[11px] text-indigo-400 hover:text-indigo-300 hover:underline"
          title="Open worker detail"
        >
          Detail
        </a>
        {status === "suspended" ? (
          <button
            data-testid="reactivate-btn"
            onClick={() => onReactivate(worker.worker_id)}
            disabled={busy}
            className="inline-flex items-center gap-1 text-[11px] text-emerald-400 hover:text-emerald-300 disabled:opacity-40"
            title="Reactivate worker — it will accept claims again"
          >
            <Play size={10} /> Reactivate
          </button>
        ) : (
          <button
            data-testid="suspend-btn"
            onClick={() => onSuspend(worker.worker_id)}
            disabled={busy || status === "offline"}
            className="inline-flex items-center gap-1 text-[11px] text-amber-400 hover:text-amber-300 disabled:opacity-40"
            title="Suspend worker — it will stop accepting new claims"
          >
            <Pause size={10} /> Suspend
          </button>
        )}
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function WorkersPage() {
  const qc    = useQueryClient();
  const toast = useToast();
  const [filter, setFilter] = useState<"all" | "active" | "suspended" | "offline">("all");

  const workersQ = useQuery<WorkerRecord[], Error>({
    queryKey:        ["workers"],
    queryFn:         () => defaultApi.listWorkers({ limit: 500 }),
    refetchInterval: 10_000,
  });

  const fleetQ = useQuery<FleetReport, Error>({
    queryKey:        ["fleet"],
    queryFn:         () => defaultApi.getFleet(),
    refetchInterval: 10_000,
  });

  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: ["workers"] });
    void qc.invalidateQueries({ queryKey: ["fleet"] });
  };

  const suspendM = useMutation({
    mutationFn: (id: string) => defaultApi.suspendWorker(id, "suspended by operator"),
    onSuccess: (_, id) => {
      toast.success(`Worker ${id} suspended.`);
      invalidate();
    },
    onError: (e: Error, _id) => {
      toast.error(`Suspend failed: ${e.message}`);
      invalidate();
    },
  });

  const reactivateM = useMutation({
    mutationFn: (id: string) => defaultApi.reactivateWorker(id),
    onSuccess: (_, id) => {
      toast.success(`Worker ${id} reactivated.`);
      invalidate();
    },
    onError: (e: Error, _id) => {
      toast.error(`Reactivate failed: ${e.message}`);
      invalidate();
    },
  });

  const isLoading  = workersQ.isLoading || fleetQ.isLoading;
  const isFetching = workersQ.isFetching || fleetQ.isFetching;
  const errored    = workersQ.isError ? workersQ.error : fleetQ.isError ? fleetQ.error : null;

  if (errored) {
    return (
      <ErrorFallback
        error={errored}
        resource="workers"
        onRetry={() => {
          void workersQ.refetch();
          void fleetQ.refetch();
        }}
      />
    );
  }

  const workers = workersQ.data ?? [];
  const fleet   = fleetQ.data;

  const visible = filter === "all"
    ? workers
    : workers.filter(w => w.status === filter);

  // Prefer backend fleet aggregates when available; fall back to derived counts
  // so the summary strip still renders before /v1/fleet resolves.
  const total   = fleet?.total   ?? workers.length;
  const active  = fleet?.active  ?? workers.filter(w => w.status === "active").length;
  const healthy = fleet?.healthy ?? workers.filter(w => w.health?.is_alive).length;
  const suspended = workers.filter(w => w.status === "suspended").length;

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <Users size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Workers
          {!isLoading && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal">
              {visible.length}
              {filter !== "all" && ` / ${total} total`}
            </span>
          )}
        </span>

        {/* Filter */}
        <div className="flex items-center rounded border border-gray-200 dark:border-zinc-700 overflow-hidden ml-2">
          {(["all", "active", "suspended", "offline"] as const).map(f => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={clsx(
                "px-2.5 py-1 text-[11px] capitalize transition-colors",
                f !== "all" && "border-l border-gray-200 dark:border-zinc-700",
                filter === f
                  ? "bg-gray-200 dark:bg-zinc-700 text-gray-800 dark:text-zinc-200"
                  : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300",
              )}
            >
              {f}
            </button>
          ))}
        </div>

        <button
          onClick={() => {
            void workersQ.refetch();
            void fleetQ.refetch();
          }}
          disabled={isFetching}
          className="ml-auto flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {/* Fleet summary strip */}
      {!isLoading && (
        <div className="grid grid-cols-2 gap-3 px-4 py-4 border-b border-gray-200 dark:border-zinc-800 shrink-0 lg:grid-cols-4">
          <StatCard
            label="Total Workers"
            value={total}
            description="registered in this tenant"
            variant="info"
          />
          <StatCard
            label="Active"
            value={active}
            description={active > 0 ? "accepting claims" : "none accepting"}
            variant="success"
          />
          <StatCard
            label="Healthy"
            value={healthy}
            description="heartbeat within TTL"
            variant={healthy === total ? "success" : "warning"}
          />
          <StatCard
            label="Suspended"
            value={suspended}
            description={suspended > 0 ? "paused by operator" : "none paused"}
            variant={suspended > 0 ? "warning" : "default"}
          />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading worker registry…</span>
          </div>
        ) : visible.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
              <Users size={24} className="text-gray-400 dark:text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">
              {total === 0 ? "No workers registered" : `No ${filter} workers`}
            </p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-sm">
              {total === 0
                ? "Cairn workers register on startup via POST /v1/workers/register. Check that at least one cairn-sdk worker is running and pointed at this control plane."
                : "Try switching the filter to 'all'."}
            </p>
          </div>
        ) : (
          <div className="min-w-[960px]">
            {/* Column headers */}
            <div className="flex items-center h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 sticky top-0">
              <div className="flex-1 pl-4 pr-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Worker</span>
              </div>
              <div className="w-36 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Tenant</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Status</span>
              </div>
              <div className="w-20 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Active</span>
              </div>
              <div className="w-32 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Last Heartbeat</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Registered</span>
              </div>
              <div className="w-44 shrink-0 px-2 pr-4 text-right">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Actions</span>
              </div>
            </div>

            {visible.map((worker, i) => (
              <WorkerRow
                key={worker.worker_id}
                worker={worker}
                even={i % 2 === 0}
                onSuspend={(id) => suspendM.mutate(id)}
                onReactivate={(id) => reactivateM.mutate(id)}
                busy={suspendM.isPending || reactivateM.isPending}
              />
            ))}
          </div>
        )}
      </div>

      {/* Data source note */}
      {!isLoading && total > 0 && (
        <div className="flex items-center gap-1.5 px-4 py-2 border-t border-gray-200 dark:border-zinc-800 shrink-0">
          <Activity size={10} className="text-gray-300 dark:text-zinc-600 shrink-0" />
          <span className="text-[10px] text-gray-300 dark:text-zinc-600">
            Live from <code className="font-mono">GET /v1/workers</code> + <code className="font-mono">GET /v1/fleet</code> — refreshes every 10 s.
          </span>
        </div>
      )}
    </div>
  );
}

export default WorkersPage;
