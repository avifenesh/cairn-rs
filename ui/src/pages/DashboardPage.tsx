import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  CircleDot,
  Clock,
  Cpu,
  ListChecks,
  ServerCrash,
  XCircle,
  Zap,
} from "lucide-react";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { defaultApi } from "../lib/api";
import type { StatCardVariant } from "../components/StatCard";

// ── Health badge ──────────────────────────────────────────────────────────────

function HealthBadge({ healthy }: { healthy: boolean }) {
  return (
    <span
      className={clsx(
        "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-semibold",
        healthy
          ? "bg-emerald-950 text-emerald-400 ring-1 ring-emerald-800"
          : "bg-red-950 text-red-400 ring-1 ring-red-800"
      )}
    >
      {healthy ? (
        <CheckCircle2 size={12} strokeWidth={2.5} />
      ) : (
        <XCircle size={12} strokeWidth={2.5} />
      )}
      {healthy ? "Healthy" : "Degraded"}
    </span>
  );
}

// ── Provider health row ───────────────────────────────────────────────────────

function ProviderHealthPanel({ count }: { count: number }) {
  return (
    <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-sm font-semibold text-zinc-200 flex items-center gap-2">
          <Cpu size={15} className="text-zinc-400" />
          Provider Health
        </h2>
        <span className="text-xs text-zinc-500">{count} active</span>
      </div>

      {count === 0 ? (
        <div className="flex flex-col items-center justify-center py-6 text-center">
          <Cpu size={28} className="text-zinc-700 mb-2" />
          <p className="text-sm text-zinc-500">No providers configured</p>
          <p className="text-xs text-zinc-600 mt-1">
            Add a provider binding to start routing LLM calls
          </p>
        </div>
      ) : (
        <p className="text-sm text-zinc-400">{count} provider(s) reporting health</p>
      )}
    </div>
  );
}

// ── Critical events list ──────────────────────────────────────────────────────

function CriticalEventsList({ events }: { events: string[] }) {
  return (
    <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-sm font-semibold text-zinc-200 flex items-center gap-2">
          <AlertTriangle size={15} className="text-amber-400" />
          Recent Critical Events
        </h2>
        <span className="text-xs text-zinc-500">{events.length} events</span>
      </div>

      {events.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-6 text-center">
          <CheckCircle2 size={28} className="text-emerald-700 mb-2" />
          <p className="text-sm text-zinc-400">No critical events</p>
          <p className="text-xs text-zinc-600 mt-1">System is running normally</p>
        </div>
      ) : (
        <ul className="space-y-2">
          {events.map((evt, i) => (
            <li
              key={i}
              className="flex items-start gap-2 rounded-lg bg-zinc-800/60 px-3 py-2 text-sm"
            >
              <AlertTriangle size={13} className="mt-0.5 shrink-0 text-amber-400" />
              <span className="text-zinc-300 break-words">{evt}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function DashboardPage() {
  const {
    data: dashboard,
    isLoading,
    isError,
    error,
    dataUpdatedAt,
  } = useQuery({
    queryKey: ["dashboard"],
    queryFn: () => defaultApi.getDashboard(),
    refetchInterval: 15_000, // auto-refresh every 15 s
  });

  // ── Error state ──────────────────────────────────────────────────────────
  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center p-8">
        <ServerCrash size={40} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load dashboard</p>
        <p className="text-sm text-zinc-500">
          {error instanceof Error ? error.message : "Unknown error"}
        </p>
      </div>
    );
  }

  // ── Stat card definitions ────────────────────────────────────────────────
  const activeRunsVariant: StatCardVariant =
    (dashboard?.active_runs ?? 0) > 0 ? "info" : "default";

  const failedVariant: StatCardVariant =
    (dashboard?.failed_runs_24h ?? 0) > 0 ? "danger" : "success";

  const approvalsVariant: StatCardVariant =
    (dashboard?.pending_approvals ?? 0) > 0 ? "warning" : "default";

  const lastUpdated = dataUpdatedAt
    ? new Date(dataUpdatedAt).toLocaleTimeString()
    : null;

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-50 p-6 space-y-6">
      {/* ── Header ─────────────────────────────────────────────────────── */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-zinc-100 flex items-center gap-2">
            <Activity size={20} className="text-blue-400" />
            Operator Dashboard
          </h1>
          <p className="text-xs text-zinc-500 mt-0.5">
            Real-time view of your Cairn deployment
          </p>
        </div>

        <div className="flex items-center gap-3">
          {dashboard && <HealthBadge healthy={dashboard.system_healthy} />}
          {lastUpdated && (
            <span className="text-xs text-zinc-600 flex items-center gap-1">
              <Clock size={11} />
              {lastUpdated}
            </span>
          )}
        </div>
      </div>

      {/* ── Stat cards grid ─────────────────────────────────────────────── */}
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <StatCard
          label="Active Runs"
          value={dashboard?.active_runs ?? 0}
          description={
            (dashboard?.active_runs ?? 0) > 0
              ? `${dashboard!.active_runs} run(s) in progress`
              : "No active runs"
          }
          icon={Zap}
          variant={activeRunsVariant}
          loading={isLoading}
        />
        <StatCard
          label="Active Tasks"
          value={dashboard?.active_tasks ?? 0}
          description={
            (dashboard?.active_tasks ?? 0) > 0
              ? `${dashboard!.active_tasks} task(s) running`
              : "No active tasks"
          }
          icon={ListChecks}
          variant={(dashboard?.active_tasks ?? 0) > 0 ? "info" : "default"}
          loading={isLoading}
        />
        <StatCard
          label="Pending Approvals"
          value={dashboard?.pending_approvals ?? 0}
          description={
            (dashboard?.pending_approvals ?? 0) > 0
              ? "Operator action required"
              : "Inbox clear"
          }
          icon={CircleDot}
          variant={approvalsVariant}
          loading={isLoading}
        />
        <StatCard
          label="Failed Runs (24h)"
          value={dashboard?.failed_runs_24h ?? 0}
          description={
            (dashboard?.failed_runs_24h ?? 0) > 0
              ? "Failures in last 24 hours"
              : "No failures today"
          }
          icon={AlertTriangle}
          variant={failedVariant}
          loading={isLoading}
        />
      </div>

      {/* ── Secondary metrics ───────────────────────────────────────────── */}
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <StatCard
          label="Active Providers"
          value={dashboard?.active_providers ?? 0}
          icon={Cpu}
          variant="default"
          loading={isLoading}
        />
        <StatCard
          label="Active Plugins"
          value={dashboard?.active_plugins ?? 0}
          icon={Zap}
          variant="default"
          loading={isLoading}
        />
        <StatCard
          label="Memory Docs"
          value={dashboard?.memory_doc_count ?? 0}
          icon={ListChecks}
          variant="default"
          loading={isLoading}
        />
        <StatCard
          label="Eval Runs Today"
          value={dashboard?.eval_runs_today ?? 0}
          icon={Activity}
          variant="default"
          loading={isLoading}
        />
      </div>

      {/* ── Lower panels ────────────────────────────────────────────────── */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <CriticalEventsList events={dashboard?.recent_critical_events ?? []} />
        <ProviderHealthPanel count={dashboard?.active_providers ?? 0} />
      </div>

      {/* ── Degraded components (only shown when non-empty) ─────────────── */}
      {(dashboard?.degraded_components?.length ?? 0) > 0 && (
        <div className="rounded-xl bg-amber-950/40 ring-1 ring-amber-800/50 p-5">
          <h2 className="text-sm font-semibold text-amber-300 flex items-center gap-2 mb-3">
            <AlertTriangle size={15} />
            Degraded Components
          </h2>
          <ul className="flex flex-wrap gap-2">
            {dashboard!.degraded_components.map((c) => (
              <li
                key={c}
                className="rounded-md bg-amber-900/50 px-2.5 py-1 text-xs font-medium text-amber-300"
              >
                {c}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

export default DashboardPage;
