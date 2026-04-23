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
  Search, X, Download, ExternalLink,
} from "lucide-react";
import { clsx } from "clsx";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";
import { useVirtualScroll, DEFAULT_ROW_HEIGHT } from "../hooks/useVirtualScroll";
import type { LlmCallTrace } from "../lib/types";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";
import { useScope } from "../hooks/useScope";
import { StatCard } from "../components/StatCard";
import { Drawer } from "../components/Drawer";
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

function TraceRow({
  trace, even, onOpen,
}: {
  trace: LlmCallTrace;
  even: boolean;
  onOpen: (t: LlmCallTrace) => void;
}) {
  // Keep inner hash-links from also triggering the row's onClick.
  const stop = (e: React.MouseEvent) => e.stopPropagation();
  return (
    <tr
      data-virtual-row
      style={{ height: DEFAULT_ROW_HEIGHT }}
      onClick={() => onOpen(trace)}
      onKeyDown={e => {
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onOpen(trace); }
      }}
      tabIndex={0}
      role="button"
      aria-label={`Open trace ${trace.trace_id}`}
      className={clsx(
        ds.table.rowBorder, ds.table.rowHover,
        even ? ds.table.rowEven : ds.table.rowOdd,
        "cursor-pointer focus:outline-none focus:ring-1 focus:ring-indigo-500",
      )}
    >
      <td className="px-3 font-mono whitespace-nowrap text-[11px] hidden sm:table-cell"
          title={trace.session_id ?? ''}>
        {trace.session_id ? (
          <a
            href={`#session/${encodeURIComponent(trace.session_id)}`}
            onClick={stop}
            className="text-indigo-500 hover:text-indigo-400 hover:underline"
          >
            {shortId(trace.session_id)}
          </a>
        ) : (
          <span className="text-gray-500 dark:text-zinc-400">—</span>
        )}
      </td>
      <td className="px-3 font-mono whitespace-nowrap text-[11px] hidden md:table-cell"
          title={trace.run_id ?? ''}>
        {trace.run_id ? (
          <a
            href={`#run/${encodeURIComponent(trace.run_id)}`}
            onClick={stop}
            className="text-indigo-500 hover:text-indigo-400 hover:underline"
          >
            {shortId(trace.run_id)}
          </a>
        ) : (
          <span className="text-gray-500 dark:text-zinc-400">—</span>
        )}
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

// ── Detail drawer ─────────────────────────────────────────────────────────────

/**
 * TraceDetailDrawer — surfaces everything the server persists for a single
 * LLM call so operators don't have to open the JSON payload to triage.
 *
 * Prompt/response *bodies* are intentionally absent: cairn stores provider
 * call metadata (model, tokens, latency, cost, status) but never the prompt
 * or completion text itself — doing so would leak customer data through
 * /v1/traces.  This drawer shows the full metadata we do have and links
 * out to session/run pages for deeper context.
 */
function TraceDetailDrawer({
  trace, onClose,
}: {
  trace: LlmCallTrace | null;
  onClose: () => void;
}) {
  const t = trace;
  return (
    <Drawer
      open={t !== null}
      onClose={onClose}
      title={t ? `Trace — ${shortId(t.trace_id)}` : "Trace"}
      width="w-[28rem]"
    >
      {t && (
        <div className="p-4 space-y-4 text-[12px] text-gray-700 dark:text-zinc-300">
          <DetailRow label="Trace ID"  value={t.trace_id} mono copyable />
          <DetailRow label="Model"     value={t.model_id} />
          <DetailRow label="Provider"  value={inferProvider(t.model_id)} />

          <div className="grid grid-cols-2 gap-3">
            <DetailRow label="Latency"         value={fmtLatency(t.latency_ms)} />
            <DetailRow label="Cost"            value={fmtCost(t.cost_micros)} />
            <DetailRow label="Prompt tokens"   value={fmtTokens(t.prompt_tokens)} />
            <DetailRow label="Completion tokens" value={fmtTokens(t.completion_tokens)} />
          </div>

          <DetailRow label="Status" value={
            t.is_error ? (
              <span className="inline-flex items-center gap-1 text-red-400">
                <AlertTriangle size={11} /> error
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 text-emerald-400">
                <CheckCircle2 size={11} /> ok
              </span>
            )
          } />

          <DetailRow label="Timestamp" value={fmtTime(t.created_at_ms)} />

          <div className="border-t border-gray-200 dark:border-zinc-800 pt-3 space-y-2">
            <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-zinc-500">
              Context
            </div>
            <DetailLink label="Session" id={t.session_id} hash="session" />
            <DetailLink label="Run"     id={t.run_id}     hash="run" />
          </div>

          <div className="border-t border-gray-200 dark:border-zinc-800 pt-3">
            <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-zinc-500 mb-1.5">
              Prompt / response body
            </div>
            <p className="text-[11px] text-gray-500 dark:text-zinc-500 leading-relaxed">
              Cairn does not persist prompt or completion text in the trace log —
              only metadata. Open the parent session or run to see the full
              conversation transcript.
            </p>
          </div>
        </div>
      )}
    </Drawer>
  );
}

function DetailRow({
  label, value, mono, copyable,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
  copyable?: boolean;
}) {
  return (
    <div>
      <div className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-zinc-500">
        {label}
      </div>
      <div className={clsx(
        "mt-0.5 break-all",
        mono && "font-mono text-[11px]",
      )}>
        {value}
        {copyable && typeof value === "string" && (
          <button
            onClick={() => { void navigator.clipboard?.writeText(value); }}
            className="ml-2 text-[10px] text-indigo-500 hover:text-indigo-400"
            title="Copy"
          >
            copy
          </button>
        )}
      </div>
    </div>
  );
}

function DetailLink({
  label, id, hash,
}: {
  label: string;
  id: string | null;
  hash: "session" | "run";
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-[10px] uppercase tracking-wider text-gray-500 dark:text-zinc-500">
        {label}
      </span>
      {id ? (
        <a
          href={`#${hash}/${encodeURIComponent(id)}`}
          className="inline-flex items-center gap-1 font-mono text-[11px] text-indigo-500 hover:text-indigo-400 hover:underline"
          title={id}
        >
          {shortId(id)}
          <ExternalLink size={10} />
        </a>
      ) : (
        <span className="text-[11px] text-gray-500 dark:text-zinc-500">—</span>
      )}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function TracesPage() {
  const { ms: refreshMs, setOption: setRefreshOption, interval: refreshInterval } = useAutoRefresh("traces", "30s");
  const [scope] = useScope();

  const [filterQuery, setFilterQuery] = useState('');
  const [selected, setSelected]       = useState<LlmCallTrace | null>(null);

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    // Scope is part of the key so switching tenant/workspace/project
    // invalidates the cache instead of returning stale rows from a
    // different project.
    queryKey: [
      "traces",
      scope.tenant_id,
      scope.workspace_id,
      scope.project_id,
    ],
    queryFn:  () => defaultApi.getTraces({
      limit:        500,
      tenant_id:    scope.tenant_id,
      workspace_id: scope.workspace_id,
      project_id:   scope.project_id,
    }),
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
                <TH ch="Run"       hide="hidden md:table-cell" />
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
                  <td style={{ height: offsetY }} colSpan={9} />
                </tr>
              )}

              {filtered.length === 0 ? (
                <tr>
                  <td colSpan={9} className="px-4 py-12 text-center text-[13px] text-gray-400 dark:text-zinc-600">
                    No traces match this filter
                  </td>
                </tr>
              ) : (
                visibleItems.map(({ item, index }) => (
                  <TraceRow
                    key={item.trace_id}
                    trace={item}
                    even={index % 2 === 0}
                    onOpen={setSelected}
                  />
                ))
              )}

              {/* Bottom spacer — takes up the height of rows below the render window */}
              {bottomSpacerHeight > 0 && (
                <tr aria-hidden="true">
                  <td style={{ height: bottomSpacerHeight }} colSpan={9} />
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}

      <TraceDetailDrawer trace={selected} onClose={() => setSelected(null)} />
    </div>
  );
}

export default TracesPage;
