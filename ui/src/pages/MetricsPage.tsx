/**
 * MetricsPage — API usage analytics from GET /v1/metrics.
 *
 * The metrics endpoint is populated by the request-tracing middleware in
 * cairn-app (main.rs) which records every HTTP request into a rolling
 * 1 000-entry ring buffer.  This page visualises that buffer.
 */

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  RefreshCw, Loader2, AlertTriangle, TrendingUp, TrendingDown,
  Minus, Activity, Clock, Zap, ExternalLink,
} from "lucide-react";
import { clsx } from "clsx";
import { BarChart } from "../components/BarChart";
import { defaultApi } from "../lib/api";

// ── Types ─────────────────────────────────────────────────────────────────────

interface MetricsSnapshot {
  total_requests:   number;
  requests_by_path: Record<string, number>;
  avg_latency_ms:   number;
  p50_latency_ms:   number;
  p95_latency_ms:   number;
  p99_latency_ms:   number;
  error_rate:       number;
  errors_by_status: Record<string, number>;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtLatency(ms: number): string {
  if (ms === 0) return "—";
  if (ms < 1_000) return `${ms}ms`;
  return `${(ms / 1_000).toFixed(2)}s`;
}

function fmtPct(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`;
}

function fmtCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000)     return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/** Colour a bar by HTTP path semantics */
function pathColor(path: string): string {
  if (path.startsWith("/v1/runs"))     return "#3b82f6";  // blue
  if (path.startsWith("/v1/tasks"))    return "#8b5cf6";  // violet
  if (path.startsWith("/v1/session"))  return "#06b6d4";  // cyan
  if (path.startsWith("/v1/events"))   return "#f59e0b";  // amber
  if (path.startsWith("/v1/memory"))   return "#10b981";  // emerald
  if (path.startsWith("/v1/provider")) return "#ec4899";  // pink
  if (path.startsWith("/v1/admin"))    return "#f97316";  // orange
  if (path.startsWith("/health"))      return "#64748b";  // slate
  return "#6366f1";                                        // indigo default
}

/** Colour a status code */
function statusColor(code: string): string {
  const n = Number(code);
  if (n >= 500) return "#ef4444";  // red
  if (n >= 400) return "#f59e0b";  // amber
  if (n >= 300) return "#06b6d4";  // cyan
  return "#10b981";                 // emerald
}

// ── Stat card ─────────────────────────────────────────────────────────────────

function StatCard({
  label, value, sub, accent = "indigo", trend,
}: {
  label:   string;
  value:   string;
  sub?:    string;
  accent?: "indigo" | "emerald" | "amber" | "red" | "zinc";
  trend?:  "up" | "down" | "flat";
}) {
  const border = {
    indigo:  "border-l-indigo-500",
    emerald: "border-l-emerald-500",
    amber:   "border-l-amber-500",
    red:     "border-l-red-500",
    zinc:    "border-l-zinc-600",
  }[accent];
  const textColor = {
    indigo:  "text-indigo-400",
    emerald: "text-emerald-400",
    amber:   "text-amber-400",
    red:     "text-red-400",
    zinc:    "text-gray-700 dark:text-zinc-300",
  }[accent];

  return (
    <div className={clsx(
      "bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 rounded-xl p-4 flex flex-col gap-1.5",
      border,
    )}>
      <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider truncate">
        {label}
      </p>
      <div className="flex items-baseline gap-2">
        <p className={clsx("text-[22px] font-semibold tabular-nums leading-none", textColor)}>
          {value}
        </p>
        {trend === "up"   && <TrendingUp   size={13} className="text-red-400 shrink-0" />}
        {trend === "down" && <TrendingDown  size={13} className="text-emerald-400 shrink-0" />}
        {trend === "flat" && <Minus         size={13} className="text-gray-400 dark:text-zinc-600 shrink-0" />}
      </div>
      {sub && <p className="text-[11px] text-gray-400 dark:text-zinc-600 truncate">{sub}</p>}
    </div>
  );
}

// ── Latency percentile strip ──────────────────────────────────────────────────

function LatencyStrip({ data }: { data: MetricsSnapshot }) {
  const bars = [
    { label: "avg",  ms: data.avg_latency_ms, color: "#6366f1" },
    { label: "p50",  ms: data.p50_latency_ms, color: "#06b6d4" },
    { label: "p95",  ms: data.p95_latency_ms, color: "#f59e0b" },
    { label: "p99",  ms: data.p99_latency_ms, color: "#ef4444" },
  ];
  const max = Math.max(...bars.map(b => b.ms), 1);

  return (
    <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-xl p-4">
      <div className="flex items-center gap-2 mb-4">
        <Clock size={13} className="text-gray-400 dark:text-zinc-500" />
        <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
          Latency Percentiles
        </p>
      </div>
      <div className="grid grid-cols-4 gap-3">
        {bars.map(({ label, ms, color }) => {
          const pct = (ms / max) * 100;
          return (
            <div key={label} className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-600 uppercase">{label}</span>
                <span className="text-[12px] font-mono tabular-nums" style={{ color }}>
                  {fmtLatency(ms)}
                </span>
              </div>
              {/* Mini vertical bar */}
              <div className="h-16 bg-gray-100 dark:bg-zinc-800 rounded-sm overflow-hidden flex items-end">
                <div
                  className="w-full rounded-sm transition-all duration-500"
                  style={{ height: `${Math.max(pct, 2)}%`, backgroundColor: color + "99" }}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Error breakdown ───────────────────────────────────────────────────────────

function ErrorBreakdown({ errors }: { errors: Record<string, number> }) {
  const entries = Object.entries(errors)
    .map(([code, count]) => ({ label: `HTTP ${code}`, value: count, color: statusColor(code) }))
    .sort((a, b) => b.value - a.value);

  const total = entries.reduce((s, e) => s + e.value, 0);

  if (entries.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-8 gap-2 text-center">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-emerald-950/30 border border-emerald-900/40">
          <Activity size={16} className="text-emerald-400" />
        </div>
        <p className="text-[12px] text-emerald-400 font-medium">No errors recorded</p>
        <p className="text-[11px] text-gray-300 dark:text-zinc-700">All requests returned 2xx/3xx</p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <BarChart
        items={entries}
        formatValue={v => String(v)}
        maxItems={8}
        barHeight={7}
        rowGap={8}
      />
      <p className="text-[10px] text-gray-300 dark:text-zinc-700 text-right">{total.toLocaleString()} total errors</p>
    </div>
  );
}

// ── Top endpoints ─────────────────────────────────────────────────────────────

function TopEndpoints({ byPath }: { byPath: Record<string, number> }) {
  const items = Object.entries(byPath)
    .map(([path, count]) => ({
      label: path,
      value: count,
      color: pathColor(path),
    }))
    .sort((a, b) => b.value - a.value)
    .slice(0, 10);

  if (items.length === 0) {
    return <p className="text-[12px] text-gray-400 dark:text-zinc-600 italic py-2">No request data yet.</p>;
  }

  return (
    <BarChart
      items={items}
      formatValue={fmtCount}
      maxItems={10}
      barHeight={6}
      rowGap={10}
    />
  );
}

// ── Endpoint table ────────────────────────────────────────────────────────────

function EndpointTable({ byPath }: { byPath: Record<string, number> }) {
  const [showAll, setShowAll] = useState(false);
  const sorted = Object.entries(byPath)
    .sort(([, a], [, b]) => b - a);
  const total = sorted.reduce((s, [, c]) => s + c, 0);
  const visible = showAll ? sorted : sorted.slice(0, 15);

  if (sorted.length === 0) return null;

  return (
    <div className="rounded-xl border border-gray-200 dark:border-zinc-800 overflow-hidden">
      <div className="flex items-center h-8 px-4 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950">
        <span className="flex-1 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Endpoint</span>
        <span className="w-20 text-right text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Requests</span>
        <span className="w-16 text-right text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Share</span>
      </div>
      {visible.map(([path, count], i) => {
        const share = total > 0 ? (count / total) * 100 : 0;
        return (
          <div
            key={path}
            className={clsx(
              "flex items-center h-8 px-4 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0",
              i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50",
            )}
          >
            <span
              className="flex-1 text-[11px] font-mono text-gray-500 dark:text-zinc-400 truncate min-w-0 pr-3"
              title={path}
            >
              <span className="w-1.5 h-1.5 rounded-full inline-block mr-2 shrink-0"
                style={{ backgroundColor: pathColor(path) }} />
              {path}
            </span>
            <span className="w-20 text-right text-[11px] tabular-nums text-gray-700 dark:text-zinc-300 font-mono">
              {count.toLocaleString()}
            </span>
            <span className="w-16 text-right text-[11px] tabular-nums text-gray-400 dark:text-zinc-600">
              {share.toFixed(1)}%
            </span>
          </div>
        );
      })}
      {!showAll && sorted.length > 15 && (
        <div className="flex justify-center py-2 border-t border-gray-200 dark:border-zinc-800">
          <button
            onClick={() => setShowAll(true)}
            className="text-[11px] text-indigo-500 hover:text-indigo-400 transition-colors"
          >
            Show all {sorted.length} endpoints
          </button>
        </div>
      )}
    </div>
  );
}

// ── Placeholder (when /v1/metrics is unavailable) ─────────────────────────────

function MetricsUnavailable() {
  return (
    <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-xl p-6 space-y-4">
      <div className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-amber-950/30 border border-amber-800/40">
          <AlertTriangle size={16} className="text-amber-400" />
        </div>
        <div>
          <p className="text-[13px] font-semibold text-gray-800 dark:text-zinc-200">Metrics endpoint unavailable</p>
          <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-1">
            The <code className="bg-gray-100 dark:bg-zinc-800 rounded px-1 text-gray-500 dark:text-zinc-400">GET /v1/metrics</code> endpoint
            returned an error. This usually means:
          </p>
        </div>
      </div>
      <ul className="space-y-1.5 pl-4">
        {[
          "You are running cairn-app from lib.rs (bootstrap test server) which doesn't expose /v1/metrics",
          "The server requires an admin token — check your Authorization header",
          "The metrics middleware has not yet recorded any requests",
        ].map((msg, i) => (
          <li key={i} className="flex items-start gap-2 text-[12px] text-gray-400 dark:text-zinc-500">
            <span className="mt-1.5 h-1 w-1 rounded-full bg-zinc-700 shrink-0" />
            {msg}
          </li>
        ))}
      </ul>
      <div className="rounded-lg bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-4 py-3">
        <p className="text-[11px] font-mono text-gray-400 dark:text-zinc-500 mb-1">
          # Verify the endpoint is reachable:
        </p>
        <p className="text-[11px] font-mono text-emerald-400">
          curl -H 'Authorization: Bearer $TOKEN' http://localhost:3000/v1/metrics
        </p>
      </div>
      <p className="text-[11px] text-gray-400 dark:text-zinc-600">
        Once the server receives requests, this page will automatically populate with live data.
        The metrics ring buffer tracks the last 1 000 requests.
      </p>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function MetricsPage() {
  const {
    data, isLoading, isError,
    refetch, isFetching, dataUpdatedAt,
  } = useQuery<MetricsSnapshot>({
    queryKey: ["api-metrics"],
    queryFn:  () => defaultApi.getMetrics(),
    refetchInterval: 10_000,
    retry: 1,
  });

  const errorRate   = data?.error_rate ?? 0;
  const errorTrend  = errorRate > 0.1 ? "up" as const
                    : errorRate < 0.01 ? "flat" as const
                    : "flat" as const;
  const updatedAt = dataUpdatedAt
    ? new Date(dataUpdatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
    : null;

  return (
    <div className="h-full overflow-y-auto bg-white dark:bg-zinc-950">
      <div className="max-w-5xl mx-auto px-5 py-5 space-y-5">

        {/* Toolbar */}
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-[14px] font-semibold text-gray-900 dark:text-zinc-100">API Metrics</h2>
            <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">
              Rolling 1 000-request window · live from <code className="text-gray-400 dark:text-zinc-500">/v1/metrics</code>
            </p>
          </div>
          <div className="flex items-center gap-3">
            {updatedAt && (
              <span className="text-[11px] text-gray-400 dark:text-zinc-600 flex items-center gap-1">
                <Clock size={10} /> {updatedAt}
              </span>
            )}
            <button
              onClick={() => refetch()}
              disabled={isFetching}
              className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900
                         text-gray-400 dark:text-zinc-500 text-[12px] px-2.5 py-1.5 hover:text-gray-800 dark:hover:text-zinc-200 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800
                         disabled:opacity-40 transition-colors"
            >
              <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
              Refresh
            </button>
            <a
              href="/v1/metrics/prometheus"
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900
                         text-gray-400 dark:text-zinc-500 text-[12px] px-2.5 py-1.5 hover:text-gray-800 dark:hover:text-zinc-200 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800
                         transition-colors"
              title="Open Prometheus exposition format"
            >
              <ExternalLink size={11} />
              Prometheus
            </a>
          </div>
        </div>

        {/* Loading */}
        {isLoading && (
          <div className="flex items-center justify-center py-16 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading metrics…</span>
          </div>
        )}

        {/* Error / unavailable */}
        {isError && !isLoading && (
          <MetricsUnavailable />
        )}

        {/* Data */}
        {data && !isError && (
          <>
            {/* Key metric cards */}
            <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
              <StatCard
                label="Total Requests"
                value={fmtCount(data.total_requests)}
                sub="in ring buffer (last 1k)"
                accent="indigo"
              />
              <StatCard
                label="Error Rate"
                value={fmtPct(data.error_rate)}
                sub={`${Object.values(data.errors_by_status).reduce((s, v) => s + v, 0)} errors`}
                accent={data.error_rate > 0.05 ? "red" : data.error_rate > 0.01 ? "amber" : "emerald"}
                trend={errorTrend}
              />
              <StatCard
                label="p95 Latency"
                value={fmtLatency(data.p95_latency_ms)}
                sub={`avg ${fmtLatency(data.avg_latency_ms)}`}
                accent={data.p95_latency_ms > 1000 ? "amber" : data.p95_latency_ms > 300 ? "zinc" : "emerald"}
              />
              <StatCard
                label="p99 Latency"
                value={fmtLatency(data.p99_latency_ms)}
                sub={`p50 ${fmtLatency(data.p50_latency_ms)}`}
                accent={data.p99_latency_ms > 5000 ? "red" : data.p99_latency_ms > 1000 ? "amber" : "zinc"}
              />
            </div>

            {/* Latency percentile chart */}
            <LatencyStrip data={data} />

            {/* Two-column: top endpoints + error breakdown */}
            <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
              {/* Top endpoints bar chart */}
              <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-xl p-4">
                <div className="flex items-center gap-2 mb-4">
                  <Zap size={13} className="text-gray-400 dark:text-zinc-500" />
                  <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
                    Top Endpoints by Request Count
                  </p>
                </div>
                <TopEndpoints byPath={data.requests_by_path} />
              </div>

              {/* Error breakdown */}
              <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-xl p-4">
                <div className="flex items-center gap-2 mb-4">
                  <AlertTriangle size={13} className="text-gray-400 dark:text-zinc-500" />
                  <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
                    Error Breakdown by Status Code
                  </p>
                </div>
                <ErrorBreakdown errors={data.errors_by_status} />
              </div>
            </div>

            {/* Full endpoint table */}
            <div>
              <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-3">
                All Endpoints
              </p>
              <EndpointTable byPath={data.requests_by_path} />
            </div>

            {/* Footer note */}
            <p className="text-[10px] text-gray-300 dark:text-zinc-700 flex items-center gap-1.5">
              <Activity size={10} />
              Metrics are stored in a fixed 1 000-entry ring buffer in cairn-app.
              Restarts reset the buffer. Refreshes every 10 s.
            </p>
          </>
        )}
      </div>
    </div>
  );
}

export default MetricsPage;
