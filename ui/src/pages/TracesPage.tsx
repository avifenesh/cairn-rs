/**
 * TracesPage — LLM call trace browser with virtual scrolling.
 *
 * Uses useVirtualScroll to render only the visible slice of potentially
 * thousands of trace rows.  Two spacer <tr> elements pad the table top/bottom
 * so the scrollbar thumb matches the full data size.
 */

import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  RefreshCw, Loader2, AlertTriangle, CheckCircle2,
  Search, X, Download,
} from "lucide-react";
import { clsx } from "clsx";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";
import { useVirtualScroll, DEFAULT_ROW_HEIGHT } from "../hooks/useVirtualScroll";
import type { LlmCallTrace } from "../lib/types";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";
import { StatCard } from "../components/StatCard";
import { ds } from "../lib/design-system";

// ── Helpers ───────────────────────────────────────────────────────────────────

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

function inferProvider(modelId: string): string {
  const m = modelId.toLowerCase();
  if (m.startsWith("gpt") || m.startsWith("o1") || m.startsWith("o3")) return "OpenAI";
  if (m.startsWith("claude"))   return "Anthropic";
  if (m.startsWith("gemini"))   return "Google";
  if (m.startsWith("llama") || m.startsWith("qwen") || m.startsWith("mistral") ||
      m.startsWith("nomic"))    return "Open-Weight";
  if (m.startsWith("titan") || m.startsWith("nova")) return "Bedrock";
  return "—";
}


// ── Column config ─────────────────────────────────────────────────────────────

const TH = ({ ch, right, hide }: { ch: string; right?: boolean; hide?: string }) => (
  <th className={clsx(
    right ? ds.table.thRight : ds.table.th,
    "bg-gray-50 dark:bg-zinc-900 sticky top-0 z-10",
    hide,
  )}>
    {ch}
  </th>
);

// ── Virtual trace row ─────────────────────────────────────────────────────────

function TraceRow({ trace, even }: { trace: LlmCallTrace; even: boolean }) {
  return (
    <tr
      data-virtual-row
      style={{ height: DEFAULT_ROW_HEIGHT }}
      className={clsx(
        ds.table.rowBorder, ds.table.rowHover,
        even ? ds.table.rowEven : ds.table.rowOdd,
      )}
    >
      <td className="px-3 font-mono text-gray-500 dark:text-zinc-400 whitespace-nowrap text-[11px] hidden sm:table-cell"
          title={trace.session_id ?? ''}>
        {shortId(trace.session_id ?? '—')}
      </td>
      <td className="px-3 text-xs text-gray-700 dark:text-zinc-300 whitespace-nowrap">
        {trace.model_id}
      </td>
      <td className="px-3 text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap hidden sm:table-cell">
        {inferProvider(trace.model_id)}
      </td>
      <td className={clsx(
        "px-3 text-[11px] whitespace-nowrap tabular-nums text-right",
        trace.latency_ms > 5_000 ? "text-amber-400" : "text-gray-500 dark:text-zinc-400",
      )}>
        {fmtLatency(trace.latency_ms)}
      </td>
      <td className="px-3 text-[11px] text-gray-500 dark:text-zinc-400 tabular-nums text-right whitespace-nowrap hidden md:table-cell">
        {fmtTokens(trace.prompt_tokens)} / {fmtTokens(trace.completion_tokens)}
      </td>
      <td className="px-3 text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums text-right whitespace-nowrap hidden sm:table-cell">
        {fmtCost(trace.cost_micros)}
      </td>
      <td className="px-3 whitespace-nowrap">
        {trace.is_error ? (
          <span className="inline-flex items-center gap-1 text-[10px] text-red-400">
            <AlertTriangle size={10} /> error
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 text-[10px] text-emerald-400">
            <CheckCircle2 size={10} /> ok
          </span>
        )}
      </td>
      <td className="px-3 text-[11px] text-gray-400 dark:text-zinc-600 tabular-nums whitespace-nowrap hidden sm:table-cell">
        {fmtTime(trace.created_at_ms)}
      </td>
    </tr>
  );
}

// ── CSV export ────────────────────────────────────────────────────────────────

function exportCsv(traces: LlmCallTrace[]) {
  const header = 'Session ID,Model,Provider,Latency (ms),Input Tokens,Output Tokens,Cost (µUSD),Error,Timestamp\n';
  const rows = traces.map(r =>
    [r.session_id ?? '', r.model_id, inferProvider(r.model_id),
     r.latency_ms, r.prompt_tokens, r.completion_tokens,
     r.cost_micros, r.is_error ? 'error' : '', r.created_at_ms]
    .map(v => `"${String(v).replace(/"/g, '""')}"`)
    .join(',')
  ).join('\n');
  const blob = new Blob([header + rows], { type: 'text/csv' });
  const url  = URL.createObjectURL(blob);
  const a    = document.createElement('a');
  a.href = url; a.download = 'traces.csv'; a.click();
  URL.revokeObjectURL(url);
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function TracesPage() {
  const { ms: refreshMs, setOption: setRefreshOption, interval: refreshInterval } = useAutoRefresh("traces", "30s");

  const [filterQuery, setFilterQuery] = useState('');

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["traces"],
    queryFn:  () => defaultApi.getTraces(500),
    refetchInterval: refreshMs,
  });

  const traces = data?.traces ?? [];

  // ── Client-side filter ────────────────────────────────────────────────────
  const filtered = useMemo(() => {
    const q = filterQuery.trim().toLowerCase();
    if (!q) return traces;
    return traces.filter(t =>
      t.model_id.toLowerCase().includes(q) ||
      (t.session_id ?? '').includes(q) ||
      inferProvider(t.model_id).toLowerCase().includes(q) ||
      (t.is_error ? 'error' : 'ok').includes(q),
    );
  }, [traces, filterQuery]);

  // ── Aggregate stats ───────────────────────────────────────────────────────
  const totalTokens = traces.reduce((s, t) => s + t.prompt_tokens + t.completion_tokens, 0);
  const totalCost   = traces.reduce((s, t) => s + t.cost_micros, 0);
  const avgLatency  = traces.length > 0
    ? Math.round(traces.reduce((s, t) => s + t.latency_ms, 0) / traces.length)
    : 0;
  const errorRate = traces.length > 0
    ? ((traces.filter(t => t.is_error).length / traces.length) * 100).toFixed(1)
    : "0.0";

  // ── Virtual scroll ────────────────────────────────────────────────────────
  const { containerRef, visibleItems, totalHeight, offsetY } = useVirtualScroll({
    items:     filtered,
    rowHeight: DEFAULT_ROW_HEIGHT,
    overscan:  20,
  });

  const rowHeight = DEFAULT_ROW_HEIGHT;
  const bottomSpacerHeight = Math.max(
    0,
    totalHeight - offsetY - visibleItems.length * rowHeight,
  );

  if (isError) return (
    <ErrorFallback error={error} resource="traces" onRetry={() => void refetch()} />
  );

  return (
    <div className={clsx("flex flex-col h-full", ds.surface.pageDense)}>
      {/* Toolbar */}
      <div className={clsx(ds.toolbar.base, ds.surface.pageDense)}>
        <span className={ds.toolbar.title}>
          LLM Traces
          {!isLoading && (
            <span className={ds.toolbar.count}>
              {filterQuery ? `${filtered.length} / ${traces.length}` : traces.length}
              {filtered.length > 0 && (
                <span className="ml-1.5 text-[10px] text-indigo-500">
                  virtual
                </span>
              )}
            </span>
          )}
        </span>

        {/* Search filter */}
        <div className="relative flex-1 max-w-xs">
          <Search size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
          <input
            value={filterQuery}
            onChange={e => setFilterQuery(e.target.value)}
            placeholder="Filter by model, session, provider…"
            className="w-full h-7 rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 text-[12px] text-gray-700 dark:text-zinc-300
                       placeholder-zinc-600 pl-7 pr-7 focus:outline-none focus:border-indigo-500 transition-colors"
          />
          {filterQuery && (
            <button
              onClick={() => setFilterQuery('')}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400"
            >
              <X size={11} />
            </button>
          )}
        </div>

        <button
          onClick={() => exportCsv(filtered)}
          disabled={filtered.length === 0}
          title="Export filtered traces as CSV"
          className={ds.btn.ghost}
        >
          <Download size={11} />
        </button>

        {/* Auto-refresh control */}
        <div className="flex items-center gap-1">
          <div className="relative">
            <select
              value={refreshInterval.option}
              onChange={e => setRefreshOption(e.target.value as import('../hooks/useAutoRefresh').RefreshOption)}
              className={ds.autoRefresh.select}
              title="Auto-refresh interval"
            >
              {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
            </select>
            {isFetching
              ? <span className={ds.autoRefresh.iconWrap}><RefreshCw size={9} className="animate-spin text-indigo-400" /></span>
              : <span className={clsx(ds.autoRefresh.iconWrap, "text-gray-400 dark:text-zinc-600")}><RefreshCw size={9} /></span>
            }
          </div>
          <button onClick={() => refetch()} disabled={isFetching}
            className={ds.btn.secondary}
            title="Refresh now"
          >
            <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
            <span className="hidden sm:inline">Refresh</span>
          </button>
        </div>
      </div>

      {/* Stat strip */}
      {!isLoading && traces.length > 0 && (
        <div className="grid grid-cols-2 sm:grid-cols-5 gap-3 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 shrink-0">
          <StatCard label="Calls"        value={traces.length} variant="info" />
          <StatCard label="Total tokens" value={fmtTokens(totalTokens)} description="prompt + completion" variant="info" />
          <StatCard label="Avg latency"  value={fmtLatency(avgLatency)} variant="info" />
          <StatCard label="Total cost"   value={fmtCost(totalCost)} variant="info" />
          <StatCard label="Error rate"   value={`${errorRate}%`} variant="info" />
        </div>
      )}

      {/* Virtualized table */}
      {isLoading ? (
        <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
          <Loader2 size={16} className="animate-spin" />
          <span className="text-[13px]">Loading…</span>
        </div>
      ) : (
        // containerRef attaches to the scrollable div — useVirtualScroll tracks its scrollTop
        <div
          ref={containerRef}
          className="flex-1 overflow-y-auto overflow-x-auto"
          aria-rowcount={filtered.length}
        >
          <table className="w-full min-w-[600px] text-[13px] border-collapse">
            <thead>
              <tr>
                <TH ch="Session"   hide="hidden sm:table-cell" />
                <TH ch="Model" />
                <TH ch="Provider"  hide="hidden sm:table-cell" />
                <TH ch="Latency"   right />
                <TH ch="Tokens"    right hide="hidden md:table-cell" />
                <TH ch="Cost"      right hide="hidden sm:table-cell" />
                <TH ch="Status" />
                <TH ch="Timestamp" hide="hidden sm:table-cell" />
              </tr>
            </thead>
            <tbody>
              {/* Top spacer — takes up the height of rows above the render window */}
              {offsetY > 0 && (
                <tr aria-hidden="true">
                  <td style={{ height: offsetY }} colSpan={8} />
                </tr>
              )}

              {filtered.length === 0 ? (
                <tr>
                  <td colSpan={8} className="px-4 py-12 text-center text-[13px] text-gray-400 dark:text-zinc-600">
                    No traces match this filter
                  </td>
                </tr>
              ) : (
                visibleItems.map(({ item, index }) => (
                  <TraceRow key={item.trace_id} trace={item} even={index % 2 === 0} />
                ))
              )}

              {/* Bottom spacer — takes up the height of rows below the render window */}
              {bottomSpacerHeight > 0 && (
                <tr aria-hidden="true">
                  <td style={{ height: bottomSpacerHeight }} colSpan={8} />
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

export default TracesPage;
