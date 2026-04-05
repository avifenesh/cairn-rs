import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Waves,
  RefreshCw,
  ServerCrash,
  Inbox,
  ChevronRight,
  ChevronDown,
  AlertCircle,
  CheckCircle2,
  Clock,
  Coins,
  Cpu,
  Zap,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import type { LlmCallTrace } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function fmtCost(micros: number): string {
  if (micros === 0) return "—";
  if (micros < 1000) return `${micros} µ$`;
  return `$${(micros / 1_000_000).toFixed(4)}`;
}

function fmtTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

function shortId(id: string): string {
  return id.length > 20 ? `${id.slice(0, 8)}\u2026${id.slice(-4)}` : id;
}

// ── Latency bar ───────────────────────────────────────────────────────────────
// Renders a proportional bar relative to maxLatency in the current result set.

function LatencyBar({
  latencyMs,
  maxMs,
  isError,
}: {
  latencyMs: number;
  maxMs: number;
  isError: boolean;
}) {
  const pct = maxMs > 0 ? Math.max(2, (latencyMs / maxMs) * 100) : 2;
  return (
    <div className="flex items-center gap-2 min-w-0 w-36">
      <div className="flex-1 h-2 rounded-full bg-zinc-800 overflow-hidden">
        <div
          className={clsx(
            "h-full rounded-full transition-all",
            isError
              ? "bg-red-600"
              : latencyMs > 5000
              ? "bg-amber-500"
              : latencyMs > 2000
              ? "bg-yellow-500"
              : "bg-indigo-500",
          )}
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="text-xs tabular-nums text-zinc-400 shrink-0 w-14 text-right">
        {latencyMs >= 1000 ? `${(latencyMs / 1000).toFixed(1)}s` : `${latencyMs}ms`}
      </span>
    </div>
  );
}

// ── Expandable trace row ──────────────────────────────────────────────────────

function TraceRow({
  trace,
  maxLatencyMs,
}: {
  trace: LlmCallTrace;
  maxLatencyMs: number;
}) {
  const [open, setOpen] = useState(false);
  const totalTokens = trace.prompt_tokens + trace.completion_tokens;

  return (
    <>
      {/* Summary row */}
      <tr
        onClick={() => setOpen((v) => !v)}
        className={clsx(
          "cursor-pointer border-b border-zinc-800/60 transition-colors",
          open ? "bg-zinc-800/60" : "hover:bg-zinc-900/60",
        )}
      >
        {/* Expand toggle */}
        <td className="px-3 py-3 w-6">
          {open ? (
            <ChevronDown size={13} className="text-zinc-500" />
          ) : (
            <ChevronRight size={13} className="text-zinc-600" />
          )}
        </td>

        {/* Status */}
        <td className="px-2 py-3 w-8">
          {trace.is_error ? (
            <AlertCircle size={14} className="text-red-400" />
          ) : (
            <CheckCircle2 size={14} className="text-emerald-500" />
          )}
        </td>

        {/* Model */}
        <td className="px-3 py-3">
          <span className="text-xs font-mono font-medium text-zinc-200">
            {trace.model_id}
          </span>
        </td>

        {/* Latency bar */}
        <td className="px-3 py-3">
          <LatencyBar
            latencyMs={trace.latency_ms}
            maxMs={maxLatencyMs}
            isError={trace.is_error}
          />
        </td>

        {/* Tokens */}
        <td className="px-3 py-3 text-right">
          <span className="text-xs tabular-nums text-zinc-400">
            {fmtTokens(totalTokens)}
          </span>
          <span className="text-[10px] text-zinc-600 ml-1">tok</span>
        </td>

        {/* Cost */}
        <td className="px-3 py-3 text-right">
          <span className="text-xs tabular-nums text-zinc-400">
            {fmtCost(trace.cost_micros)}
          </span>
        </td>

        {/* Time */}
        <td className="px-3 py-3 text-right">
          <span className="text-xs text-zinc-600 whitespace-nowrap">
            {fmtTime(trace.created_at_ms)}
          </span>
        </td>
      </tr>

      {/* Expanded details */}
      {open && (
        <tr className="border-b border-zinc-800/60 bg-zinc-900/40">
          <td colSpan={7} className="px-8 py-4">
            <div className="grid grid-cols-2 gap-x-8 gap-y-1 sm:grid-cols-3 lg:grid-cols-4 text-xs">
              <DetailField label="Trace ID"          value={trace.trace_id} mono />
              <DetailField label="Model"             value={trace.model_id} mono />
              <DetailField label="Latency"           value={`${trace.latency_ms} ms`} />
              <DetailField label="Status"            value={trace.is_error ? "Error" : "Success"}
                valueClass={trace.is_error ? "text-red-400" : "text-emerald-400"} />
              <DetailField label="Prompt tokens"     value={fmtTokens(trace.prompt_tokens)} />
              <DetailField label="Completion tokens" value={fmtTokens(trace.completion_tokens)} />
              <DetailField label="Total tokens"      value={fmtTokens(totalTokens)} />
              <DetailField label="Cost"              value={fmtCost(trace.cost_micros)} />
              {trace.session_id && (
                <DetailField label="Session" value={shortId(trace.session_id)} mono />
              )}
              {trace.run_id && (
                <DetailField label="Run" value={shortId(trace.run_id)} mono />
              )}
              <DetailField label="Timestamp" value={fmtTime(trace.created_at_ms)} />
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

function DetailField({
  label,
  value,
  mono = false,
  valueClass,
}: {
  label: string;
  value: string;
  mono?: boolean;
  valueClass?: string;
}) {
  return (
    <div className="flex flex-col gap-0.5 py-1">
      <span className="text-[10px] text-zinc-600 uppercase tracking-wider">{label}</span>
      <span className={clsx("text-xs break-all", mono && "font-mono", valueClass ?? "text-zinc-300")}>
        {value}
      </span>
    </div>
  );
}

// ── Summary bar ───────────────────────────────────────────────────────────────

function SummaryBar({ traces }: { traces: LlmCallTrace[] }) {
  if (traces.length === 0) return null;

  const totalCost    = traces.reduce((s, t) => s + t.cost_micros, 0);
  const totalTokens  = traces.reduce((s, t) => s + t.prompt_tokens + t.completion_tokens, 0);
  const avgLatency   = Math.round(traces.reduce((s, t) => s + t.latency_ms, 0) / traces.length);
  const errorCount   = traces.filter((t) => t.is_error).length;
  const errorRate    = ((errorCount / traces.length) * 100).toFixed(1);

  const items = [
    { icon: Waves,       label: "Calls",       value: String(traces.length) },
    { icon: Clock,       label: "Avg latency", value: `${avgLatency >= 1000 ? (avgLatency / 1000).toFixed(1) + "s" : avgLatency + "ms"}` },
    { icon: Zap,         label: "Total tokens",value: fmtTokens(totalTokens) },
    { icon: Coins,       label: "Total cost",  value: fmtCost(totalCost) },
    { icon: AlertCircle, label: "Error rate",  value: `${errorRate}%`,
      valueClass: errorCount > 0 ? "text-red-400" : "text-emerald-400" },
  ];

  return (
    <div className="flex flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3 border-b border-zinc-800 bg-zinc-900/40">
      {items.map(({ icon: Icon, label, value, valueClass }) => (
        <div key={label} className="flex items-center gap-2">
          <Icon size={13} className="text-zinc-500 shrink-0" />
          <span className="text-xs text-zinc-500">{label}</span>
          <span className={clsx("text-xs font-medium tabular-nums", valueClass ?? "text-zinc-300")}>
            {value}
          </span>
        </div>
      ))}
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function TracesPage() {
  const [filterError, setFilterError] = useState<"all" | "errors" | "ok">("all");

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["traces"],
    queryFn: () => defaultApi.getTraces(500),
    refetchInterval: 20_000,
  });

  const allTraces = data?.traces ?? [];
  const filtered =
    filterError === "errors"
      ? allTraces.filter((t) => t.is_error)
      : filterError === "ok"
      ? allTraces.filter((t) => !t.is_error)
      : allTraces;

  const maxLatencyMs = filtered.reduce((m, t) => Math.max(m, t.latency_ms), 1);

  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center p-8">
        <ServerCrash size={40} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load traces</p>
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
        <Waves size={15} className="text-indigo-400 shrink-0" />
        <h2 className="text-sm font-semibold text-zinc-200">
          LLM Traces
          {!isLoading && (
            <span className="ml-2 text-xs text-zinc-500 font-normal">
              {filtered.length}
              {filterError !== "all" ? ` / ${allTraces.length} total` : ""}
            </span>
          )}
        </h2>

        {/* Error filter */}
        <select
          value={filterError}
          onChange={(e) => setFilterError(e.target.value as typeof filterError)}
          className="rounded-md bg-zinc-800 border border-zinc-700 text-zinc-300 text-xs px-2.5 py-1.5 focus:outline-none focus:ring-1 focus:ring-indigo-500"
        >
          <option value="all">All calls</option>
          <option value="ok">Successful only</option>
          <option value="errors">Errors only</option>
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

      {/* ── Summary bar ───────────────────────────────────────────────── */}
      {!isLoading && <SummaryBar traces={filtered} />}

      {/* ── Table ─────────────────────────────────────────────────────── */}
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <SkeletonRows />
        ) : filtered.length === 0 ? (
          <EmptyState filter={filterError} />
        ) : (
          <table className="min-w-full text-sm">
            <thead className="sticky top-0 z-10 bg-zinc-950">
              <tr className="border-b border-zinc-800">
                <th className="px-3 py-2 w-6" />
                <th className="px-2 py-2 w-8" />
                {[
                  { label: "Model",   cls: "text-left"  },
                  { label: "Latency", cls: "text-left"  },
                  { label: "Tokens",  cls: "text-right" },
                  { label: "Cost",    cls: "text-right" },
                  { label: "Time",    cls: "text-right" },
                ].map(({ label, cls }) => (
                  <th
                    key={label}
                    className={clsx(
                      "px-3 py-2 text-xs font-medium text-zinc-500 uppercase tracking-widest whitespace-nowrap",
                      cls,
                    )}
                  >
                    {label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {filtered.map((trace) => (
                <TraceRow
                  key={trace.trace_id}
                  trace={trace}
                  maxLatencyMs={maxLatencyMs}
                />
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

// ── Empty state ───────────────────────────────────────────────────────────────

function EmptyState({ filter }: { filter: "all" | "errors" | "ok" }) {
  return (
    <div className="flex flex-col items-center justify-center py-24 gap-3 text-center">
      <Cpu size={36} className="text-zinc-700" />
      <p className="text-sm text-zinc-400">
        {filter === "errors"
          ? "No error traces — all calls succeeded"
          : filter === "ok"
          ? "No successful traces yet"
          : "No LLM traces recorded yet"}
      </p>
      <p className="text-xs text-zinc-600">
        Traces appear when provider calls complete via{" "}
        <code className="text-zinc-500 bg-zinc-800 rounded px-1">ProviderCallCompleted</code> events.
      </p>
    </div>
  );
}

// ── Loading skeleton ──────────────────────────────────────────────────────────

function SkeletonRows() {
  return (
    <div className="divide-y divide-zinc-800/60">
      {Array.from({ length: 10 }).map((_, i) => (
        <div key={i} className="flex items-center gap-3 px-4 py-3.5 animate-pulse">
          <div className="h-3 w-3 rounded-full bg-zinc-800 shrink-0" />
          <div className="h-3 w-3 rounded-full bg-zinc-800 shrink-0" />
          <div className="h-3 w-36 rounded bg-zinc-800" />
          <div className="h-2 w-32 rounded-full bg-zinc-800" />
          <div className="ml-auto h-3 w-14 rounded bg-zinc-800" />
          <div className="h-3 w-14 rounded bg-zinc-800" />
          <div className="h-3 w-28 rounded bg-zinc-800" />
        </div>
      ))}
    </div>
  );
}

export default TracesPage;
