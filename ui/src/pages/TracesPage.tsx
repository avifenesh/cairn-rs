import { useQuery } from "@tanstack/react-query";
import { RefreshCw, Loader2, ServerCrash, AlertTriangle, CheckCircle2 } from "lucide-react";
import { DataTable } from "../components/DataTable";
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
        <div className="grid grid-cols-2 sm:grid-cols-5 gap-x-6 gap-y-3 px-5 py-3 border-b border-zinc-800 bg-zinc-900 shrink-0">
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
          : (
          <DataTable<LlmCallTrace>
            data={traces}
            columns={[
              { key: 'session',    header: 'Session',   render: r => <span className="font-mono text-[11px] text-zinc-400 whitespace-nowrap">{shortId(r.session_id ?? '')}</span>,      sortValue: r => r.session_id ?? '' },
              { key: 'model',      header: 'Model',     render: r => <span className="text-xs text-zinc-300 whitespace-nowrap">{r.model_id}</span>,                             sortValue: r => r.model_id },
              { key: 'provider',   header: 'Provider',  render: r => <span className="text-[11px] text-zinc-500 whitespace-nowrap">{inferProvider(r.model_id)}</span> },
              { key: 'latency',    header: 'Latency',   render: r => <span className={`text-[11px] tabular-nums ${r.latency_ms > 5000 ? 'text-amber-400' : 'text-zinc-400'}`}>{fmtLatency(r.latency_ms)}</span>, sortValue: r => r.latency_ms },
              { key: 'tokens',     header: 'Tokens',    render: r => <span className="text-[11px] text-zinc-400 tabular-nums whitespace-nowrap">{fmtTokens(r.prompt_tokens)} / {fmtTokens(r.completion_tokens)}</span>, sortValue: r => (r.prompt_tokens + r.completion_tokens) },
              { key: 'cost',       header: 'Cost',      render: r => <span className="text-[11px] text-zinc-500 tabular-nums">{fmtCost(r.cost_micros)}</span>,                  sortValue: r => r.cost_micros },
              { key: 'status',     header: 'Status',    render: r => r.is_error ? <span className="inline-flex items-center gap-1 text-[10px] text-red-400"><AlertTriangle size={10}/>error</span> : <span className="inline-flex items-center gap-1 text-[10px] text-emerald-400"><CheckCircle2 size={10}/>ok</span>, sortValue: r => r.is_error ? 1 : 0 },
              { key: 'timestamp',  header: 'Timestamp', render: r => <span className="text-[11px] text-zinc-600 tabular-nums whitespace-nowrap">{fmtTime(r.created_at_ms)}</span>,  sortValue: r => r.created_at_ms },
            ]}
            filterFn={(r, q) => r.model_id.toLowerCase().includes(q) || (r.session_id ?? '').includes(q) || inferProvider(r.model_id).toLowerCase().includes(q)}
            csvRow={r => [(r.session_id ?? ''), r.model_id, inferProvider(r.model_id), r.latency_ms, r.prompt_tokens, r.completion_tokens, r.cost_micros, r.is_error ? 'error' : '', r.created_at_ms]}
            csvHeaders={['Session ID', 'Model', 'Provider', 'Latency (ms)', 'Input Tokens', 'Output Tokens', 'Cost (µUSD)', 'Error', 'Timestamp']}
            filename="traces"
            emptyText="No traces recorded yet"
          />
        )
        }
      </div>
    </div>
  );
}

export default TracesPage;
