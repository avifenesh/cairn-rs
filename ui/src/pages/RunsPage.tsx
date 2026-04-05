import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { X, RefreshCw, ServerCrash, ChevronRight } from "lucide-react";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { defaultApi } from "../lib/api";
import type { RunRecord, RunState } from "../lib/types";

function fmtTime(ms: number) {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit",
  });
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

// ── Table ─────────────────────────────────────────────────────────────────────

function RunsTable({ runs, selectedId, onSelect }: {
  runs: RunRecord[]; selectedId: string | null; onSelect: (r: RunRecord) => void;
}) {
  if (runs.length === 0) return (
    <div className="flex flex-col items-center justify-center py-20 gap-2 text-zinc-600">
      <p className="text-[13px]">No runs match this filter</p>
    </div>
  );

  return (
    <table className="min-w-full">
      <thead className="sticky top-0 z-10 bg-zinc-950">
        <tr className="border-b border-zinc-800">
          {["Run ID","Session","State","Created","Updated"].map((h, i) => (
            <th key={h} className={clsx(
              "px-4 py-2 text-[11px] font-medium text-zinc-500 uppercase tracking-wider whitespace-nowrap",
              i >= 3 ? "text-right" : "text-left",
            )}>{h}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {runs.map((run, idx) => {
          const sel = run.run_id === selectedId;
          return (
            <tr key={run.run_id} onClick={() => onSelect(run)}
              className={clsx("cursor-pointer border-b border-zinc-800/40 h-9 transition-colors",
                sel ? "bg-zinc-800/60" : idx % 2 === 0 ? "bg-transparent hover:bg-zinc-900/60" : "bg-zinc-900/20 hover:bg-zinc-900/60",
              )}>
              <td className="px-4 py-0 font-mono text-[12px] text-zinc-300 whitespace-nowrap">
                <span className="flex items-center gap-1.5">
                  {sel && <ChevronRight size={11} className="text-indigo-400 shrink-0" />}
                  {shortId(run.run_id)}
                </span>
              </td>
              <td className="px-4 py-0 font-mono text-[11px] text-zinc-500 whitespace-nowrap">
                {shortId(run.session_id)}
              </td>
              <td className="px-4 py-0 whitespace-nowrap">
                <StateBadge state={run.state} compact />
              </td>
              <td className="px-4 py-0 text-[11px] text-zinc-500 whitespace-nowrap text-right">
                {fmtTime(run.created_at)}
              </td>
              <td className="px-4 py-0 text-[11px] text-zinc-500 whitespace-nowrap text-right">
                {fmtTime(run.updated_at)}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
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

  if (isError) return (
    <div className="flex flex-col items-center justify-center h-full gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500"/>
      <p className="text-[13px] font-medium text-zinc-300">Failed to load runs</p>
      <p className="text-[13px] text-zinc-600">{error instanceof Error ? error.message : "Unknown error"}</p>
      <button onClick={() => void refetch()} className="mt-1 px-3 py-1.5 rounded bg-zinc-800 text-[13px] text-zinc-300 hover:bg-zinc-700 transition-colors">Retry</button>
    </div>
  );

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
        <select value={filter} onChange={e => setFilter(e.target.value as RunState | "all")}
          className="rounded bg-zinc-900 border border-zinc-800 text-zinc-400 text-[12px] px-2 py-1 focus:outline-none focus:ring-1 focus:ring-indigo-500">
          <option value="all">All states</option>
          {ALL_STATES.map(s => <option key={s} value={s}>{STATE_LABEL[s]}</option>)}
        </select>
        <button onClick={() => void refetch()} disabled={isFetching}
          className="ml-auto flex items-center gap-1.5 rounded bg-zinc-900 border border-zinc-800 text-zinc-500 text-[12px] px-2.5 py-1 hover:text-zinc-200 hover:bg-zinc-800 disabled:opacity-40 transition-colors">
          <RefreshCw size={11} className={clsx(isFetching && "animate-spin")}/>Refresh
        </button>
      </div>
      {/* Content */}
      <div className="flex flex-1 overflow-hidden">
        <div className={clsx("flex-1 overflow-y-auto", selected && "border-r border-zinc-800")}>
          {isLoading ? <Skeleton /> : <RunsTable runs={filtered} selectedId={selected?.run_id ?? null} onSelect={r => setSelected(p => p?.run_id===r.run_id ? null : r)} />}
        </div>
        {selected && <DetailPanel run={selected} onClose={() => setSelected(null)} />}
      </div>
    </div>
  );
}

export default RunsPage;
