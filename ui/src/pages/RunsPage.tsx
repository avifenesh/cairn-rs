import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  X, RefreshCw, ServerCrash, Inbox, ChevronRight,
  Pause, Play, Loader2, Clock, ListChecks, DollarSign, Activity,
} from "lucide-react";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { defaultApi } from "../lib/api";
import type { RunRecord, RunState, TaskRecord, TaskState } from "../lib/types";

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

function shortId(id: string): string {
  return id.length > 20 ? `${id.slice(0, 8)}\u2026${id.slice(-6)}` : id;
}

// ── State filter options ──────────────────────────────────────────────────────

const ALL_STATES: RunState[] = [
  "pending",
  "running",
  "paused",
  "waiting_approval",
  "waiting_dependency",
  "completed",
  "failed",
  "canceled",
];

const STATE_FILTER_LABEL: Record<RunState, string> = {
  pending:            "Pending",
  running:            "Running",
  paused:             "Paused",
  waiting_approval:   "Awaiting Approval",
  waiting_dependency: "Waiting",
  completed:          "Completed",
  failed:             "Failed",
  canceled:           "Canceled",
};

// ── Detail panel ─────────────────────────────────────────────────────────────

interface DetailPanelProps {
  run: RunRecord;
  onClose: () => void;
}

function DetailPanel({ run, onClose }: DetailPanelProps) {
  return (
    <aside className="flex flex-col w-96 shrink-0 border-l border-zinc-800 bg-zinc-900 h-full overflow-y-auto">
      {/* Header */}
      <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800 sticky top-0 bg-zinc-900 z-10">
        <div className="flex items-center gap-2 min-w-0">
          <ChevronRight size={14} className="text-zinc-500 shrink-0" />
          <span className="text-sm font-semibold text-zinc-100 font-mono truncate">
            {shortId(run.run_id)}
          </span>
        </div>
        <button
          onClick={onClose}
          className="rounded-md p-1 text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
          aria-label="Close panel"
        >
          <X size={16} />
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 p-5 space-y-5">
        {/* State */}
        <div>
          <label className="text-xs text-zinc-500 uppercase tracking-widest">State</label>
          <div className="mt-1.5">
            <StateBadge state={run.state} />
          </div>
        </div>

        {/* IDs */}
        <Section title="Identifiers">
          <Field label="Run ID"     value={run.run_id} mono />
          <Field label="Session"    value={run.session_id} mono />
          {run.parent_run_id && (
            <Field label="Parent Run" value={run.parent_run_id} mono />
          )}
        </Section>

        {/* Project */}
        <Section title="Project">
          <Field label="Tenant"    value={run.project.tenant_id} />
          <Field label="Workspace" value={run.project.workspace_id} />
          <Field label="Project"   value={run.project.project_id} />
        </Section>

        {/* Prompt / Agent */}
        {(run.prompt_release_id || run.agent_role_id) && (
          <Section title="Prompt &amp; Agent">
            {run.prompt_release_id && (
              <Field label="Prompt Release" value={run.prompt_release_id} mono />
            )}
            {run.agent_role_id && (
              <Field label="Agent Role" value={run.agent_role_id} mono />
            )}
          </Section>
        )}

        {/* Failure info */}
        {(run.failure_class || run.pause_reason) && (
          <Section title="Status Details">
            {run.failure_class && (
              <Field label="Failure Class" value={run.failure_class} />
            )}
            {run.pause_reason && (
              <Field label="Pause Reason" value={run.pause_reason} />
            )}
            {run.resume_trigger && (
              <Field label="Resume Trigger" value={run.resume_trigger} />
            )}
          </Section>
        )}

        {/* Timestamps */}
        <Section title="Timestamps">
          <Field label="Created" value={fmtTime(run.created_at)} />
          <Field label="Updated" value={fmtTime(run.updated_at)} />
          <Field label="Version" value={String(run.version)} />
        </Section>
      </div>
    </aside>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-xs text-zinc-500 uppercase tracking-widest mb-2">{title}</p>
      <div className="rounded-lg bg-zinc-800/50 ring-1 ring-zinc-700/50 divide-y divide-zinc-700/40">
        {children}
      </div>
    </div>
  );
}

function Field({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-start justify-between px-3 py-2 gap-3">
      <span className="text-xs text-zinc-500 shrink-0 pt-0.5">{label}</span>
      <span className={clsx(
        "text-xs text-zinc-300 text-right break-all",
        mono && "font-mono",
      )}>
        {value}
      </span>
    </div>
  );
}

// ── Runs table ────────────────────────────────────────────────────────────────

interface TableProps {
  runs: RunRecord[];
  selectedId: string | null;
  onSelect: (run: RunRecord) => void;
}

function RunsTable({ runs, selectedId, onSelect }: TableProps) {
  if (runs.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-24 text-center gap-3">
        <Inbox size={36} className="text-zinc-700" />
        <p className="text-sm text-zinc-400">No runs match this filter</p>
        <p className="text-xs text-zinc-600">Try selecting a different state or clear the filter</p>
      </div>
    );
  }

  return (
    <div className="overflow-x-auto">
      <table className="min-w-full text-sm">
        <thead>
          <tr className="border-b border-zinc-800">
            {["Run ID", "Session", "State", "Created", "Updated"].map((h) => (
              <th
                key={h}
                className="px-4 py-3 text-left text-xs font-medium text-zinc-500 uppercase tracking-widest whitespace-nowrap"
              >
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-zinc-800/60">
          {runs.map((run) => {
            const selected = run.run_id === selectedId;
            return (
              <tr
                key={run.run_id}
                onClick={() => onSelect(run)}
                className={clsx(
                  "cursor-pointer transition-colors",
                  selected ? "bg-zinc-800" : "hover:bg-zinc-900/70",
                )}
              >
                <td className="px-4 py-3 font-mono text-zinc-300 whitespace-nowrap">
                  <span className="flex items-center gap-1.5">
                    {selected && (
                      <ChevronRight size={12} className="text-indigo-400 shrink-0" />
                    )}
                    {shortId(run.run_id)}
                  </span>
                </td>
                <td className="px-4 py-3 font-mono text-zinc-500 whitespace-nowrap text-xs">
                  {shortId(run.session_id)}
                </td>
                <td className="px-4 py-3 whitespace-nowrap">
                  <StateBadge state={run.state} compact />
                </td>
                <td className="px-4 py-3 text-zinc-500 whitespace-nowrap text-xs">
                  {fmtTime(run.created_at)}
                </td>
                <td className="px-4 py-3 text-zinc-500 whitespace-nowrap text-xs">
                  {fmtTime(run.updated_at)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// ── Loading skeleton ──────────────────────────────────────────────────────────

function SkeletonRows() {
  return (
    <div className="divide-y divide-zinc-800/60">
      {Array.from({ length: 8 }).map((_, i) => (
        <div key={i} className="flex items-center gap-4 px-4 py-3 animate-pulse">
          <div className="h-3 w-40 rounded bg-zinc-800" />
          <div className="h-3 w-28 rounded bg-zinc-800" />
          <div className="h-5 w-20 rounded-full bg-zinc-800" />
          <div className="h-3 w-32 rounded bg-zinc-800 ml-auto" />
          <div className="h-3 w-32 rounded bg-zinc-800" />
        </div>
      ))}
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function RunsPage() {
  const [stateFilter, setStateFilter] = useState<RunState | "all">("all");
  const [selectedRun, setSelectedRun] = useState<RunRecord | null>(null);

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["runs"],
    queryFn: () => defaultApi.getRuns({ limit: 200 }),
    refetchInterval: 15_000,
  });

  const runs = data ?? [];
  const filtered =
    stateFilter === "all" ? runs : runs.filter((r) => r.state === stateFilter);

  function handleSelect(run: RunRecord) {
    setSelectedRun((prev) => (prev?.run_id === run.run_id ? null : run));
  }

  // ── Error state ──────────────────────────────────────────────────────────
  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center p-8">
        <ServerCrash size={40} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load runs</p>
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
    <div className="flex flex-col h-full">
      {/* ── Toolbar ─────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-800 shrink-0 bg-zinc-950">
        <h2 className="text-sm font-semibold text-zinc-200 mr-2">
          Runs
          {!isLoading && (
            <span className="ml-2 text-xs text-zinc-500 font-normal">
              {filtered.length}
              {stateFilter !== "all" ? ` / ${runs.length} total` : ""}
            </span>
          )}
        </h2>

        {/* State filter */}
        <select
          value={stateFilter}
          onChange={(e) => setStateFilter(e.target.value as RunState | "all")}
          className="rounded-md bg-zinc-800 border border-zinc-700 text-zinc-300 text-xs px-2.5 py-1.5 focus:outline-none focus:ring-1 focus:ring-indigo-500"
        >
          <option value="all">All states</option>
          {ALL_STATES.map((s) => (
            <option key={s} value={s}>
              {STATE_FILTER_LABEL[s]}
            </option>
          ))}
        </select>

        {/* Refresh button */}
        <button
          onClick={() => void refetch()}
          disabled={isFetching}
          className="ml-auto flex items-center gap-1.5 rounded-md bg-zinc-800 border border-zinc-700 text-zinc-400 text-xs px-2.5 py-1.5 hover:text-zinc-200 hover:bg-zinc-700 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={13} className={clsx(isFetching && "animate-spin")} />
          Refresh
        </button>
      </div>

      {/* ── Content: table + optional detail panel ───────────────────────── */}
      <div className="flex flex-1 overflow-hidden">
        {/* Table */}
        <div className={clsx("flex-1 overflow-y-auto", selectedRun && "border-r border-zinc-800")}>
          {isLoading ? <SkeletonRows /> : (
            <RunsTable
              runs={filtered}
              selectedId={selectedRun?.run_id ?? null}
              onSelect={handleSelect}
            />
          )}
        </div>

        {/* Detail panel — slides in when a row is selected */}
        {selectedRun && (
          <DetailPanel run={selectedRun} onClose={() => setSelectedRun(null)} />
        )}
      </div>
    </div>
  );
}

export default RunsPage;
