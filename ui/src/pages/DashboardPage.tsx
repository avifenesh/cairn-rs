import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  CheckCircle2,
  Clock,
  ServerCrash,
} from "lucide-react";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { EventLog } from "../components/EventLog";
import { defaultApi } from "../lib/api";
import type { StatCardVariant } from "../components/StatCard";

// ── Section header ────────────────────────────────────────────────────────────

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-3">
      {children}
    </p>
  );
}

// ── Panel shell ───────────────────────────────────────────────────────────────

function Panel({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={clsx("bg-zinc-900 border border-zinc-800 rounded-lg p-4", className)}>
      {children}
    </div>
  );
}

// ── Critical events ───────────────────────────────────────────────────────────

function CriticalEvents({ events }: { events: string[] }) {
  return (
    <Panel>
      <div className="flex items-center justify-between mb-3">
        <SectionLabel>Critical events</SectionLabel>
        <span className="text-[11px] text-zinc-600">{events.length}</span>
      </div>
      {events.length === 0 ? (
        <div className="flex items-center gap-2 py-4 justify-center text-zinc-600">
          <CheckCircle2 size={14} className="text-emerald-600" />
          <span className="text-[13px]">No critical events</span>
        </div>
      ) : (
        <ul className="space-y-1.5">
          {events.map((evt, i) => (
            <li key={i} className="flex items-start gap-2 rounded bg-zinc-800/50 px-3 py-2">
              <AlertTriangle size={12} className="mt-0.5 shrink-0 text-amber-500" />
              <span className="text-[13px] text-zinc-300 break-words">{evt}</span>
            </li>
          ))}
        </ul>
      )}
    </Panel>
  );
}

// ── Degraded banner ───────────────────────────────────────────────────────────

function DegradedBanner({ components }: { components: string[] }) {
  if (components.length === 0) return null;
  return (
    <div className="border border-amber-800/50 bg-amber-500/5 rounded-lg px-4 py-3 flex items-start gap-3">
      <AlertTriangle size={14} className="text-amber-500 mt-0.5 shrink-0" />
      <div>
        <p className="text-[13px] font-medium text-amber-400">
          {components.length} degraded component{components.length > 1 ? 's' : ''}
        </p>
        <p className="text-[11px] text-amber-600 mt-0.5">
          {components.join(', ')}
        </p>
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function DashboardPage() {
  const { data, isLoading, isError, error, dataUpdatedAt } = useQuery({
    queryKey: ["dashboard"],
    queryFn: () => defaultApi.getDashboard(),
    refetchInterval: 15_000,
  });

  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3 text-center p-8">
        <ServerCrash size={32} className="text-red-500" />
        <p className="text-[13px] font-medium text-zinc-300">Failed to load dashboard</p>
        <p className="text-[13px] text-zinc-600">
          {error instanceof Error ? error.message : "Unknown error"}
        </p>
      </div>
    );
  }

  const runs      = data?.active_runs ?? 0;
  const tasks     = data?.active_tasks ?? 0;
  const approvals = data?.pending_approvals ?? 0;
  const failed    = data?.failed_runs_24h ?? 0;

  const runVariant:  StatCardVariant = runs      > 0 ? "info"    : "default";
  const taskVariant: StatCardVariant = tasks     > 0 ? "info"    : "default";
  const aprVariant:  StatCardVariant = approvals > 0 ? "warning" : "default";
  const failVariant: StatCardVariant = failed    > 0 ? "danger"  : "success";

  const updatedAt = dataUpdatedAt ? new Date(dataUpdatedAt).toLocaleTimeString() : null;

  return (
    <div className="h-full overflow-y-auto bg-zinc-950">
      <div className="max-w-5xl mx-auto px-6 py-6 space-y-6">

        {/* Header row */}
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-[13px] font-medium text-zinc-200">Overview</h2>
            <p className="text-[11px] text-zinc-600 mt-0.5">Real-time deployment status</p>
          </div>
          <div className="flex items-center gap-3">
            {data && (
              <span className={clsx(
                "inline-flex items-center gap-1.5 rounded px-2 py-1 text-[11px] font-medium",
                data.system_healthy
                  ? "bg-emerald-500/10 text-emerald-400"
                  : "bg-red-500/10 text-red-400",
              )}>
                <span className={clsx("w-1.5 h-1.5 rounded-full",
                  data.system_healthy ? "bg-emerald-500" : "bg-red-500")} />
                {data.system_healthy ? "All systems operational" : "Degraded"}
              </span>
            )}
            {updatedAt && (
              <span className="flex items-center gap-1 text-[11px] text-zinc-600">
                <Clock size={11} />
                {updatedAt}
              </span>
            )}
          </div>
        </div>

        {/* Degraded banner */}
        {data && <DegradedBanner components={data.degraded_components ?? []} />}

        {/* Primary metrics */}
        <div>
          <SectionLabel>Activity</SectionLabel>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard label="Active Runs"      value={runs}      variant={runVariant}  loading={isLoading} />
            <StatCard label="Active Tasks"     value={tasks}     variant={taskVariant} loading={isLoading} />
            <StatCard label="Pending Approvals" value={approvals} variant={aprVariant}  loading={isLoading} />
            <StatCard label="Failed (24 h)"    value={failed}    variant={failVariant} loading={isLoading} />
          </div>
        </div>

        {/* Secondary metrics */}
        <div>
          <SectionLabel>Infrastructure</SectionLabel>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard label="Providers"   value={data?.active_providers ?? 0} loading={isLoading} />
            <StatCard label="Plugins"     value={data?.active_plugins   ?? 0} loading={isLoading} />
            <StatCard label="Memory Docs" value={data?.memory_doc_count  ?? 0} loading={isLoading} />
            <StatCard label="Evals Today" value={data?.eval_runs_today   ?? 0} loading={isLoading} />
          </div>
        </div>

        {/* Lower row */}
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          <CriticalEvents events={data?.recent_critical_events ?? []} />
          <Panel>
            <SectionLabel>Live event stream</SectionLabel>
            <EventLog maxEvents={15} />
          </Panel>
        </div>

      </div>
    </div>
  );
}

export default DashboardPage;
