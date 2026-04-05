import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  X, RefreshCw, ServerCrash, Inbox, ChevronRight,
  Pause, Play, Loader2, Clock, ListChecks, DollarSign, Activity,
} from "lucide-react";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { defaultApi } from "../lib/api";
import type { RunRecord, RunState, TaskState } from "../lib/types";

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

// ── Task state badge (reuses colour logic) ────────────────────────────────────

const TASK_STATE_STYLE: Record<string, string> = {
  queued:              'bg-zinc-800 text-zinc-400 ring-zinc-700',
  leased:              'bg-sky-950 text-sky-300 ring-sky-800',
  running:             'bg-blue-950 text-blue-300 ring-blue-800',
  completed:           'bg-emerald-950 text-emerald-400 ring-emerald-800',
  failed:              'bg-red-950 text-red-400 ring-red-800',
  canceled:            'bg-zinc-900 text-zinc-500 ring-zinc-700',
  paused:              'bg-amber-950 text-amber-300 ring-amber-800',
  waiting_dependency:  'bg-violet-950 text-violet-300 ring-violet-800',
  retryable_failed:    'bg-orange-950 text-orange-300 ring-orange-800',
  dead_lettered:       'bg-red-950 text-red-500 ring-red-800',
};

function TaskBadge({ state }: { state: TaskState }) {
  const style = TASK_STATE_STYLE[state] ?? TASK_STATE_STYLE.queued;
  return (
    <span className={clsx('inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium ring-1', style)}>
      {state.replace(/_/g, ' ')}
    </span>
  );
}

// ── Detail panel ─────────────────────────────────────────────────────────────

interface DetailPanelProps {
  run: RunRecord;
  onClose: () => void;
}

function DetailPanel({ run, onClose }: DetailPanelProps) {
  const qc = useQueryClient();

  // Sub-resource queries.
  const { data: events, isLoading: eventsLoading } = useQuery({
    queryKey: ['run-events', run.run_id],
    queryFn: () => defaultApi.getRunEvents(run.run_id, 50),
    refetchInterval: 10_000,
  });

  const { data: tasks, isLoading: tasksLoading } = useQuery({
    queryKey: ['run-tasks', run.run_id],
    queryFn: () => defaultApi.getRunTasks(run.run_id),
    retry: false, // 404 when run has no tasks yet
  });

  const { data: cost } = useQuery({
    queryKey: ['run-cost', run.run_id],
    queryFn: () => defaultApi.getRunCost(run.run_id),
    refetchInterval: 15_000,
  });

  // Pause / resume mutations.
  const pause = useMutation({
    mutationFn: () => defaultApi.pauseRun(run.run_id),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['runs'] }),
  });

  const resume = useMutation({
    mutationFn: () => defaultApi.resumeRun(run.run_id),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['runs'] }),
  });

  const canPause  = run.state === 'running' || run.state === 'pending';
  const canResume = run.state === 'paused';

  return (
    <aside className="flex flex-col w-[26rem] shrink-0 border-l border-zinc-800 bg-zinc-900 h-full overflow-y-auto">
      {/* ── Header ──────────────────────────────────────────────────────── */}
      <div className="flex items-center justify-between px-5 py-3.5 border-b border-zinc-800 sticky top-0 bg-zinc-900 z-10">
        <div className="flex items-center gap-2 min-w-0">
          <ChevronRight size={14} className="text-zinc-500 shrink-0" />
          <span className="text-sm font-semibold text-zinc-100 font-mono truncate">
            {shortId(run.run_id)}
          </span>
        </div>
        <button onClick={onClose} className="rounded-md p-1 text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors">
          <X size={16} />
        </button>
      </div>

      <div className="flex-1 p-5 space-y-5">

        {/* ── (1) State — large badge + pause/resume ─────────────────────── */}
        <div className="flex items-center justify-between">
          <div>
            <p className="text-[10px] text-zinc-500 uppercase tracking-widest mb-1.5">State</p>
            <StateBadge state={run.state} />
          </div>
          <div className="flex gap-2">
            {canPause && (
              <button
                onClick={() => pause.mutate()}
                disabled={pause.isPending}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-amber-900/50 ring-1 ring-amber-700/60 text-amber-300 text-xs font-medium hover:bg-amber-900 disabled:opacity-50 transition-colors"
              >
                {pause.isPending ? <Loader2 size={11} className="animate-spin" /> : <Pause size={11} />}
                Pause
              </button>
            )}
            {canResume && (
              <button
                onClick={() => resume.mutate()}
                disabled={resume.isPending}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-emerald-900/50 ring-1 ring-emerald-700/60 text-emerald-300 text-xs font-medium hover:bg-emerald-900 disabled:opacity-50 transition-colors"
              >
                {resume.isPending ? <Loader2 size={11} className="animate-spin" /> : <Play size={11} />}
                Resume
              </button>
            )}
          </div>
        </div>

        {/* ── (4) Cost breakdown ─────────────────────────────────────────── */}
        <Section title={<><DollarSign size={12} className="text-zinc-500" /> Cost</>}>
          {cost ? (
            <div className="grid grid-cols-2 gap-0 divide-y divide-x divide-zinc-700/40">
              <CostCell label="Total cost" value={`$${(cost.total_cost_micros / 1_000_000).toFixed(6)}`} />
              <CostCell label="Provider calls" value={String(cost.provider_calls)} />
              <CostCell label="Tokens in" value={cost.total_tokens_in.toLocaleString()} />
              <CostCell label="Tokens out" value={cost.total_tokens_out.toLocaleString()} />
            </div>
          ) : (
            <p className="px-3 py-2 text-xs text-zinc-600 italic">No cost data yet.</p>
          )}
        </Section>

        {/* ── (3) Tasks ──────────────────────────────────────────────────── */}
        <Section title={<><ListChecks size={12} className="text-zinc-500" /> Tasks ({tasks?.length ?? '…'})</>}>
          {tasksLoading ? (
            <SkeletonLines n={2} />
          ) : !tasks || tasks.length === 0 ? (
            <p className="px-3 py-2 text-xs text-zinc-600 italic">No tasks.</p>
          ) : (
            <div className="divide-y divide-zinc-700/40">
              {tasks.map((t) => (
                <div key={t.task_id} className="flex items-center justify-between px-3 py-2 gap-3">
                  <span className="text-[11px] font-mono text-zinc-400 truncate">{shortId(t.task_id)}</span>
                  <TaskBadge state={t.state} />
                </div>
              ))}
            </div>
          )}
        </Section>

        {/* ── (2) Event timeline ─────────────────────────────────────────── */}
        <Section title={<><Clock size={12} className="text-zinc-500" /> Timeline ({events?.length ?? '…'})</>}>
          {eventsLoading ? (
            <SkeletonLines n={4} />
          ) : !events || events.length === 0 ? (
            <p className="px-3 py-2 text-xs text-zinc-600 italic">No events recorded yet.</p>
          ) : (
            <div className="divide-y divide-zinc-700/30 max-h-72 overflow-y-auto">
              {events.map((ev) => (
                <div key={ev.position} className="flex items-center gap-3 px-3 py-1.5">
                  <span className="text-[10px] text-zinc-600 font-mono tabular-nums shrink-0 w-14 text-right">
                    #{ev.position}
                  </span>
                  <span className="text-[11px] font-mono text-indigo-400 truncate flex-1">
                    {ev.event_type}
                  </span>
                  <span className="text-[10px] text-zinc-600 shrink-0">
                    {new Date(ev.stored_at).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })}
                  </span>
                </div>
              ))}
            </div>
          )}
        </Section>

        {/* ── Identifiers / metadata ─────────────────────────────────────── */}
        <Section title={<><Activity size={12} className="text-zinc-500" /> Details</>}>
          <Field label="Run ID"     value={run.run_id} mono />
          <Field label="Session"    value={run.session_id} mono />
          {run.parent_run_id && <Field label="Parent" value={run.parent_run_id} mono />}
          <Field label="Created"    value={fmtTime(run.created_at)} />
          <Field label="Updated"    value={fmtTime(run.updated_at)} />
          {run.failure_class && <Field label="Failure" value={run.failure_class} />}
        </Section>

      </div>
    </aside>
  );
}

function CostCell({ label, value }: { label: string; value: string }) {
  return (
    <div className="px-3 py-2">
      <p className="text-[10px] text-zinc-500">{label}</p>
      <p className="text-xs text-zinc-200 font-mono mt-0.5">{value}</p>
    </div>
  );
}

function SkeletonLines({ n }: { n: number }) {
  return (
    <div className="divide-y divide-zinc-700/40 animate-pulse">
      {Array.from({ length: n }).map((_, i) => (
        <div key={i} className="flex items-center gap-3 px-3 py-2">
          <div className="h-2.5 w-24 rounded bg-zinc-800" />
          <div className="h-2.5 w-16 rounded bg-zinc-800 ml-auto" />
        </div>
      ))}
    </div>
  );
}

function Section({ title, children }: { title: React.ReactNode; children: React.ReactNode }) {
  return (
    <div>
      <p className="flex items-center gap-1.5 text-[10px] text-zinc-500 uppercase tracking-widest mb-2">{title}</p>
      <div className="rounded-lg bg-zinc-800/50 ring-1 ring-zinc-700/50 overflow-hidden">
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
