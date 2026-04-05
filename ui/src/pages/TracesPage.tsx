import { useQuery } from "@tanstack/react-query";
import { RefreshCw, Loader2, ServerCrash, Inbox, AlertTriangle, CheckCircle2 } from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import type { LlmCallTrace } from "../lib/types";

// ── Helpers ────────────────────────────────────────────────────────────────────

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const fmtTokens = (n: number) =>
  n >= 1_000 ? `${(n / 1_000).toFixed(1)}k` : String(n);

const fmtLatency = (ms: number) =>
  ms >= 1_000 ? `${(ms / 1_000).toFixed(2)}s` : `${ms}ms`;

const fmtCost = (micros: number) =>
  micros === 0 ? "—" : `$${(micros / 1_000_000).toFixed(5)}`;

/** Extract a short provider name from the model ID (e.g. "gpt-4" → "OpenAI"). */
function inferProvider(modelId: string): string {
  const m = modelId.toLowerCase();
  if (m.startsWith("gpt") || m.startsWith("o1") || m.startsWith("o3")) return "OpenAI";
  if (m.startsWith("claude"))   return "Anthropic";
  if (m.startsWith("gemini"))   return "Google";
  if (m.startsWith("llama") || m.startsWith("qwen") || m.startsWith("mistral") ||
      m.startsWith("nomic"))    return "Ollama";
  if (m.startsWith("titan") || m.startsWith("nova")) return "Bedrock";
  return "—";
}

// ── Stat cards ────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub }: { label: string; value: string | number; sub?: string }) {
  return (
    <div className="border-l-2 border-indigo-500 pl-3 py-0.5">
      <p className="text-[11px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-zinc-600 mt-0.5">{sub}</p>}
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

function TracesTable({ traces }: { traces: LlmCallTrace[] }) {
  if (traces.length === 0) return (
    <div className="flex flex-col items-center justify-center py-16 gap-2 text-zinc-700">
      <Inbox size={26} />
      <p className="text-[13px]">No traces recorded yet</p>
    </div>
  );

  return (
    <table className="min-w-full text-[13px]">
      <thead className="bg-zinc-900 sticky top-0 z-10">
        <tr>
          <TH ch="Trace ID" />
          <TH ch="Model" />
          <TH ch="Provider" />
          <TH ch="Status" />
          <TH ch="Tokens In" right />
          <TH ch="Tokens Out" right />
          <TH ch="Latency" right />
          <TH ch="Cost" right />
          <TH ch="Timestamp" />
        </tr>
      </thead>
      <tbody className="divide-y divide-zinc-800/50">
        {traces.map((trace, i) => (
          <tr key={trace.trace_id}
            className={clsx(
              "group transition-colors",
              i % 2 === 0 ? "bg-zinc-900" : "bg-[#111113]",
              "hover:bg-zinc-800/70",
            )}>
            <td className="px-3 py-1.5 font-mono text-zinc-400 whitespace-nowrap text-[12px]">
              {shortId(trace.trace_id)}
            </td>
            <td className="px-3 py-1.5 font-mono text-zinc-300 whitespace-nowrap">
              {trace.model_id}
            </td>
            <td className="px-3 py-1.5 text-zinc-500 whitespace-nowrap">
              {inferProvider(trace.model_id)}
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap">
              {trace.is_error ? (
                <span className="inline-flex items-center gap-1 text-[11px] text-red-400">
                  <AlertTriangle size={11} /> Error
                </span>
              ) : (
                <span className="inline-flex items-center gap-1 text-[11px] text-emerald-400">
                  <CheckCircle2 size={11} /> OK
                </span>
              )}
            </td>
            <td className="px-3 py-1.5 text-zinc-400 whitespace-nowrap tabular-nums text-right font-mono text-[12px]">
              {fmtTokens(trace.prompt_tokens)}
            </td>
            <td className="px-3 py-1.5 text-zinc-400 whitespace-nowrap tabular-nums text-right font-mono text-[12px]">
              {fmtTokens(trace.completion_tokens)}
            </td>
            <td className={clsx(
              "px-3 py-1.5 whitespace-nowrap tabular-nums text-right font-mono text-[12px]",
              trace.latency_ms > 5_000 ? "text-amber-400" : "text-zinc-400",
            )}>
              {fmtLatency(trace.latency_ms)}
            </td>
            <td className="px-3 py-1.5 text-zinc-500 whitespace-nowrap tabular-nums text-right font-mono text-[12px]">
              {fmtCost(trace.cost_micros)}
            </td>
            <td className="px-3 py-1.5 text-zinc-500 whitespace-nowrap tabular-nums">
              {fmtTime(trace.created_at_ms)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function TracesPage() {
  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["traces"],
    queryFn: () => defaultApi.getTraces(500),
    refetchInterval: 30_000,
  });

  const traces = data?.traces ?? [];

  // Aggregate stats.
  const totalTokens = traces.reduce((s, t) => s + t.prompt_tokens + t.completion_tokens, 0);
  const totalCost   = traces.reduce((s, t) => s + t.cost_micros, 0);
  const avgLatency  = traces.length > 0
    ? Math.round(traces.reduce((s, t) => s + t.latency_ms, 0) / traces.length)
    : 0;
  const errorRate   = traces.length > 0
    ? ((traces.filter(t => t.is_error).length / traces.length) * 100).toFixed(1)
    : "0.0";

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-zinc-300 font-medium">Failed to load traces</p>
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
          LLM Traces
          {!isLoading && (
            <span className="ml-2 text-[12px] text-zinc-500 font-normal">{traces.length}</span>
          )}
        </span>
        <button onClick={() => refetch()} disabled={isFetching}
          className="ml-auto flex items-center gap-1 text-[12px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40 transition-colors">
          <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!isLoading && traces.length > 0 && (
        <div className="flex items-center gap-8 px-5 py-3 border-b border-zinc-800 bg-zinc-900 shrink-0">
          <StatCard label="Calls"        value={traces.length}       />
          <StatCard label="Total tokens" value={fmtTokens(totalTokens)} sub="prompt + completion" />
          <StatCard label="Avg latency"  value={fmtLatency(avgLatency)} />
          <StatCard label="Total cost"   value={fmtCost(totalCost)}  />
          <StatCard label="Error rate"   value={`${errorRate}%`}    />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading
          ? <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-600">
              <Loader2 size={16} className="animate-spin" />
              <span className="text-[13px]">Loading…</span>
            </div>
          : <TracesTable traces={traces} />
        }
      </div>
    </div>
  );
}

export default TracesPage;
