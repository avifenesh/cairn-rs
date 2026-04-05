import React, { useCallback, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  CheckCircle2,
  Clock,
  Database,
  Cpu,
  Zap,
  Activity,
  TrendingUp,
  TrendingDown,
  Coins,
  Play,
  Pause,
  Timer,
  Radio,
  RefreshCw,
  Download,
  Printer,
  ChevronDown,
} from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { EventLog } from "../components/EventLog";
import { MiniChart } from "../components/MiniChart";
import { BarChart } from "../components/BarChart";
import { defaultApi } from "../lib/api";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";
import type { StatCardVariant } from "../components/StatCard";
import type { HealthCheckEntry, RunRecord } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtMicros(micros: number): string {
  if (micros === 0) return "$0.00";
  const usd = micros / 1_000_000;
  if (usd < 0.001) return `$${(usd * 1000).toFixed(3)}m`;
  if (usd < 1)     return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000)     return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function fmtElapsed(createdAtMs: number): string {
  const s = Math.floor((Date.now() - createdAtMs) / 1_000);
  if (s < 60)      return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60)      return `${m}m ${s % 60}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

function fmtUptime(secs: number) {
  if (secs < 60)   return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  const h = Math.floor(secs / 3600);
  return `${h}h ${Math.floor((secs % 3600) / 60)}m`;
}

const ACTIVE_STATES = new Set(["running", "pending", "paused", "waiting_approval", "waiting_dependency"]);

function runStateColors(state: string): string {
  switch (state) {
    case "running":           return "text-emerald-400 bg-emerald-400/10";
    case "paused":            return "text-amber-400  bg-amber-400/10";
    case "waiting_approval":  return "text-purple-400 bg-purple-400/10";
    case "pending":           return "text-sky-400    bg-sky-400/10";
    default:                  return "text-zinc-500   bg-zinc-800";
  }
}

// ── Primitives ────────────────────────────────────────────────────────────────

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-3">
      {children}
    </p>
  );
}

function Panel({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={clsx("bg-zinc-900 border border-zinc-800 rounded-lg p-4", className)}>
      {children}
    </div>
  );
}

function Skeleton({ className }: { className?: string }) {
  return <div className={clsx("rounded bg-zinc-800 animate-pulse", className)} />;
}

// ── Widget: Active Runs ───────────────────────────────────────────────────────

function ActiveRunRow({ run }: { run: RunRecord }) {
  const handleClick = useCallback(() => {
    window.location.hash = `run/${run.run_id}`;
  }, [run.run_id]);

  return (
    <button
      onClick={handleClick}
      className="w-full flex items-center gap-3 px-3 py-2 rounded-md hover:bg-zinc-800/60 transition-colors text-left group"
    >
      {/* State icon */}
      <span className={clsx(
        "shrink-0 flex h-6 w-6 items-center justify-center rounded-full",
        run.state === "running" ? "bg-emerald-500/15" : "bg-zinc-800",
      )}>
        {run.state === "running"
          ? <Play  size={9} className="text-emerald-400 fill-emerald-400" />
          : run.state === "paused"
          ? <Pause size={9} className="text-amber-400" />
          : <Timer size={9} className="text-zinc-500" />
        }
      </span>

      {/* Run id + project */}
      <div className="flex-1 min-w-0">
        <p className="text-[12px] font-mono text-zinc-200 truncate group-hover:text-zinc-100">
          {run.run_id}
        </p>
        <p className="text-[10px] text-zinc-600 truncate font-mono">
          {run.project.tenant_id}/{run.project.project_id}
        </p>
      </div>

      {/* State badge + elapsed */}
      <div className="flex flex-col items-end gap-0.5 shrink-0">
        <span className={clsx(
          "text-[10px] font-medium px-1.5 py-0.5 rounded-full",
          runStateColors(run.state),
        )}>
          {run.state.replace(/_/g, " ")}
        </span>
        <span className="text-[10px] font-mono text-zinc-600 tabular-nums">
          {fmtElapsed(run.created_at)}
        </span>
      </div>
    </button>
  );
}

function ActiveRunsWidget() {
  const { data: runs, isLoading } = useQuery({
    queryKey: ["runs-active"],
    queryFn:  () => defaultApi.getRuns({ limit: 50 }),
    refetchInterval: 5_000,
    select: (rows) => rows.filter(r => ACTIVE_STATES.has(r.state))
                          .sort((a, b) => b.created_at - a.created_at)
                          .slice(0, 8),
  });

  return (
    <Panel className="flex flex-col min-h-[160px]">
      <div className="flex items-center justify-between mb-2">
        <SectionLabel>Active Runs</SectionLabel>
        <span className="text-[10px] text-zinc-600 flex items-center gap-1">
          <Radio size={9} className="text-emerald-500" />
          5s
        </span>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          {[0, 1, 2].map(i => (
            <div key={i} className="flex items-center gap-3 px-3 py-2">
              <Skeleton className="h-6 w-6 rounded-full" />
              <div className="flex-1 space-y-1">
                <Skeleton className="h-3 w-3/4" />
                <Skeleton className="h-2 w-1/2" />
              </div>
              <Skeleton className="h-4 w-16" />
            </div>
          ))}
        </div>
      ) : !runs || runs.length === 0 ? (
        <div className="flex-1 flex flex-col items-center justify-center py-6 gap-2">
          <CheckCircle2 size={18} className="text-emerald-600/50" />
          <p className="text-[12px] text-zinc-600">No active runs</p>
        </div>
      ) : (
        <div className="space-y-0.5">
          {runs.map(run => <ActiveRunRow key={run.run_id} run={run} />)}
        </div>
      )}
    </Panel>
  );
}

// ── Widget: Cost ──────────────────────────────────────────────────────────────

/** Render n CSS-only bars for visual interest (token proportion breakdown). */
function TokenBar({ inputTokens, outputTokens }: { inputTokens: number; outputTokens: number }) {
  const total = inputTokens + outputTokens;
  if (total === 0) {
    return (
      <div className="flex gap-0.5 h-6 items-end">
        {Array.from({ length: 12 }, (_, i) => (
          <div key={i} className="flex-1 rounded-sm bg-zinc-800" style={{ height: "30%" }} />
        ))}
      </div>
    );
  }

  // Synthetic per-bucket bars: distribute total across 12 segments using a
  // slight decay so it resembles a call-distribution histogram.
  const decay = [1, 0.92, 0.87, 0.94, 0.78, 0.85, 0.91, 0.73, 0.88, 0.82, 0.69, 0.76];
  const inPct  = inputTokens  / total;
  const bars = decay.map((d, i) => ({
    h:     Math.max(8, Math.round(d * 80)),
    isIn:  i < Math.round(inPct * 12),
  }));

  return (
    <div className="flex gap-0.5 h-8 items-end">
      {bars.map((b, i) => (
        <div
          key={i}
          className={clsx(
            "flex-1 rounded-sm transition-all",
            b.isIn ? "bg-indigo-500/60" : "bg-sky-500/50",
          )}
          style={{ height: `${b.h}%` }}
        />
      ))}
    </div>
  );
}

function CostWidget() {
  const { data: costs, isLoading, dataUpdatedAt } = useQuery({
    queryKey: ["costs-widget"],
    queryFn:  () => defaultApi.getCosts(),
    refetchInterval: 30_000,
  });

  const { data: prevCosts } = useQuery({
    queryKey: ["costs-widget-prev"],
    queryFn:  () => defaultApi.getCosts(),
    staleTime: Infinity,
    gcTime: 30_000,
    enabled: false,        // seeded from cache; we just compare snapshots
  });

  const trend = costs && prevCosts && prevCosts.total_cost_micros > 0
    ? costs.total_cost_micros > prevCosts.total_cost_micros ? "up" : "down"
    : null;

  return (
    <Panel>
      <div className="flex items-center justify-between mb-2">
        <SectionLabel>Costs</SectionLabel>
        {dataUpdatedAt > 0 && (
          <span className="text-[10px] text-zinc-700 tabular-nums">
            {new Date(dataUpdatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
          </span>
        )}
      </div>

      {isLoading ? (
        <div className="space-y-2">
          <Skeleton className="h-6 w-24" />
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-3 w-3/4" />
        </div>
      ) : !costs ? (
        <p className="text-[12px] text-zinc-600 italic">Unavailable</p>
      ) : (
        <div className="space-y-2">
          {/* Total cost headline */}
          <div className="flex items-baseline gap-2">
            <span className="text-[22px] font-semibold tabular-nums text-zinc-100 leading-none">
              {fmtMicros(costs.total_cost_micros)}
            </span>
            {trend === "up" && <TrendingUp  size={13} className="text-red-400" />}
            {trend === "down" && <TrendingDown size={13} className="text-emerald-400" />}
          </div>

          {/* Sparkline: token distribution */}
          <TokenBar
            inputTokens={costs.total_tokens_in}
            outputTokens={costs.total_tokens_out}
          />

          {/* Legend */}
          <div className="flex items-center justify-between text-[10px] text-zinc-600">
            <span className="flex items-center gap-1">
              <span className="w-2 h-2 rounded-sm bg-indigo-500/60 inline-block" />
              in {fmtTokens(costs.total_tokens_in)}
            </span>
            <span className="flex items-center gap-1">
              <span className="w-2 h-2 rounded-sm bg-sky-500/50 inline-block" />
              out {fmtTokens(costs.total_tokens_out)}
            </span>
            <span className="flex items-center gap-1">
              <Coins size={9} />
              {costs.total_provider_calls.toLocaleString()} calls
            </span>
          </div>
        </div>
      )}
    </Panel>
  );
}

// ── Widget: Provider status ───────────────────────────────────────────────────

function ProviderDot({ status }: { status: string }) {
  return (
    <span className={clsx(
      "inline-block w-2 h-2 rounded-full shrink-0",
      status === "healthy"      ? "bg-emerald-500"                        :
      status === "degraded"     ? "bg-amber-500 animate-pulse"            :
      status === "unhealthy"    ? "bg-red-500 animate-pulse"              :
      status === "unconfigured" ? "bg-zinc-600"                           :
                                  "bg-zinc-600",
    )} />
  );
}

function ProviderStatusWidget() {
  const { data: health, isLoading: healthLoading } = useQuery({
    queryKey: ["detailed-health"],
    queryFn:  () => defaultApi.getDetailedHealth(),
    refetchInterval: 30_000,
    retry: false,
  });

  const { data: ollamaModels, isLoading: modelsLoading } = useQuery({
    queryKey: ["ollama-models-dashboard"],
    queryFn:  () => defaultApi.getOllamaModels(),
    refetchInterval: 60_000,
    retry: false,
  });

  const isLoading = healthLoading || modelsLoading;

  type ProviderRow = { name: string; status: string; detail: string };
  const providers: ProviderRow[] = [];

  if (health) {
    providers.push({
      name:   "Store",
      status: health.checks.store.status,
      detail: health.checks.store.latency_ms !== undefined
        ? `${health.checks.store.latency_ms}ms`
        : health.checks.store.status,
    });
    providers.push({
      name:   "Ollama",
      status: health.checks.ollama.status,
      detail: health.checks.ollama.status === "unconfigured"
        ? "not configured"
        : health.checks.ollama.models !== undefined
        ? `${health.checks.ollama.models} model${health.checks.ollama.models !== 1 ? "s" : ""}`
        : health.checks.ollama.status,
    });
    providers.push({
      name:   "Events",
      status: health.checks.event_buffer.status,
      detail: health.checks.event_buffer.status,
    });
    providers.push({
      name:   "Memory",
      status: health.checks.memory.status,
      detail: health.checks.memory.rss_mb !== undefined
        ? `${health.checks.memory.rss_mb}MB RSS`
        : health.checks.memory.status,
    });
  }

  return (
    <Panel>
      <div className="flex items-center justify-between mb-2">
        <SectionLabel>Providers</SectionLabel>
        {ollamaModels && (
          <span className="text-[10px] text-zinc-600 font-mono truncate max-w-[100px]">
            {ollamaModels.host.replace(/^https?:\/\//, "")}
          </span>
        )}
      </div>

      {isLoading ? (
        <div className="space-y-2.5">
          {[0, 1, 2, 3].map(i => (
            <div key={i} className="flex items-center gap-2">
              <Skeleton className="h-2 w-2 rounded-full" />
              <Skeleton className="h-3 flex-1" />
              <Skeleton className="h-3 w-12" />
            </div>
          ))}
        </div>
      ) : providers.length === 0 ? (
        <p className="text-[12px] text-zinc-600 italic">Health endpoint unavailable</p>
      ) : (
        <div className="space-y-2">
          {providers.map(p => (
            <div key={p.name} className="flex items-center gap-2.5">
              <ProviderDot status={p.status} />
              <span className="text-[12px] text-zinc-400 flex-1">{p.name}</span>
              <span className={clsx(
                "text-[10px] font-mono tabular-nums",
                p.status === "healthy"      ? "text-emerald-500" :
                p.status === "unconfigured" ? "text-zinc-600"    :
                p.status === "degraded"     ? "text-amber-400"   : "text-red-400",
              )}>
                {p.detail}
              </span>
            </div>
          ))}

          {/* Ollama model list (compact) */}
          {ollamaModels && ollamaModels.models.length > 0 && (
            <div className="pt-2 mt-1 border-t border-zinc-800/60">
              <p className="text-[10px] text-zinc-700 mb-1.5">Local models</p>
              <div className="flex flex-wrap gap-1">
                {ollamaModels.models.map(m => (
                  <span key={m} className="text-[10px] font-mono text-zinc-500 bg-zinc-800 px-1.5 py-0.5 rounded">
                    {m}
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </Panel>
  );
}

// ── Widget: System health ─────────────────────────────────────────────────────

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
    queryFn:  () => defaultApi.getDetailedHealth(),
    refetchInterval: 30_000,
    retry: false,
  });

  return (
    <Panel>
      <div className="flex items-center justify-between mb-3">
        <SectionLabel>System Health</SectionLabel>
        {data && (
          <span className={clsx(
            "inline-flex items-center gap-1.5 text-[10px] font-medium rounded px-1.5 py-0.5",
            data.status === "healthy"  ? "bg-emerald-500/10 text-emerald-400" :
            data.status === "degraded" ? "bg-amber-500/10 text-amber-400"     :
                                          "bg-red-500/10 text-red-400",
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

          <div className="flex items-center justify-between pt-2 mt-1 border-t border-zinc-800/60">
            <span className="text-[10px] text-zinc-700 font-mono">v{data.version}</span>
            <span className="text-[10px] text-zinc-700 font-mono tabular-nums">
              up {fmtUptime(data.uptime_seconds)}
            </span>
          </div>
        </div>
      )}
    </Panel>
  );
}

// ── Widget: Critical events ───────────────────────────────────────────────────

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
            <li key={`evt-${i}`} className="flex items-start gap-2 rounded bg-zinc-800/50 px-3 py-2">
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
          {components.length} degraded component{components.length > 1 ? "s" : ""}
        </p>
        <p className="text-[11px] text-amber-600 mt-0.5">{components.join(", ")}</p>
      </div>
    </div>
  );
}

// ── Widget: Event sparkline ───────────────────────────────────────────────────

/**
 * Derives an hourly-bucket event count from the last 12h of event positions.
 * Since the global event log only gives us position + timestamp, we bucket
 * stored_at timestamps into hourly counts.
 */
function useHourlyEventCounts(): number[] {
  const { data } = useQuery({
    queryKey: ["events-recent-sparkline"],
    queryFn:  () => defaultApi.getRecentEvents(200),
    refetchInterval: 30_000,
    retry: false,
    staleTime: 15_000,
  });

  const hours = 12;
  const now   = Date.now();
  const buckets = Array.from({ length: hours }, (_, i) => {
    const bucketStart = now - (hours - i) * 3_600_000;
    const bucketEnd   = bucketStart + 3_600_000;
    return (data ?? []).filter((e) => {
      const ts = typeof e === "object" && e !== null && "timestamp" in e
        ? new Date((e as { timestamp: string }).timestamp).getTime()
        : 0;
      return ts >= bucketStart && ts < bucketEnd;
    }).length;
  });

  // If all zeros (no data yet), return a gentle flat line so the chart renders.
  return buckets.every((b) => b === 0) ? Array.from({ length: hours }, () => 0) : buckets;
}

function EventSparklineCard({ totalEvents }: { totalEvents: number }) {
  const hourly = useHourlyEventCounts();

  return (
    <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-4 flex items-center justify-between gap-4">
      <div>
        <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-1.5">Events (12h)</p>
        <p className="text-[22px] font-semibold tabular-nums text-zinc-100 leading-none">{totalEvents}</p>
        <p className="text-[10px] text-zinc-600 mt-1">total in log</p>
      </div>
      <MiniChart
        data={hourly}
        width={100}
        height={40}
        color="#6366f1"
        baseline
        className="shrink-0"
      />
    </div>
  );
}

// ── Widget: Top models by token usage ─────────────────────────────────────────

const MODEL_COLORS: Record<string, string> = {
  qwen3:    "#8b5cf6",
  llama:    "#f59e0b",
  mistral:  "#10b981",
  nomic:    "#06b6d4",
  gemma:    "#f97316",
  claude:   "#a855f7",
  gpt:      "#22d3ee",
  deepseek: "#e879f9",
};

function modelColor(modelId: string): string {
  const lower = modelId.toLowerCase();
  for (const [key, color] of Object.entries(MODEL_COLORS)) {
    if (lower.includes(key)) return color;
  }
  return "#6366f1";
}

function fmtTokensShort(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000)     return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function ModelUsageWidget() {
  const { data: tracesData, isLoading } = useQuery({
    queryKey: ["traces-model-usage"],
    queryFn:  () => defaultApi.getTraces(200),
    refetchInterval: 60_000,
    staleTime: 30_000,
    retry: false,
  });

  const modelItems = useMemo(() => {
    const traces = tracesData?.traces ?? [];
    const byModel: Record<string, number> = {};
    for (const t of traces) {
      byModel[t.model_id] = (byModel[t.model_id] ?? 0) + t.prompt_tokens + t.completion_tokens;
    }
    return Object.entries(byModel)
      .map(([label, value]) => ({ label, value, color: modelColor(label) }))
      .sort((a, b) => b.value - a.value);
  }, [tracesData]);

  return (
    <Panel>
      <SectionLabel>Top Models · Token Usage</SectionLabel>
      {isLoading ? (
        <div className="space-y-2 animate-pulse">
          {[0, 1, 2].map((i) => <div key={i} className="h-5 rounded bg-zinc-800" />)}
        </div>
      ) : modelItems.length === 0 ? (
        <p className="text-[12px] text-zinc-600 italic py-1">
          No traces yet — model usage appears after LLM calls.
        </p>
      ) : (
        <BarChart
          items={modelItems}
          formatValue={fmtTokensShort}
          maxItems={6}
          barHeight={6}
          rowGap={8}
        />
      )}
    </Panel>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

// ── Onboarding banner ─────────────────────────────────────────────────────────

const ONBOARDED_KEY = 'cairn_onboarded';

const STEPS = [
  { n: 1, title: 'Connect an LLM', body: 'Set OLLAMA_HOST to a running Ollama instance, or configure an OpenAI-compatible provider under Providers.' },
  { n: 2, title: 'Create a Session', body: 'POST /v1/sessions (or click New Session in the Sessions page) to create a conversation context for your agent.' },
  { n: 3, title: 'Start a Run', body: 'POST /v1/runs with a session_id to kick off an agent execution. Runs, tasks, and costs appear here in real time.' },
];

function OnboardingBanner() {
  const [dismissed, setDismissed] = useState(
    () => localStorage.getItem(ONBOARDED_KEY) === 'true',
  );

  if (dismissed) return null;

  function dismiss(permanent: boolean) {
    if (permanent) localStorage.setItem(ONBOARDED_KEY, 'true');
    setDismissed(true);
  }

  return (
    <div className="rounded-lg border-l-2 border-l-indigo-500 border border-zinc-800 bg-zinc-900 p-4">
      <div className="flex items-start justify-between gap-4 mb-3">
        <div>
          <p className="text-[13px] font-semibold text-zinc-100">Welcome to Cairn ✦</p>
          <p className="text-[11px] text-zinc-500 mt-0.5">
            A self-hostable control plane for production AI agents. Here's how to get started.
          </p>
        </div>
        <button
          onClick={() => dismiss(false)}
          aria-label="Dismiss onboarding"
          className="p-1 rounded text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800 transition-colors shrink-0"
        >
          ×
        </button>
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {STEPS.map(({ n, title, body }) => (
          <div key={n} className="flex gap-3">
            <span className="shrink-0 w-5 h-5 rounded-full bg-indigo-500/20 text-indigo-400 text-[11px] font-bold flex items-center justify-center mt-0.5">
              {n}
            </span>
            <div>
              <p className="text-[12px] font-medium text-zinc-200">{title}</p>
              <p className="text-[11px] text-zinc-500 mt-0.5 leading-snug">{body}</p>
            </div>
          </div>
        ))}
      </div>

      <div className="flex items-center gap-4 mt-3 pt-3 border-t border-zinc-800/60">
        <button
          onClick={() => dismiss(true)}
          className="text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors"
        >
          Don&apos;t show again
        </button>
        <a
          href="#sessions"
          onClick={() => { window.location.hash = 'sessions'; dismiss(false); }}
          className="ml-auto text-[11px] text-indigo-400 hover:text-indigo-300 transition-colors"
        >
          Go to Sessions →
        </a>
      </div>
    </div>
  );
}

// ── Dashboard tab definitions ─────────────────────────────────────────────────

const DASH_TABS = ['Overview', 'Runs', 'Tasks', 'Activity'] as const;
type DashTab = typeof DASH_TABS[number];

// ── Export helpers ────────────────────────────────────────────────────────────

function triggerDownload(content: string, filename: string, mime: string) {
  const blob = new Blob([content], { type: mime });
  const url  = URL.createObjectURL(blob);
  const a    = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

interface DashboardSnapshot {
  exported_at:       string;
  system_healthy:    boolean;
  active_runs:       number;
  active_tasks:      number;
  pending_approvals: number;
  failed_runs_24h:   number;
  total_events:      number;
  total_sessions:    number;
  total_runs:        number;
  uptime_seconds:    number;
  active_providers:  number;
  active_plugins:    number;
  memory_doc_count:  number;
  eval_runs_today:   number;
}

function buildSnapshot(
  stats: import("../lib/types").SystemStats | undefined,
  data:  import("../lib/types").DashboardOverview | undefined,
): DashboardSnapshot {
  return {
    exported_at:       new Date().toISOString(),
    system_healthy:    data?.system_healthy    ?? true,
    active_runs:       stats?.active_runs       ?? data?.active_runs       ?? 0,
    active_tasks:      stats?.total_tasks       ?? data?.active_tasks      ?? 0,
    pending_approvals: stats?.pending_approvals ?? data?.pending_approvals ?? 0,
    failed_runs_24h:   data?.failed_runs_24h    ?? 0,
    total_events:      stats?.total_events      ?? 0,
    total_sessions:    stats?.total_sessions    ?? 0,
    total_runs:        stats?.total_runs        ?? 0,
    uptime_seconds:    stats?.uptime_seconds    ?? 0,
    active_providers:  data?.active_providers   ?? 0,
    active_plugins:    data?.active_plugins     ?? 0,
    memory_doc_count:  data?.memory_doc_count   ?? 0,
    eval_runs_today:   data?.eval_runs_today    ?? 0,
  };
}

function exportJson(snap: DashboardSnapshot) {
  triggerDownload(
    JSON.stringify(snap, null, 2),
    `cairn-dashboard-${new Date().toISOString().slice(0, 10)}.json`,
    "application/json",
  );
}

function exportCsv(snap: DashboardSnapshot) {
  const rows = [
    ["Metric", "Value"],
    ["Exported at",       snap.exported_at],
    ["System healthy",    String(snap.system_healthy)],
    ["Active runs",       String(snap.active_runs)],
    ["Active tasks",      String(snap.active_tasks)],
    ["Pending approvals", String(snap.pending_approvals)],
    ["Failed runs (24h)", String(snap.failed_runs_24h)],
    ["Total events",      String(snap.total_events)],
    ["Total sessions",    String(snap.total_sessions)],
    ["Total runs",        String(snap.total_runs)],
    ["Uptime (seconds)",  String(snap.uptime_seconds)],
    ["Active providers",  String(snap.active_providers)],
    ["Active plugins",    String(snap.active_plugins)],
    ["Memory docs",       String(snap.memory_doc_count)],
    ["Eval runs today",   String(snap.eval_runs_today)],
  ];
  const csv = rows.map(r => r.map(v => `"${v.replace(/"/g, '""')}"`).join(",")).join("\n");
  triggerDownload(
    csv,
    `cairn-dashboard-${new Date().toISOString().slice(0, 10)}.csv`,
    "text/csv;charset=utf-8;",
  );
}


// ── Export menu ───────────────────────────────────────────────────────────────

function ExportMenu({ onJson, onCsv, onPrint }: {
  onJson:  () => void;
  onCsv:   () => void;
  onPrint: () => void;
}) {
  const [open, setOpen] = React.useState(false);
  return (
    <div className="relative">
      <button
        onClick={() => setOpen(v => !v)}
        className="flex items-center gap-1.5 h-7 px-2.5 rounded border border-zinc-700 bg-zinc-900
                   text-[11px] text-zinc-400 hover:text-zinc-200 hover:border-zinc-600 transition-colors"
      >
        <Download size={11} />
        Export
        <ChevronDown size={10} className={clsx("transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />
          <div className="absolute right-0 top-full mt-1 z-20 min-w-[148px] rounded-lg border border-zinc-700 bg-zinc-900 shadow-xl py-1 overflow-hidden">
            <button onClick={() => { onJson(); setOpen(false); }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[12px] text-zinc-300 hover:bg-zinc-800 transition-colors">
              <Download size={11} className="text-zinc-500 shrink-0" />
              Export JSON
            </button>
            <button onClick={() => { onCsv(); setOpen(false); }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[12px] text-zinc-300 hover:bg-zinc-800 transition-colors">
              <Download size={11} className="text-zinc-500 shrink-0" />
              Export CSV
            </button>
            <div className="h-px bg-zinc-800 my-1" />
            <button onClick={() => { onPrint(); setOpen(false); }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[12px] text-zinc-300 hover:bg-zinc-800 transition-colors">
              <Printer size={11} className="text-zinc-500 shrink-0" />
              Print / Save PDF
            </button>
          </div>
        </>
      )}
    </div>
  );
}


export function DashboardPage() {
  const [activeTab, setActiveTab] = useState<DashTab>('Overview');
  const { ms: refreshMs, setOption: setRefreshOption, interval: refreshInterval } = useAutoRefresh("dashboard", "15s");

  const { data: stats, dataUpdatedAt: statsUpdatedAt, isFetching: statsFetching, refetch: refetchStats } = useQuery({
    queryKey: ["stats"],
    queryFn:  () => defaultApi.getStats(),
    refetchInterval: refreshMs || 5_000,
    retry: false,
  });

  const { data, isLoading, isError, error, dataUpdatedAt, refetch: refetchDashboard, isFetching: dashFetching } = useQuery({
    queryKey: ["dashboard"],
    queryFn:  () => defaultApi.getDashboard(),
    refetchInterval: refreshMs || 15_000,
  });

  const { data: recentEventsData } = useQuery({
    queryKey: ["recent-events"],
    queryFn:  () => defaultApi.getRecentEvents(50),
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

  const runs      = stats?.active_runs       ?? data?.active_runs       ?? 0;
  const tasks     = stats?.total_tasks       ?? data?.active_tasks      ?? 0;
  const approvals = stats?.pending_approvals ?? data?.pending_approvals ?? 0;
  const failed    = data?.failed_runs_24h    ?? 0;

  const runVariant:  StatCardVariant = runs      > 0 ? "info"    : "default";
  const taskVariant: StatCardVariant = tasks     > 0 ? "info"    : "default";
  const aprVariant:  StatCardVariant = approvals > 0 ? "warning" : "default";
  const failVariant: StatCardVariant = failed    > 0 ? "danger"  : "success";

  const latestUpdate = statsUpdatedAt || dataUpdatedAt;
  const updatedAt = latestUpdate ? new Date(latestUpdate).toLocaleTimeString() : null;

  return (
    <div className="h-full overflow-y-auto bg-zinc-950">
      <div className="max-w-5xl mx-auto px-6 py-6 space-y-6">

        {/* Onboarding banner — only on first visit */}
        <OnboardingBanner />

        {/* Header */}
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
            {/* Auto-refresh control */}
            <div className="flex items-center gap-1">
              <div className="relative">
                <select value={refreshInterval.option}
                  onChange={e => setRefreshOption(e.target.value as import('../hooks/useAutoRefresh').RefreshOption)}
                  className="appearance-none rounded border border-zinc-700 bg-zinc-900 text-[11px] font-mono pl-5 pr-2 h-7 text-zinc-400 focus:outline-none focus:border-indigo-500 transition-colors"
                >
                  {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
                </select>
                <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none">
                  {(statsFetching || dashFetching)
                    ? <RefreshCw size={9} className="animate-spin text-indigo-400" />
                    : <RefreshCw size={9} className="text-zinc-600" />
                  }
                </span>
              </div>
              <button onClick={() => { void refetchStats(); void refetchDashboard(); }}
                disabled={statsFetching || dashFetching}
                className="flex items-center gap-1 h-7 px-2 rounded border border-zinc-700 bg-zinc-900 text-[11px] text-zinc-500 hover:text-zinc-200 hover:border-zinc-600 disabled:opacity-40 transition-colors"
              >
                <RefreshCw size={11} className={(statsFetching || dashFetching) ? "animate-spin" : ""} />
                <span className="hidden sm:inline">Refresh</span>
              </button>
            </div>
            {/* Export menu — hidden in print */}
            <div className="no-print">
              <ExportMenu
                onJson={() => exportJson(buildSnapshot(stats, data))}
                onCsv={() => exportCsv(buildSnapshot(stats, data))}
                onPrint={() => window.print()}
              />
            </div>
          </div>
        </div>

        {/* Print header — only visible when printing */}
        <div className="print-header hidden">
          <h1 style={{ fontSize: 18, fontWeight: 700, marginBottom: 4 }}>
            cairn — Dashboard Export
          </h1>
          <p style={{ fontSize: 12, color: '#6b7280' }}>
            Exported at {new Date().toLocaleString()} ·{' '}
            {data?.system_healthy ? '✓ All systems operational' : '⚠ Degraded'}
          </p>
          <hr style={{ margin: '10px 0', borderColor: '#e5e7eb' }} />
        </div>

        {/* Degraded banner */}
        {data && <DegradedBanner components={data.degraded_components ?? []} />}

        {/* Activity stat cards */}
        <div>
          <SectionLabel>Activity</SectionLabel>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard label="Active Runs"       value={runs}      variant={runVariant}  loading={isLoading} help="Agent runs currently in pending, running, paused, or waiting state." />
            <StatCard label="Active Tasks"      value={tasks}     variant={taskVariant} loading={isLoading} help="Work items queued, leased, or running across all agent tasks." />
            <StatCard label="Pending Approvals" value={approvals} variant={aprVariant}  loading={isLoading} help="Human-in-the-loop gates waiting for an operator approve or reject decision." />
            <StatCard label="Failed (24h)"      value={failed}    variant={failVariant} loading={isLoading} help="Runs that transitioned to the 'failed' state in the last 24 hours." />
          </div>
          {/* Event sparkline — full-width strip beneath stat cards */}
          {!isLoading && (
            <div className="mt-3">
              <EventSparklineCard totalEvents={stats?.total_events ?? 0} />
            </div>
          )}
        </div>

        {/* Infrastructure stat cards */}
        <div>
          <SectionLabel>Infrastructure</SectionLabel>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard label="Providers"   value={data?.active_providers ?? 0} loading={isLoading} help="Registered LLM provider connections (e.g. Ollama, OpenAI)." />
            <StatCard label="Plugins"     value={data?.active_plugins   ?? 0} loading={isLoading} help="Active cairn plugins — external tools and skill extensions." />
            <StatCard label="Memory Docs" value={data?.memory_doc_count  ?? 0} loading={isLoading} help="Total document chunks indexed in the knowledge base for retrieval." />
            <StatCard label="Evals Today" value={data?.eval_runs_today   ?? 0} loading={isLoading} help="Evaluation runs completed today across all prompt releases." />
          </div>
        </div>

        {/* Tab bar */}
        <div
          role="tablist"
          aria-label="Dashboard sections"
          className="flex items-center gap-0 border-b border-zinc-800 -mb-2"
          onKeyDown={e => {
            const idx = DASH_TABS.indexOf(activeTab);
            if (e.key === 'ArrowRight') setActiveTab(DASH_TABS[(idx + 1) % DASH_TABS.length]);
            else if (e.key === 'ArrowLeft') setActiveTab(DASH_TABS[(idx - 1 + DASH_TABS.length) % DASH_TABS.length]);
          }}
        >
          {DASH_TABS.map(tab => (
            <button
              key={tab}
              role="tab"
              aria-selected={activeTab === tab}
              aria-controls={`dash-panel-${tab.toLowerCase()}`}
              id={`dash-tab-${tab.toLowerCase()}`}
              onClick={() => setActiveTab(tab)}
              className={clsx(
                'px-4 h-9 text-[12px] font-medium transition-colors border-b-2 -mb-px',
                activeTab === tab
                  ? 'text-zinc-100 border-indigo-500'
                  : 'text-zinc-500 border-transparent hover:text-zinc-300',
              )}
            >
              {tab}
            </button>
          ))}
        </div>

        {/* Tab panels */}
        <div
          role="tabpanel"
          id={`dash-panel-${activeTab.toLowerCase()}`}
          aria-labelledby={`dash-tab-${activeTab.toLowerCase()}`}
        >
          {activeTab === 'Overview' && (
            <div className="space-y-6">
              {/* Live widgets row — Active Runs + Cost/Providers + Event log */}
              <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
                <ActiveRunsWidget />
                <div className="flex flex-col gap-4">
                  <CostWidget />
                  <ProviderStatusWidget />
                </div>
                <Panel className="flex flex-col">
                  <SectionLabel>Recent Activity</SectionLabel>
                  <EventLog initialEvents={recentEventsData ?? []} maxEvents={50} />
                </Panel>
              </div>
              <ModelUsageWidget />
              <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
                <CriticalEvents events={data?.recent_critical_events ?? []} />
                <SystemHealthCard />
              </div>
            </div>
          )}

          {activeTab === 'Runs' && (
            <Panel>
              <SectionLabel>Recent Runs</SectionLabel>
              <ActiveRunsWidget />
            </Panel>
          )}

          {activeTab === 'Tasks' && (
            <Panel>
              <SectionLabel>Recent Tasks</SectionLabel>
              <div className="space-y-1 pt-1">
                {(stats?.total_tasks ?? 0) === 0 ? (
                  <p className="text-[12px] text-zinc-600 py-4 text-center">No tasks yet.</p>
                ) : (
                  <p className="text-[12px] text-zinc-500">
                    {stats?.total_tasks ?? 0} total tasks · {runs} active
                  </p>
                )}
              </div>
            </Panel>
          )}

          {activeTab === 'Activity' && (
            <Panel className="flex flex-col min-h-[320px]">
              <SectionLabel>Live Event Stream</SectionLabel>
              <EventLog initialEvents={recentEventsData ?? []} maxEvents={100} />
            </Panel>
          )}
        </div>

      </div>
    </div>
  );
}

export default DashboardPage;
