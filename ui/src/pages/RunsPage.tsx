import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { X, RefreshCw, ChevronRight, Download, Plus, Loader2, LayoutList, GanttChart } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { DataTable } from "../components/DataTable";
import { useTableKeyboard } from "../hooks/useTableKeyboard";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import type { RunRecord, RunState } from "../lib/types";
import { TimelineView, ZoomSelector } from "../components/TimelineView";
import type { ZoomLevel } from "../components/TimelineView";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";

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

const ALL_STATES: RunState[] = [
  "pending","running","paused","waiting_approval","waiting_dependency",
  "completed","failed","canceled",
];
const STATE_LABEL: Record<RunState, string> = {
  pending:"Pending", running:"Running", paused:"Paused",
  waiting_approval:"Awaiting Approval", waiting_dependency:"Waiting",
  completed:"Completed", failed:"Failed", canceled:"Canceled",
};

// ── Detail panel ──────────────────────────────────────────────────────────────

function FieldRow({ label, value, mono=false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-start justify-between px-3 py-2 gap-3 border-b border-zinc-800 last:border-0">
      <span className="text-[11px] text-zinc-500 shrink-0">{label}</span>
      <span className={clsx("text-[11px] text-zinc-300 text-right break-all", mono && "font-mono")}>{value}</span>
    </div>
  );
}
function SectionLabel({ children }: { children: React.ReactNode }) {
  return <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2">{children}</p>;
}

function DetailPanel({ run, onClose }: { run: RunRecord; onClose: () => void }) {
  return (
    <aside className="flex flex-col w-80 shrink-0 border-l border-zinc-800 bg-zinc-950 h-full overflow-y-auto">
      <div className="flex items-center gap-2 px-4 h-11 border-b border-zinc-800 sticky top-0 bg-zinc-950">
        <span className="text-[13px] font-medium font-mono text-zinc-200 truncate flex-1">
          {shortId(run.run_id)}
        </span>
        <button onClick={onClose} className="p-1 rounded text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800 transition-colors">
          <X size={14} />
        </button>
      </div>
      <div className="p-4 space-y-4">
        <div>
          <SectionLabel>State</SectionLabel>
          <StateBadge state={run.state} />
        </div>
        <div>
          <SectionLabel>Identifiers</SectionLabel>
          <div className="rounded-lg bg-zinc-900 border border-zinc-800">
            <FieldRow label="Run ID"    value={run.run_id}    mono />
            <FieldRow label="Session"   value={run.session_id} mono />
            {run.parent_run_id && <FieldRow label="Parent" value={run.parent_run_id} mono />}
          </div>
        </div>
        <div>
          <SectionLabel>Project</SectionLabel>
          <div className="rounded-lg bg-zinc-900 border border-zinc-800">
            <FieldRow label="Tenant"    value={run.project.tenant_id} />
            <FieldRow label="Workspace" value={run.project.workspace_id} />
            <FieldRow label="Project"   value={run.project.project_id} />
          </div>
        </div>
        {(run.failure_class || run.pause_reason) && (
          <div>
            <SectionLabel>Details</SectionLabel>
            <div className="rounded-lg bg-zinc-900 border border-zinc-800">
              {run.failure_class && <FieldRow label="Failure" value={run.failure_class} />}
              {run.pause_reason  && <FieldRow label="Paused"  value={run.pause_reason}  />}
            </div>
          </div>
        )}
        <div>
          <SectionLabel>Timestamps</SectionLabel>
          <div className="rounded-lg bg-zinc-900 border border-zinc-800">
            <FieldRow label="Created" value={fmtTime(run.created_at)} />
            <FieldRow label="Updated" value={fmtTime(run.updated_at)} />
          </div>
        </div>
      </div>
    </aside>
  );
}


function Skeleton() {
  return <div className="divide-y divide-zinc-800/40">
    {Array.from({length:10}).map((_,i)=>(
      <div key={i} className="flex items-center gap-4 px-4 h-9 animate-pulse">
        <div className="h-2.5 w-36 rounded bg-zinc-800"/>
        <div className="h-2.5 w-24 rounded bg-zinc-800"/>
        <div className="h-4 w-16 rounded bg-zinc-800"/>
        <div className="ml-auto h-2.5 w-28 rounded bg-zinc-800"/>
        <div className="h-2.5 w-28 rounded bg-zinc-800"/>
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
  const toast = useToast();

  const mutation = useMutation({
    mutationFn: () => {
      const runs = Array.from({ length: count }, (_, i) => ({
        session_id:   sessionId.trim() || "session_1",
        run_id:       prefix.trim() ? `${prefix.trim()}${i + 1}` : undefined,
      }));
      return defaultApi.batchCreateRuns(runs);
    },
    onSuccess: result => {
      const ok  = result.results.filter(r => r.ok).length;
      const bad = result.results.filter(r => !r.ok).length;
      if (ok > 0)  toast.success(`Created ${ok} run${ok !== 1 ? "s" : ""}.`);
      if (bad > 0) toast.error(`${bad} run${bad !== 1 ? "s" : ""} failed to create.`);
      onDone();
    },
    onError: () => toast.error("Batch create failed."),
  });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="w-full max-w-sm rounded-lg bg-zinc-900 border border-zinc-800 shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
          <h2 className="text-[13px] font-medium text-zinc-200">Batch Create Runs</h2>
          <button onClick={onClose} className="p-1 rounded text-zinc-600 hover:text-zinc-300 transition-colors">
            <X size={14} />
          </button>
        </div>

        {/* Form */}
        <div className="px-4 py-4 space-y-4">
          <div>
            <label className="block text-[11px] text-zinc-500 mb-1">Number of runs</label>
            <input
              type="number"
              min={1}
              max={50}
              value={count}
              onChange={e => setCount(Math.max(1, Math.min(50, Number(e.target.value))))}
              className="w-full rounded border border-zinc-700 bg-zinc-800 text-zinc-200 text-[13px]
                         px-3 py-2 focus:outline-none focus:border-indigo-500"
            />
            <p className="mt-1 text-[10px] text-zinc-600">Maximum 50 per batch</p>
          </div>

          <div>
            <label className="block text-[11px] text-zinc-500 mb-1">Session ID</label>
            <input
              type="text"
              value={sessionId}
              onChange={e => setSession(e.target.value)}
              placeholder="session_1"
              className="w-full rounded border border-zinc-700 bg-zinc-800 text-zinc-200 text-[13px]
                         px-3 py-2 focus:outline-none focus:border-indigo-500 font-mono"
            />
          </div>

          <div>
            <label className="block text-[11px] text-zinc-500 mb-1">Run ID prefix (optional)</label>
            <input
              type="text"
              value={prefix}
              onChange={e => setPrefix(e.target.value)}
              placeholder="run_batch_"
              className="w-full rounded border border-zinc-700 bg-zinc-800 text-zinc-200 text-[13px]
                         px-3 py-2 focus:outline-none focus:border-indigo-500 font-mono"
            />
            <p className="mt-1 text-[10px] text-zinc-600">
              Runs will be named {prefix || "auto"}{prefix ? "1" : ""} through {prefix || "auto"}{prefix ? count : ""}
            </p>
          </div>
        </div>

        {/* Actions */}
        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-zinc-800">
          <button
            onClick={onClose}
            className="rounded border border-zinc-700 text-zinc-400 text-[12px] px-3 py-1.5 hover:text-zinc-200 transition-colors"
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
            Create {count} run{count !== 1 ? "s" : ""}
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
  const [selected, setSelected]     = useState<RunRecord | null>(null);
  const [viewMode, setViewMode]     = useState<"table" | "timeline">("table");
  const [zoom, setZoom]             = useState<ZoomLevel>("6h");
  const [showBatchCreate, setShowBatchCreate] = useState(false);
  const qc = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["runs"],
    queryFn: () => defaultApi.getRuns({ limit: 200 }),
    refetchInterval: refreshMs,
  });

  const runs = data ?? [];
  const filtered = filter === "all" ? runs : runs.filter(r => r.state === filter);

  const kbd = useTableKeyboard({
    items:  filtered,
    getKey: r => r.run_id,
    onOpen: r => { window.location.hash = `run/${r.run_id}`; },
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
      <div className="flex items-center gap-3 px-4 h-11 border-b border-zinc-800 shrink-0 bg-zinc-950">
        <span className="text-[13px] font-medium text-zinc-200">
          Runs
          {!isLoading && <span className="ml-2 text-[11px] text-zinc-600 font-normal">
            {filtered.length}{filter !== "all" ? ` / ${runs.length}` : ""}
          </span>}
        </span>
        {selCount > 0 && (
          <span className="text-[11px] text-indigo-400 font-medium">
            {selCount} selected
          </span>
        )}
        <select value={filter} onChange={e => setFilter(e.target.value as RunState | "all")}
          className="rounded bg-zinc-900 border border-zinc-800 text-zinc-400 text-[12px] px-2 py-1 focus:outline-none focus:ring-1 focus:ring-indigo-500">
          <option value="all">All states</option>
          {ALL_STATES.map(s => <option key={s} value={s}>{STATE_LABEL[s]}</option>)}
        </select>
        {/* View toggle */}
        <div className="flex items-center rounded border border-zinc-700 overflow-hidden">
          <button onClick={() => setViewMode("table")} title="Table view"
            className={clsx("flex items-center gap-1 px-2.5 py-1 text-[11px] transition-colors",
              viewMode === "table" ? "bg-zinc-700 text-zinc-200" : "text-zinc-500 hover:text-zinc-300")}>
            <LayoutList size={12} /> Table
          </button>
          <button onClick={() => setViewMode("timeline")} title="Timeline view"
            className={clsx("flex items-center gap-1 px-2.5 py-1 text-[11px] border-l border-zinc-700 transition-colors",
              viewMode === "timeline" ? "bg-zinc-700 text-zinc-200" : "text-zinc-500 hover:text-zinc-300")}>
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
              className="flex items-center gap-1.5 rounded border border-zinc-700 bg-zinc-900 text-zinc-400 text-[12px] px-2.5 py-1 hover:text-zinc-200 hover:border-zinc-600 transition-colors"
            >
              <Download size={11} /> Export {selCount}
            </button>
          )}
          {selCount > 0 && (
            <button onClick={kbd.clearSelection} className="text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors px-1">
              Clear
            </button>
          )}
          <button
            onClick={() => setShowBatchCreate(true)}
            className="flex items-center gap-1.5 rounded border border-zinc-700 bg-zinc-900 text-zinc-400 text-[12px] px-2.5 py-1 hover:text-zinc-200 hover:border-indigo-600 transition-colors"
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
                className="appearance-none rounded border border-zinc-700 bg-zinc-900 text-[11px] font-mono pl-5 pr-2 h-7 text-zinc-400 focus:outline-none focus:border-indigo-500 transition-colors hover:border-zinc-600"
                title="Auto-refresh interval"
              >
                {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
              </select>
              {isFetching
                ? <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none"><RefreshCw size={9} className="animate-spin text-indigo-400" /></span>
                : <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none text-zinc-600"><RefreshCw size={9} /></span>
              }
            </div>
            <button onClick={() => refetch()} disabled={isFetching}
              className="flex items-center gap-1 h-7 px-2 rounded border border-zinc-700 bg-zinc-900 text-[11px] text-zinc-500 hover:text-zinc-200 hover:border-zinc-600 disabled:opacity-40 transition-colors"
              title="Refresh now"
            >
              <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
              <span className="hidden sm:inline">Refresh</span>
            </button>
          </div>
        </div>
      </div>
      {/* Content */}
      {viewMode === "timeline" ? (
        <div className="flex-1 overflow-y-auto bg-zinc-950">
          <TimelineView runs={filtered} zoom={zoom} />
        </div>
      ) : (
      <div className="flex flex-1 overflow-hidden">
        <div
          {...kbd.containerProps}
          className={clsx("flex-1 overflow-y-auto", selected && "border-r border-zinc-800", kbd.containerProps.className)}
        >
          {isLoading ? <Skeleton /> : (
          <DataTable<RunRecord>
            data={filtered}
            activeIndex={kbd.activeIndex}
            selectedIds={kbd.selectedKeys}
            getRowId={r => r.run_id}
            onRowClick={r => { window.location.hash = `run/${r.run_id}`; kbd.setActiveIndex(filtered.indexOf(r)); }}
            columns={[
              { key: 'run_id',    header: 'Run ID',    render: r => <span className="flex items-center gap-1.5 font-mono text-[12px] text-zinc-300 whitespace-nowrap" title={r.run_id}>{selected?.run_id===r.run_id&&<ChevronRight size={11} className="text-indigo-400 shrink-0"/>}{shortId(r.run_id)}</span>, sortValue: r => r.run_id },
              { key: 'session',   header: 'Session',   render: r => <span className="font-mono text-[11px] text-zinc-500 whitespace-nowrap" title={r.session_id}>{shortId(r.session_id)}</span> },
              { key: 'state',     header: 'State',     render: r => <StateBadge state={r.state} compact />,   sortValue: r => r.state },
              { key: 'created',   header: 'Created',   render: r => <span className="text-[11px] text-zinc-500 whitespace-nowrap tabular-nums text-right" title={fmtTime(r.created_at)}>{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at, headClass:'text-right', cellClass:'text-right' },
              { key: 'updated',   header: 'Updated',   render: r => <span className="text-[11px] text-zinc-500 whitespace-nowrap tabular-nums text-right" title={fmtTime(r.updated_at)}>{fmtRelative(r.updated_at)}</span>, sortValue: r => r.updated_at, headClass:'text-right', cellClass:'text-right' },
            ]}
            filterFn={(r, q) => r.run_id.includes(q) || r.session_id.includes(q) || r.state.includes(q)}
            csvRow={r => [r.run_id, r.session_id, r.state, r.parent_run_id??'', r.created_at, r.updated_at]}
            csvHeaders={['Run ID','Session ID','State','Parent Run','Created At','Updated At']}
            filename="runs"
            emptyText="No runs match this filter"
          />
        )}
        </div>
        {selected && <DetailPanel run={selected} onClose={() => setSelected(null)} />}
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
