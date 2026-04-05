import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  CheckCircle2,
  Clock,
  Database,
  Cpu,
  Zap,
  Activity,
} from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { EventLog } from "../components/EventLog";
import { defaultApi } from "../lib/api";
import type { StatCardVariant } from "../components/StatCard";
import type { HealthCheckEntry } from "../lib/types";

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

// ── System health card ────────────────────────────────────────────────────────

const STATUS_DOT: Record<string, string> = {
  healthy:      "bg-emerald-500",
  degraded:     "bg-amber-500 animate-pulse",
  unhealthy:    "bg-red-500 animate-pulse",
  unconfigured: "bg-zinc-600",
};

const STATUS_LABEL: Record<string, string> = {
  healthy:      "Healthy",
  degraded:     "Degraded",
  unhealthy:    "Unhealthy",
  unconfigured: "Not configured",
};

function CheckRow({
  icon: Icon, label, check,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  label: string;
  check: HealthCheckEntry;
}) {
  return (
    <div className="flex items-center gap-3 py-2 border-b border-zinc-800/60 last:border-0">
      <Icon size={13} className="text-zinc-600 shrink-0" />
      <span className="text-[12px] text-zinc-400 flex-1">{label}</span>
      <div className="flex items-center gap-2">
        {check.models !== undefined && (
          <span className="text-[10px] text-zinc-600 font-mono">{check.models} model{check.models !== 1 ? "s" : ""}</span>
        )}
        {check.latency_ms !== undefined && (
          <span className="text-[10px] text-zinc-700 font-mono tabular-nums">{check.latency_ms}ms</span>
        )}
        <span className="flex items-center gap-1.5 text-[11px] font-medium">
          <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0", STATUS_DOT[check.status] ?? "bg-zinc-600")} />
          <span className={clsx(
            check.status === "healthy"      ? "text-emerald-400" :
            check.status === "unconfigured" ? "text-zinc-600"    :
            check.status === "degraded"     ? "text-amber-400"   : "text-red-400",
          )}>
            {STATUS_LABEL[check.status] ?? check.status}
          </span>
        </span>
      </div>
    </div>
  );
}

function UsageBar({ value, max, color }: { value: number; max: number; color: string }) {
  const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
  return (
    <div className="flex items-center gap-2">
      <div className="flex-1 h-1.5 rounded-full bg-zinc-800 overflow-hidden">
        <div className={clsx("h-full rounded-full transition-all", color)} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-[10px] text-zinc-600 tabular-nums font-mono w-8 text-right shrink-0">
        {pct.toFixed(0)}%
      </span>
    </div>
  );
}

function SystemHealthCard() {
  const { data, isLoading } = useQuery({
    queryKey: ["detailed-health"],
    queryFn: () => defaultApi.getDetailedHealth(),
    refetchInterval: 30_000,
    retry: false,
  });

  const fmtUptime = (secs: number) => {
    if (secs < 60)    return `${secs}s`;
    if (secs < 3600)  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
  };

  return (
    <Panel>
      <div className="flex items-center justify-between mb-3">
        <SectionLabel>System Health</SectionLabel>
        {data && (
          <span className={clsx(
            "inline-flex items-center gap-1.5 text-[10px] font-medium rounded px-1.5 py-0.5",
            data.status === "healthy"
              ? "bg-emerald-500/10 text-emerald-400"
              : data.status === "degraded"
              ? "bg-amber-500/10 text-amber-400"
              : "bg-red-500/10 text-red-400",
          )}>
            <span className={clsx("w-1.5 h-1.5 rounded-full", STATUS_DOT[data.status])} />
            {data.status}
          </span>
        )}
      </div>

      {isLoading ? (
        <div className="space-y-2 animate-pulse">
          {[1, 2, 3].map(i => <div key={i} className="h-8 rounded bg-zinc-800" />)}
        </div>
      ) : !data ? (
        <p className="text-[12px] text-zinc-600 italic py-2">
          Health endpoint unavailable — upgrade cairn-app to v0.1.1+
        </p>
      ) : (
        <div className="space-y-0">
          <CheckRow icon={Database} label="Store"  check={data.checks.store} />
          <CheckRow icon={Zap}      label="Ollama" check={data.checks.ollama} />
          <CheckRow icon={Activity} label="Events" check={data.checks.event_buffer} />

          {/* Memory usage */}
          {(data.checks.memory.rss_mb ?? 0) > 0 && (
            <div className="pt-2 mt-1 border-t border-zinc-800/60 space-y-1.5">
              <div className="flex items-center gap-2">
                <Cpu size={13} className="text-zinc-600 shrink-0" />
                <span className="text-[12px] text-zinc-400 flex-1">Memory (RSS)</span>
                <span className="text-[11px] text-zinc-500 font-mono tabular-nums">
                  {data.checks.memory.rss_mb} MB
                </span>
              </div>
              <UsageBar
                value={data.checks.memory.rss_mb ?? 0}
                max={512}
                color={
                  (data.checks.memory.rss_mb ?? 0) > 400 ? "bg-red-500" :
                  (data.checks.memory.rss_mb ?? 0) > 256 ? "bg-amber-500" :
                  "bg-emerald-500"
                }
              />
            </div>
          )}

          {/* Uptime + version footer */}
          <div className="flex items-center justify-between pt-2 mt-1 border-t border-zinc-800/60">
            <span className="text-[10px] text-zinc-700 font-mono">
              v{data.version}
            </span>
            <span className="text-[10px] text-zinc-700 font-mono tabular-nums">
              up {fmtUptime(data.uptime_seconds)}
            </span>
          </div>
        </div>
      )}
    </Panel>
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
            <li key={`evt-${i}-${evt.slice(0, 20)}`} className="flex items-start gap-2 rounded bg-zinc-800/50 px-3 py-2">
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
  // Primary: fast real-time counts from /v1/stats (5s refresh).
  const { data: stats, dataUpdatedAt: statsUpdatedAt } = useQuery({
    queryKey: ["stats"],
    queryFn: () => defaultApi.getStats(),
    refetchInterval: 5_000,
    retry: false,
  });

  // Fallback: richer dashboard payload (15s refresh) for fields not in stats.
  const { data, isLoading, isError, error, dataUpdatedAt, refetch: refetchDashboard } = useQuery({
    queryKey: ["dashboard"],
    queryFn: () => defaultApi.getDashboard(),
    refetchInterval: 15_000,
  });

  // Seed the event log with the most recent events before SSE connects.
  const { data: recentEventsData } = useQuery({
    queryKey: ["recent-events"],
    queryFn: () => defaultApi.getRecentEvents(50),
    staleTime: 30_000,
    retry: false,
  });

  if (isError && !stats) {
    return (
      <ErrorFallback
        error={error}
        resource="dashboard"
        onRetry={() => void refetchDashboard()}
      />
    );
  }

  // Prefer stats counts (faster); fall back to dashboard payload.
  const runs      = stats?.active_runs      ?? data?.active_runs      ?? 0;
  const tasks     = stats?.total_tasks      ?? data?.active_tasks     ?? 0;
  const approvals = stats?.pending_approvals ?? data?.pending_approvals ?? 0;
  const failed    = data?.failed_runs_24h ?? 0;

  const runVariant:  StatCardVariant = runs      > 0 ? "info"    : "default";
  const taskVariant: StatCardVariant = tasks     > 0 ? "info"    : "default";
  const aprVariant:  StatCardVariant = approvals > 0 ? "warning" : "default";
  const failVariant: StatCardVariant = failed    > 0 ? "danger"  : "success";

  const latestUpdate = statsUpdatedAt || dataUpdatedAt;
  const updatedAt = latestUpdate ? new Date(latestUpdate).toLocaleTimeString() : null;

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
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
          <CriticalEvents events={data?.recent_critical_events ?? []} />
          <SystemHealthCard />
          <Panel>
            <SectionLabel>Live event stream</SectionLabel>
            <EventLog
              initialEvents={recentEventsData ?? []}
              maxEvents={50}
            />
          </Panel>
        </div>

      </div>
    </div>
  );
}

export default DashboardPage;
