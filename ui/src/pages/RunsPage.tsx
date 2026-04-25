import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { X, RefreshCw, Download, Plus, Loader2, LayoutList, GanttChart } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { DataTable } from "../components/DataTable";
import { useTableKeyboard } from "../hooks/useTableKeyboard";
import { useToast } from "../components/Toast";
import { CopyButton } from "../components/CopyButton";
import { defaultApi, ApiError } from "../lib/api";
import type { RunRecord, RunState, StuckRunReport } from "../lib/types";
import { TimelineView, ZoomSelector } from "../components/TimelineView";
import type { ZoomLevel } from "../components/TimelineView";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";
import { EmptyScopeHint } from "../components/EmptyScopeHint";
import { EntityExplainer } from "../components/EntityExplainer";
import { ENTITY_EXPLAINERS } from "../lib/entityExplainers";

function fmtTime(ms: number) {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
}
function fmtRelative(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000)       return "just now";
  if (d < 3_600_000)    return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)   return `${Math.floor(d / 3_600_000)}h ago`;
  if (d < 604_800_000)  return `${Math.floor(d / 86_400_000)}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
function shortId(id: string) {
  return id.length > 20 ? `${id.slice(0, 8)}\u2026${id.slice(-5)}` : id;
}

/** Synthesise a minimal RunRecord from a stalled-run diagnosis report
 *  so the Runs table can render rows whose full record lives outside
 *  the first page of GET /v1/runs. Fields the diagnosis report does
 *  not carry are filled with safe placeholders; clicking the row
 *  lands on RunDetailPage which refetches the complete record. */
function synthStalledRun(s: StuckRunReport): RunRecord {
  const now = Date.now();
  const createdAt = Math.max(0, now - (s.duration_ms || 0));
  return {
    run_id:            s.run_id,
    session_id:        "\u2014",
    parent_run_id:     null,
    project:           { tenant_id: "", workspace_id: "", project_id: "" },
    state:             s.state as RunState,
    prompt_release_id: null,
    agent_role_id:     null,
    failure_class:     null,
    pause_reason:      null,
    resume_trigger:    null,
    version:           0,
    created_at:        createdAt,
    updated_at:        s.last_event_ms > 0 ? s.last_event_ms : createdAt,
  };
}


const ALL_STATES: RunState[] = [
  "pending","running","paused","waiting_approval","waiting_dependency",
  "completed","failed","canceled",
];
const STATE_LABEL: Record<RunState, string> = {
  pending:"Pending", running:"Running", paused:"Paused",
  waiting_approval:"Awaiting Approval", waiting_dependency:"Waiting",
  completed:"Completed", failed:"Failed", canceled:"Canceled",
};

function Skeleton() {
  return <div className="divide-y divide-gray-200 dark:divide-zinc-800/40">
    {Array.from({length:10}).map((_,i)=>(
      <div key={i} className="flex items-center gap-4 px-4 h-9 animate-pulse">
        <div className="h-2.5 w-36 rounded bg-gray-100 dark:bg-zinc-800"/>
        <div className="h-2.5 w-24 rounded bg-gray-100 dark:bg-zinc-800"/>
        <div className="h-4 w-16 rounded bg-gray-100 dark:bg-zinc-800"/>
        <div className="ml-auto h-2.5 w-28 rounded bg-gray-100 dark:bg-zinc-800"/>
        <div className="h-2.5 w-28 rounded bg-gray-100 dark:bg-zinc-800"/>
      </div>
    ))}
  </div>;
}

// ── Batch create modal ────────────────────────────────────────────────────────

interface BatchCreateModalProps {
  onClose: () => void;
  onDone: () => void;
}

function BatchCreateModal({ onClose, onDone }: BatchCreateModalProps) {
  const [count, setCount]       = useState(3);
  const [sessionId, setSession] = useState("session_1");
  const [prefix, setPrefix]     = useState("run_batch_");
  const [planMode, setPlanMode] = useState(false);
  const toast = useToast();

  const mutation = useMutation({
    mutationFn: async () => {
      const sid = sessionId.trim() || `sess_${Date.now()}`;
      const pfx = prefix.trim() || `run-${Date.now()}-`;

      // Ensure session exists before creating runs. A 409 means the
      // session already exists — the desired end-state, so swallow it.
      // Any other failure (auth / scope / validation / network) must
      // surface through onError so the operator sees the real cause
      // instead of N confusing "session not found" per-run errors.
      try {
        await defaultApi.createSession({ session_id: sid });
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 409)) throw e;
      }

      const runs = Array.from({ length: count }, (_, i) => ({
        session_id: sid,
        run_id:     `${pfx}${i + 1}`,
        mode:       planMode ? { type: "plan" as const } : undefined,
      }));
      // #174 — issue one POST /v1/runs/batch instead of N createRun calls.
      return defaultApi.batchCreateRuns(runs);
    },
    onSuccess: result => {
      const ok  = result.results.filter(r => r.ok).length;
      const bad = result.results.filter(r => !r.ok);
      if (ok > 0) toast.success(`Created ${ok} run${ok !== 1 ? "s" : ""}.`);
      if (bad.length > 0) {
        const sample = bad[0]?.error ?? "run creation failed";
        toast.error(
          bad.length === 1
            ? `1 run failed: ${sample}`
            : `${bad.length} runs failed (first: ${sample})`,
        );
      }
      onDone();
    },
    onError: e => toast.error(e instanceof Error ? e.message : "Batch create failed."),
  });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="w-full max-w-sm rounded-lg bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
          <h2 className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">Batch Create Runs</h2>
          <button onClick={onClose} className="p-1 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <X size={14} />
          </button>
        </div>

        {/* Form */}
        <div className="px-4 py-4 space-y-4">
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1">Number of runs</label>
            <input
              type="number"
              min={1}
              max={50}
              value={count}
              onChange={e => setCount(Math.max(1, Math.min(50, Number(e.target.value))))}
              className="w-full rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800 text-gray-800 dark:text-zinc-200 text-[13px]
                         px-3 py-2 focus:outline-none focus:border-indigo-500"
            />
            <p className="mt-1 text-[10px] text-gray-400 dark:text-zinc-600">Maximum 50 per batch</p>
          </div>

          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1">Session ID</label>
            <input
              type="text"
              value={sessionId}
              onChange={e => setSession(e.target.value)}
              placeholder="session_1"
              className="w-full rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800 text-gray-800 dark:text-zinc-200 text-[13px]
                         px-3 py-2 focus:outline-none focus:border-indigo-500 font-mono"
            />
          </div>

          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1">Run ID prefix (optional)</label>
            <input
              type="text"
              value={prefix}
              onChange={e => setPrefix(e.target.value)}
              placeholder="run_batch_"
              className="w-full rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800 text-gray-800 dark:text-zinc-200 text-[13px]
                         px-3 py-2 focus:outline-none focus:border-indigo-500 font-mono"
            />
            <p className="mt-1 text-[10px] text-gray-400 dark:text-zinc-600">
              Runs will be named {prefix || "auto"}{prefix ? "1" : ""} through {prefix || "auto"}{prefix ? count : ""}
            </p>
          </div>

          <label className="flex items-start gap-3 rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800/80 px-3 py-2 cursor-pointer">
            <input
              type="checkbox"
              checked={planMode}
              onChange={e => setPlanMode(e.target.checked)}
              className="mt-0.5 rounded border-gray-300 dark:border-zinc-600 text-indigo-600 focus:ring-indigo-500"
            />
            <span className="space-y-1">
              <span className="block text-[12px] font-medium text-gray-800 dark:text-zinc-200">Plan Mode</span>
              <span className="block text-[10px] text-gray-400 dark:text-zinc-500">
                New runs stay in plan mode so the Run Detail page opens with the review panel visible.
              </span>
            </span>
          </label>
        </div>

        {/* Actions */}
        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-gray-200 dark:border-zinc-800">
          <button
            onClick={onClose}
            className="rounded border border-gray-200 dark:border-zinc-700 text-gray-500 dark:text-zinc-400 text-[12px] px-3 py-1.5 hover:text-gray-800 dark:hover:text-zinc-200 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending || count < 1}
            className="flex items-center gap-1.5 rounded bg-indigo-600 hover:bg-indigo-500
                       text-white text-[12px] font-medium px-3 py-1.5 disabled:opacity-40 transition-colors"
          >
            {mutation.isPending ? <Loader2 size={12} className="animate-spin" /> : <Plus size={12} />}
            Create {count} {planMode ? "plan " : ""}run{count !== 1 ? "s" : ""}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function RunsPage() {
  const { ms: refreshMs, setOption: setRefreshOption, interval: refreshInterval } = useAutoRefresh("runs", "15s");

  const [filter, setFilter]         = useState<RunState | "all">("all");
  const [viewMode, setViewMode]     = useState<"table" | "timeline">("table");
  const [zoom, setZoom]             = useState<ZoomLevel>("6h");
  const [showBatchCreate, setShowBatchCreate] = useState(false);
  // F29 CE — "Stalled only" toggle. Swaps the query source from
  // `GET /v1/runs` to `GET /v1/runs/stalled`. Defaults to the URL
  // hash query so the StuckRunsWidget "view all" link lands here
  // with the filter already engaged.
  const [stalledOnly, setStalledOnly] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.location.hash.includes("stalled=1");
  });
  const qc = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery<RunRecord[]>({
    queryKey: ["runs", stalledOnly ? "stalled" : "all"],
    queryFn: async (): Promise<RunRecord[]> => {
      if (!stalledOnly) return defaultApi.getRuns({ limit: 200 });
      // Join stalled-diagnosis reports back against the normal runs list
      // so the downstream DataTable keeps the full RunRecord shape
      // (created_at, session_id, parent_run_id, etc.). Falls back to
      // synthesised minimal RunRecord rows when a stalled run isn't in
      // the first page of `/v1/runs` (long-running stuck runs past the
      // default limit of 200).
      const [stalled, runs] = await Promise.all([
        defaultApi.getStalledRuns(),
        defaultApi.getRuns({ limit: 200 }),
      ]);
      const byId = new Map<string, RunRecord>();
      for (const r of runs) byId.set(r.run_id, r);
      return stalled.map((s: StuckRunReport) => byId.get(s.run_id) ?? synthStalledRun(s));
    },
    refetchInterval: refreshMs,
  });

  const runs = data ?? [];
  const filtered = filter === "all" ? runs : runs.filter(r => r.state === filter);

  const kbd = useTableKeyboard({
    items:  filtered,
    getKey: r => r.run_id,
    onOpen: r => { window.location.hash = `run/${encodeURIComponent(r.run_id)}`; },
  });

  function exportSelected() {
    const toExport = filtered.filter(r => kbd.selectedKeys.has(r.run_id));
    const blob = new Blob([JSON.stringify({
      version: '1.0', type: 'runs_export',
      exported_at: new Date().toISOString(),
      data: { runs: toExport },
    }, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a   = document.createElement('a');
    a.href = url; a.download = 'runs-export.json'; a.click();
    URL.revokeObjectURL(url);
    kbd.clearSelection();
  }

  if (isError) return (
    <ErrorFallback error={error} resource="runs" onRetry={() => void refetch()} />
  );

  const selCount = kbd.selectedKeys.size;

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Runs
          {!isLoading && <span className="ml-2 text-[11px] text-gray-400 dark:text-zinc-600 font-normal">
            {filtered.length}{filter !== "all" ? ` / ${runs.length}` : ""}
          </span>}
        </span>
        {selCount > 0 && (
          <span className="text-[11px] text-indigo-400 font-medium">
            {selCount} selected
          </span>
        )}
        <select value={filter} onChange={e => setFilter(e.target.value as RunState | "all")}
          className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] px-2 py-1 focus:outline-none focus:ring-1 focus:ring-indigo-500">
          <option value="all">All states</option>
          {ALL_STATES.map(s => <option key={s} value={s}>{STATE_LABEL[s]}</option>)}
        </select>
        {/* F29 CE — Stalled-only pill. Swaps the query source from
            /v1/runs to /v1/runs/stalled. Syncs with the URL hash so
            the StuckRunsWidget "view all" link lands here with the
            filter engaged. */}
        <button
          type="button"
          aria-pressed={stalledOnly}
          data-testid="runs-stalled-toggle"
          onClick={() => {
            const next = !stalledOnly;
            setStalledOnly(next);
            const hash = window.location.hash.replace(/^#/, "").split("?")[0] || "runs";
            window.location.hash = next ? `${hash}?stalled=1` : hash;
          }}
          title="Show only runs stalled for more than the system-default stuck-run threshold."
          className={clsx(
            "inline-flex items-center rounded border text-[12px] px-2 py-1 transition-colors",
            stalledOnly
              ? "border-amber-500/50 bg-amber-500/10 text-amber-500 dark:text-amber-400"
              : "border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-gray-500 dark:text-zinc-400 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600",
          )}
        >
          Stalled only
        </button>
        {/* View toggle */}
        <div className="flex items-center rounded border border-gray-200 dark:border-zinc-700 overflow-hidden">
          <button onClick={() => setViewMode("table")} title="Table view"
            className={clsx("flex items-center gap-1 px-2.5 py-1 text-[11px] transition-colors",
              viewMode === "table" ? "bg-gray-200 dark:bg-zinc-700 text-gray-800 dark:text-zinc-200" : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300")}>
            <LayoutList size={12} /> Table
          </button>
          <button onClick={() => setViewMode("timeline")} title="Timeline view"
            className={clsx("flex items-center gap-1 px-2.5 py-1 text-[11px] border-l border-gray-200 dark:border-zinc-700 transition-colors",
              viewMode === "timeline" ? "bg-gray-200 dark:bg-zinc-700 text-gray-800 dark:text-zinc-200" : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300")}>
            <GanttChart size={12} /> Timeline
          </button>
        </div>
        {viewMode === "timeline" && (
          <ZoomSelector value={zoom} onChange={setZoom} />
        )}
        <div className="ml-auto flex items-center gap-2">
          {selCount > 0 && (
            <button
              onClick={exportSelected}
              className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-gray-500 dark:text-zinc-400 text-[12px] px-2.5 py-1 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600 transition-colors"
            >
              <Download size={11} /> Export {selCount}
            </button>
          )}
          {selCount > 0 && (
            <button onClick={kbd.clearSelection} className="text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors px-1">
              Clear
            </button>
          )}
          <button
            onClick={() => setShowBatchCreate(true)}
            className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-gray-500 dark:text-zinc-400 text-[12px] px-2.5 py-1 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-indigo-600 transition-colors"
            title="Create multiple runs at once"
          >
            <Plus size={11} /> Batch Create
          </button>
          {/* Auto-refresh control */}
          <div className="flex items-center gap-1">
            <div className="relative">
              <select
                value={refreshInterval.option}
                onChange={e => setRefreshOption(e.target.value as import('../hooks/useAutoRefresh').RefreshOption)}
                className="appearance-none rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] font-mono pl-5 pr-2 h-7 text-gray-500 dark:text-zinc-400 focus:outline-none focus:border-indigo-500 transition-colors hover:border-zinc-600"
                title="Auto-refresh interval"
              >
                {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
              </select>
              {isFetching
                ? <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none"><RefreshCw size={9} className="animate-spin text-indigo-400" /></span>
                : <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none text-gray-400 dark:text-zinc-600"><RefreshCw size={9} /></span>
              }
            </div>
            <button onClick={() => refetch()} disabled={isFetching}
              className="flex items-center gap-1 h-7 px-2 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600 disabled:opacity-40 transition-colors"
              title="Refresh now"
            >
              <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
              <span className="hidden sm:inline">Refresh</span>
            </button>
          </div>
        </div>
      </div>
      {/* F32 — inline entity explainer. */}
      <div className="px-4 py-1.5 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
        <EntityExplainer>{ENTITY_EXPLAINERS.runsList}</EntityExplainer>
      </div>
      {/* Content */}
      {viewMode === "timeline" ? (
        <div className="flex-1 overflow-y-auto bg-white dark:bg-zinc-950">
          <TimelineView runs={filtered} zoom={zoom} />
        </div>
      ) : (
      <div className="flex flex-1 overflow-hidden">
        <div
          {...kbd.containerProps}
          className={clsx("flex-1 overflow-y-auto", kbd.containerProps.className)}
        >
          {isLoading ? <Skeleton /> : (
          <DataTable<RunRecord>
            data={filtered}
            activeIndex={kbd.activeIndex}
            selectedIds={kbd.selectedKeys}
            getRowId={r => r.run_id}
            onRowClick={r => { window.location.hash = `run/${encodeURIComponent(r.run_id)}`; kbd.setActiveIndex(filtered.indexOf(r)); }}
            columns={[
              { key: 'run_id',    header: 'Run ID',    render: r => <span className="flex items-center gap-1 font-mono text-[12px] text-gray-700 dark:text-zinc-300 whitespace-nowrap group/id" title={r.run_id}>{shortId(r.run_id)}<CopyButton text={r.run_id} label="Copy run ID" size={10} className="opacity-0 group-hover/id:opacity-100" /></span>, sortValue: r => r.run_id },
              { key: 'session',   header: 'Session',   render: r => <span className="flex items-center gap-1 font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap group/id" title={r.session_id}>{shortId(r.session_id)}<CopyButton text={r.session_id} label="Copy session ID" size={10} className="opacity-0 group-hover/id:opacity-100" /></span> },
              { key: 'state',     header: 'State',     render: r => <StateBadge state={r.state} compact />,   sortValue: r => r.state },
              { key: 'created',   header: 'Created',   render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap tabular-nums text-right" title={fmtTime(r.created_at)}>{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at, headClass:'text-right', cellClass:'text-right' },
              { key: 'updated',   header: 'Updated',   render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap tabular-nums text-right" title={fmtTime(r.updated_at)}>{fmtRelative(r.updated_at)}</span>, sortValue: r => r.updated_at, headClass:'text-right', cellClass:'text-right' },
            ]}
            filterFn={(r, q) => r.run_id.includes(q) || r.session_id.includes(q) || r.state.includes(q)}
            csvRow={r => [r.run_id, r.session_id, r.state, r.parent_run_id??'', r.created_at, r.updated_at]}
            csvHeaders={['Run ID','Session ID','State','Parent Run','Created At','Updated At']}
            filename="runs"
            emptyText="No runs match this filter — try 'All states' or create a session first"
          />
        )}
        <EmptyScopeHint empty={!isLoading && runs.length === 0} />
        </div>
      </div>
      )}
      {showBatchCreate && (
        <BatchCreateModal
          onClose={() => setShowBatchCreate(false)}
          onDone={() => {
            setShowBatchCreate(false);
            void qc.invalidateQueries({ queryKey: ["runs"] });
          }}
        />
      )}
    </div>
  );

}

export default RunsPage;
