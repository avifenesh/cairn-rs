import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { X, RefreshCw, ChevronRight, Download } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { DataTable } from "../components/DataTable";
import { useTableKeyboard } from "../hooks/useTableKeyboard";
import { defaultApi } from "../lib/api";
import type { RunRecord, RunState } from "../lib/types";

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

// ── Page ──────────────────────────────────────────────────────────────────────

export function RunsPage() {
  const [filter, setFilter] = useState<RunState | "all">("all");
  const [selected, setSelected] = useState<RunRecord | null>(null);

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["runs"],
    queryFn: () => defaultApi.getRuns({ limit: 200 }),
    refetchInterval: 15_000,
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
          <button onClick={() => void refetch()} disabled={isFetching}
            className="flex items-center gap-1.5 rounded bg-zinc-900 border border-zinc-800 text-zinc-500 text-[12px] px-2.5 py-1 hover:text-zinc-200 hover:bg-zinc-800 disabled:opacity-40 transition-colors">
            <RefreshCw size={11} className={clsx(isFetching && "animate-spin")}/>Refresh
          </button>
        </div>
      </div>
      {/* Content */}
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
    </div>
  );

}

export default RunsPage;
