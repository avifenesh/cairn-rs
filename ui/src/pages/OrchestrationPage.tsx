/**
 * OrchestrationPage — live agent orchestration hierarchy.
 *
 * Displays the full session → run → task → worker tree with real-time
 * updates: polling every 10 s + instant SSE-triggered refetch on new events.
 *
 * Tree is an indented collapsible list — no graph library required.
 */

import { useState, useEffect, useCallback, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronRight, ChevronDown, RefreshCw, Loader2,
  Radio, Layers, Play, ListChecks, Cpu,
  CheckCircle2, Clock,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { useEventStream } from "../hooks/useEventStream";
import type { SessionRecord, RunRecord, TaskRecord } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

function fmtAge(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000)      return `${Math.floor(d / 1_000)}s ago`;
  if (d < 3_600_000)   return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)  return `${Math.floor(d / 3_600_000)}h ago`;
  return `${Math.floor(d / 86_400_000)}d ago`;
}

function fmtDur(startMs: number, endMs?: number): string {
  const ms = (endMs ?? Date.now()) - startMs;
  if (ms < 1_000)  return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(1)}s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1_000)}s`;
  return `${Math.floor(ms / 3_600_000)}h ${Math.floor((ms % 3_600_000) / 60_000)}m`;
}

// ── Status pill ───────────────────────────────────────────────────────────────

const STATE_PILL: Record<string, { dot: string; text: string; label: string }> = {
  // Sessions
  open:       { dot: "bg-emerald-500",              text: "text-emerald-400", label: "open" },
  completed:  { dot: "bg-zinc-500",                 text: "text-zinc-500",   label: "done" },
  failed:     { dot: "bg-red-500 animate-pulse",    text: "text-red-400",    label: "failed" },
  archived:   { dot: "bg-zinc-700",                 text: "text-zinc-600",   label: "archived" },
  // Runs
  running:    { dot: "bg-blue-400 animate-pulse",   text: "text-blue-400",   label: "running" },
  pending:    { dot: "bg-zinc-500",                 text: "text-zinc-500",   label: "pending" },
  paused:     { dot: "bg-amber-400",                text: "text-amber-400",  label: "paused" },
  waiting_approval: { dot: "bg-purple-400", text: "text-purple-400", label: "awaiting approval" },
  waiting_dependency: { dot: "bg-sky-400",  text: "text-sky-400",   label: "waiting" },
  canceled:   { dot: "bg-zinc-700",                 text: "text-zinc-600",   label: "canceled" },
  // Tasks
  queued:     { dot: "bg-amber-400",                text: "text-amber-400",  label: "queued" },
  leased:     { dot: "bg-indigo-400",               text: "text-indigo-400", label: "claimed" },
  dead_lettered: { dot: "bg-red-700",               text: "text-red-600",    label: "dead" },
  retryable_failed: { dot: "bg-orange-500",         text: "text-orange-400", label: "retryable" },
};

function StatePill({ state }: { state: string }) {
  const cfg = STATE_PILL[state] ?? { dot: "bg-zinc-600", text: "text-zinc-500", label: state };
  return (
    <span className={clsx("inline-flex items-center gap-1 text-[10px] font-medium", cfg.text)}>
      <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0", cfg.dot)} />
      {cfg.label}
    </span>
  );
}

// ── Tree data model ───────────────────────────────────────────────────────────

interface RunWithTasks {
  run:   RunRecord;
  tasks: TaskRecord[];
}

interface SessionNode {
  session:      SessionRecord;
  runs:         RunWithTasks[];
  hasActive:    boolean;   // any run is running/pending/paused
}

function buildTree(
  sessions: SessionRecord[],
  runs:     RunRecord[],
  tasks:    TaskRecord[],
): SessionNode[] {
  const runsBySession = new Map<string, RunRecord[]>();
  const tasksByRun    = new Map<string, TaskRecord[]>();

  for (const r of runs) {
    const list = runsBySession.get(r.session_id) ?? [];
    list.push(r);
    runsBySession.set(r.session_id, list);
  }
  for (const t of tasks) {
    const rid  = t.parent_run_id ?? "__none__";
    const list = tasksByRun.get(rid) ?? [];
    list.push(t);
    tasksByRun.set(rid, list);
  }

  const ACTIVE_RUNS = new Set(["running", "pending", "paused", "waiting_approval", "waiting_dependency"]);

  return sessions
    .map(session => {
      const sessionRuns = (runsBySession.get(session.session_id) ?? [])
        .sort((a, b) => b.created_at - a.created_at)
        .map(run => ({
          run,
          tasks: (tasksByRun.get(run.run_id) ?? [])
            .sort((a, b) => a.created_at - b.created_at),
        }));
      return {
        session,
        runs:      sessionRuns,
        hasActive: sessionRuns.some(r => ACTIVE_RUNS.has(r.run.state)),
      };
    })
    .sort((a, b) => {
      // Active sessions first, then newest
      if (a.hasActive !== b.hasActive) return a.hasActive ? -1 : 1;
      return b.session.created_at - a.session.created_at;
    });
}

// ── Task row ──────────────────────────────────────────────────────────────────

function TaskRow({ task, fresh }: { task: TaskRecord; fresh: boolean }) {
  const isActive = ["leased", "running"].includes(task.state);
  const endMs    = ["completed","failed","canceled"].includes(task.state) ? task.updated_at : undefined;

  return (
    <div className={clsx(
      "flex items-center gap-2 py-1 px-2 rounded-md transition-colors",
      fresh && "bg-indigo-950/30 ring-1 ring-indigo-800/30",
    )}>
      {/* Tree connector */}
      <div className="flex items-center gap-0 shrink-0 ml-14">
        <span className="w-px h-4 bg-zinc-800 shrink-0" />
        <span className="w-3 h-px bg-zinc-800 shrink-0" />
        <ListChecks size={10} className={isActive ? "text-indigo-400" : "text-zinc-600"} />
      </div>

      {/* Task info */}
      <div className="flex items-center gap-2 flex-1 min-w-0">
        <span className="font-mono text-[11px] text-zinc-400 truncate" title={task.task_id}>
          {shortId(task.task_id)}
        </span>
        <StatePill state={task.state} />
        {task.lease_owner && (
          <span className="flex items-center gap-1 text-[10px] font-mono text-zinc-600 truncate">
            <Cpu size={9} className="shrink-0" />
            {shortId(task.lease_owner)}
          </span>
        )}
      </div>

      {/* Duration + age */}
      <div className="flex items-center gap-3 shrink-0 text-[10px] text-zinc-700 tabular-nums">
        <span>{fmtDur(task.created_at, endMs)}</span>
        <span>{fmtAge(task.created_at)}</span>
      </div>
    </div>
  );
}

// ── Run row ───────────────────────────────────────────────────────────────────

function RunRow({
  item, expanded, onToggle, fresh, freshTaskIds,
}: {
  item:         RunWithTasks;
  expanded:     boolean;
  onToggle:     () => void;
  fresh:        boolean;
  freshTaskIds: Set<string>;
}) {
  const { run, tasks } = item;
  const isActive  = ["running","pending","paused","waiting_approval","waiting_dependency"].includes(run.state);
  const isTerminal = ["completed","failed","canceled"].includes(run.state);
  const endMs      = isTerminal ? run.updated_at : undefined;

  const taskSummary = tasks.length > 0
    ? `${tasks.filter(t => t.state === "completed").length}/${tasks.length} tasks`
    : "0 tasks";

  return (
    <div>
      {/* Run header */}
      <div
        className={clsx(
          "flex items-center gap-2 py-1.5 px-2 rounded-md cursor-pointer select-none",
          "hover:bg-white/[0.03] transition-colors",
          fresh && "bg-blue-950/20 ring-1 ring-blue-800/20",
        )}
        onClick={onToggle}
      >
        {/* Tree connector + indent */}
        <div className="flex items-center gap-0 shrink-0 ml-7">
          <span className="w-px h-4 bg-zinc-800 shrink-0" />
          <span className="w-3 h-px bg-zinc-800 shrink-0" />
          {expanded
            ? <ChevronDown  size={10} className="text-zinc-500 shrink-0" />
            : <ChevronRight size={10} className="text-zinc-500 shrink-0" />
          }
        </div>
        <Play size={11} className={isActive ? "text-blue-400 shrink-0" : "text-zinc-600 shrink-0"} />

        {/* Run ID */}
        <span className="font-mono text-[12px] text-zinc-300 truncate flex-1 min-w-0" title={run.run_id}>
          {shortId(run.run_id)}
        </span>

        {/* State + task count */}
        <div className="flex items-center gap-2 shrink-0">
          <StatePill state={run.state} />
          <span className="text-[10px] text-zinc-700">{taskSummary}</span>
        </div>

        {/* Timing */}
        <div className="flex items-center gap-3 shrink-0 text-[10px] text-zinc-700 tabular-nums">
          <span>{fmtDur(run.created_at, endMs)}</span>
          <span>{fmtAge(run.created_at)}</span>
        </div>
      </div>

      {/* Task list */}
      {expanded && (
        <div className="space-y-0.5 mt-0.5">
          {tasks.length === 0 ? (
            <div className="ml-20 text-[10px] text-zinc-700 italic py-0.5">No tasks yet</div>
          ) : (
            tasks.map(t => (
              <TaskRow key={t.task_id} task={t} fresh={freshTaskIds.has(t.task_id)} />
            ))
          )}
        </div>
      )}
    </div>
  );
}

// ── Session row ───────────────────────────────────────────────────────────────

function SessionRow({
  node, expandedRuns, onToggleSession, onToggleRun,
  expandedSession, fresh, freshRunIds, freshTaskIds,
}: {
  node:            SessionNode;
  expandedSession: boolean;
  expandedRuns:    Set<string>;
  onToggleSession: () => void;
  onToggleRun:     (id: string) => void;
  fresh:           boolean;
  freshRunIds:     Set<string>;
  freshTaskIds:    Set<string>;
}) {
  const { session, runs } = node;
  const activeRuns = runs.filter(r => ["running","pending","paused","waiting_approval","waiting_dependency"].includes(r.run.state)).length;

  return (
    <div className={clsx(
      "rounded-lg border overflow-hidden transition-colors",
      node.hasActive ? "border-blue-900/50 bg-zinc-900/80" : "border-zinc-800 bg-zinc-900/40",
      fresh && "ring-1 ring-emerald-700/40",
    )}>
      {/* Session header */}
      <div
        className="flex items-center gap-2.5 px-3 py-2 cursor-pointer select-none hover:bg-white/[0.03] transition-colors"
        onClick={onToggleSession}
      >
        <div className={clsx(
          "flex h-6 w-6 items-center justify-center rounded-md shrink-0",
          node.hasActive ? "bg-blue-950/60 border border-blue-800/50" : "bg-zinc-800 border border-zinc-700",
        )}>
          <Layers size={11} className={node.hasActive ? "text-blue-400" : "text-zinc-500"} />
        </div>

        {expandedSession
          ? <ChevronDown  size={12} className="text-zinc-500 shrink-0" />
          : <ChevronRight size={12} className="text-zinc-500 shrink-0" />
        }

        {/* Session ID */}
        <span className="font-mono text-[12px] text-zinc-200 truncate flex-1 min-w-0" title={session.session_id}>
          {shortId(session.session_id)}
        </span>

        {/* Project scope */}
        <span className="text-[10px] font-mono text-zinc-600 hidden sm:block shrink-0">
          {session.project.tenant_id}/{session.project.project_id}
        </span>

        {/* State + counts */}
        <div className="flex items-center gap-2 shrink-0">
          <StatePill state={session.state} />
          <span className="text-[10px] text-zinc-600">
            {runs.length} run{runs.length !== 1 ? "s" : ""}
            {activeRuns > 0 && (
              <span className="ml-1 text-blue-500">{activeRuns} active</span>
            )}
          </span>
        </div>

        {/* Age */}
        <span className="text-[10px] text-zinc-700 tabular-nums shrink-0">
          {fmtAge(session.created_at)}
        </span>
      </div>

      {/* Run list */}
      {expandedSession && (
        <div className="border-t border-zinc-800/60 px-3 py-2 space-y-0.5 bg-zinc-950/30">
          {runs.length === 0 ? (
            <p className="text-[11px] text-zinc-700 italic pl-7 py-1">No runs in this session</p>
          ) : (
            runs.map(item => (
              <RunRow
                key={item.run.run_id}
                item={item}
                expanded={expandedRuns.has(item.run.run_id)}
                onToggle={() => onToggleRun(item.run.run_id)}
                fresh={freshRunIds.has(item.run.run_id)}
                freshTaskIds={freshTaskIds}
              />
            ))
          )}
        </div>
      )}
    </div>
  );
}

// ── Stats strip ───────────────────────────────────────────────────────────────

function StatsStrip({ nodes }: { nodes: SessionNode[] }) {
  const totalSessions  = nodes.length;
  const activeSessions = nodes.filter(n => n.hasActive).length;
  const totalRuns      = nodes.reduce((s, n) => s + n.runs.length, 0);
  const activeRuns     = nodes.reduce((s, n) =>
    s + n.runs.filter(r => ["running","pending","paused"].includes(r.run.state)).length, 0);
  const totalTasks     = nodes.reduce((s, n) =>
    s + n.runs.reduce((t, r) => t + r.tasks.length, 0), 0);
  const activeTasks    = nodes.reduce((s, n) =>
    s + n.runs.reduce((t, r) =>
      t + r.tasks.filter(tk => ["leased","running","queued"].includes(tk.state)).length, 0), 0);

  const items = [
    { label: "Sessions", total: totalSessions, active: activeSessions, icon: Layers },
    { label: "Runs",     total: totalRuns,     active: activeRuns,     icon: Play },
    { label: "Tasks",    total: totalTasks,    active: activeTasks,    icon: ListChecks },
  ];

  return (
    <div className="grid grid-cols-3 gap-3">
      {items.map(({ label, total, active, icon: Icon }) => (
        <div key={label} className="bg-zinc-900 border border-zinc-800 rounded-lg p-3 flex items-center gap-3">
          <Icon size={16} className={active > 0 ? "text-blue-400" : "text-zinc-600"} />
          <div>
            <p className="text-[11px] text-zinc-500 uppercase tracking-wider">{label}</p>
            <p className="text-[18px] font-semibold tabular-nums text-zinc-100 leading-tight">{total}</p>
            {active > 0 && (
              <p className="text-[10px] text-blue-400">{active} active</p>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function OrchestrationPage() {
  useQueryClient(); // keep for future query invalidation

  // Expand state — auto-expand active sessions/runs
  const [expandedSessions, setExpandedSessions] = useState<Set<string>>(new Set());
  const [expandedRuns,     setExpandedRuns]      = useState<Set<string>>(new Set());
  // Brief "fresh" highlight for SSE-triggered new nodes
  const [freshIds, setFreshIds] = useState<Set<string>>(new Set());

  // ── Queries ─────────────────────────────────────────────────────────────────

  const { data: sessions, isLoading: sLoading, refetch: rSessions, isFetching: sFetching } = useQuery({
    queryKey: ["orch-sessions"],
    queryFn:  () => defaultApi.getSessions({ limit: 100 }),
    refetchInterval: 10_000,
  });

  const { data: runs, isLoading: rLoading, refetch: rRuns } = useQuery({
    queryKey: ["orch-runs"],
    queryFn:  () => defaultApi.getRuns({ limit: 500 }),
    refetchInterval: 10_000,
  });

  const { data: tasks, isLoading: tLoading, refetch: rTasks } = useQuery({
    queryKey: ["orch-tasks"],
    queryFn:  () => defaultApi.getAllTasks({ limit: 1000 }),
    refetchInterval: 10_000,
  });

  // ── SSE integration ─────────────────────────────────────────────────────────

  const { events: streamEvents, status: sseStatus } = useEventStream();

  // On new SSE events: refetch relevant data and mark new node IDs as fresh
  useEffect(() => {
    if (streamEvents.length === 0) return;
    const latest = streamEvents[streamEvents.length - 1];
    const type   = latest.type;
    const payload = latest.payload as Record<string, unknown> | null;

    // Derive the new entity ID from the payload
    let newId: string | null = null;
    if (type === "session_created")   { newId = (payload as Record<string, unknown> | null)?.session_id as string; }
    if (type === "run_created")        { newId = (payload as Record<string, unknown> | null)?.run_id     as string; }
    if (type === "task_created")       { newId = (payload as Record<string, unknown> | null)?.task_id    as string; }

    // Trigger refetch
    if (type.includes("session"))      { void rSessions(); }
    if (type.includes("run"))          { void rRuns(); }
    if (type.includes("task"))         { void rTasks(); }

    // Mark as fresh for 3 s
    if (newId) {
      setFreshIds(prev => new Set([...prev, newId as string]));
      setTimeout(() => {
        setFreshIds(prev => { const next = new Set(prev); next.delete(newId as string); return next; });
      }, 3_000);

      // Auto-expand the parent
      if (type === "run_created") {
        const sessionId = (payload as Record<string, unknown> | null)?.session_id as string | undefined;
        if (sessionId) setExpandedSessions(p => new Set([...p, sessionId]));
      }
      if (type === "task_created") {
        const runId = (payload as Record<string, unknown> | null)?.run_id as string | undefined;
        if (runId) setExpandedRuns(p => new Set([...p, runId]));
      }
    }
  }, [streamEvents, rSessions, rRuns, rTasks]);

  // Auto-expand active sessions and their running runs on first load
  useEffect(() => {
    if (!sessions || !runs) return;
    const ACTIVE_RUNS = new Set(["running","pending","paused","waiting_approval","waiting_dependency"]);
    const activeRunIds = new Set(runs.filter(r => ACTIVE_RUNS.has(r.state)).map(r => r.run_id));
    const activeSessionIds = new Set(
      runs.filter(r => ACTIVE_RUNS.has(r.state)).map(r => r.session_id),
    );
    setExpandedSessions(prev => new Set([...prev, ...activeSessionIds]));
    setExpandedRuns(prev     => new Set([...prev, ...activeRunIds]));
  }, [sessions?.length, runs?.length]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Tree ────────────────────────────────────────────────────────────────────

  const tree = useMemo(
    () => buildTree(sessions ?? [], runs ?? [], tasks ?? []),
    [sessions, runs, tasks],
  );

  const freshSessionIds = useMemo(() =>
    new Set([...freshIds].filter(id => sessions?.some(s => s.session_id === id))),
    [freshIds, sessions],
  );
  const freshRunIds = useMemo(() =>
    new Set([...freshIds].filter(id => runs?.some(r => r.run_id === id))),
    [freshIds, runs],
  );
  const freshTaskIds = useMemo(() =>
    new Set([...freshIds].filter(id => tasks?.some(t => t.task_id === id))),
    [freshIds, tasks],
  );

  // ── Handlers ────────────────────────────────────────────────────────────────

  const toggleSession = useCallback((id: string) => {
    setExpandedSessions(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const toggleRun = useCallback((id: string) => {
    setExpandedRuns(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  function expandAll() {
    setExpandedSessions(new Set(tree.map(n => n.session.session_id)));
    setExpandedRuns(new Set(tree.flatMap(n => n.runs.map(r => r.run.run_id))));
  }
  function collapseAll() {
    setExpandedSessions(new Set());
    setExpandedRuns(new Set());
  }

  const isLoading = sLoading || rLoading || tLoading;
  const isFetching = sFetching;

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-zinc-800 shrink-0">
        <Layers size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[13px] font-medium text-zinc-200">
          Orchestration
          {!isLoading && (
            <span className="ml-2 text-[11px] text-zinc-600 font-normal">
              {tree.length} sessions
            </span>
          )}
        </span>

        {/* SSE status */}
        <span className={clsx(
          "flex items-center gap-1.5 text-[11px] font-medium ml-1",
          sseStatus === "connected"    ? "text-emerald-500" :
          sseStatus === "connecting"   ? "text-amber-400"   : "text-zinc-600",
        )}>
          <Radio size={10} className={sseStatus === "connected" ? "text-emerald-500" : ""} />
          {sseStatus === "connected" ? "Live" : sseStatus === "connecting" ? "Connecting…" : "Offline"}
        </span>

        <div className="ml-auto flex items-center gap-2">
          <button
            onClick={expandAll}
            className="text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors"
          >
            Expand all
          </button>
          <span className="text-zinc-800 text-[11px]">·</span>
          <button
            onClick={collapseAll}
            className="text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors"
          >
            Collapse all
          </button>
          <button
            onClick={() => { void rSessions(); void rRuns(); void rTasks(); }}
            disabled={isFetching}
            className="flex items-center gap-1 text-[12px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40 transition-colors ml-1"
          >
            <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
            Refresh
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Building orchestration tree…</span>
          </div>
        ) : (
          <>
            {/* Stats */}
            <StatsStrip nodes={tree} />

            {/* Tree */}
            {tree.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
                <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-zinc-800 border border-zinc-700">
                  <Layers size={24} className="text-zinc-500" />
                </div>
                <p className="text-[13px] font-medium text-zinc-400">No sessions yet</p>
                <p className="text-[12px] text-zinc-600 max-w-xs">
                  Sessions appear here once created via{" "}
                  <code className="bg-zinc-800 rounded px-1 text-zinc-500">POST /v1/sessions</code>.
                </p>
              </div>
            ) : (
              <div className="space-y-2">
                {tree.map(node => (
                  <SessionRow
                    key={node.session.session_id}
                    node={node}
                    expandedSession={expandedSessions.has(node.session.session_id)}
                    expandedRuns={expandedRuns}
                    onToggleSession={() => toggleSession(node.session.session_id)}
                    onToggleRun={toggleRun}
                    fresh={freshSessionIds.has(node.session.session_id)}
                    freshRunIds={freshRunIds}
                    freshTaskIds={freshTaskIds}
                  />
                ))}
              </div>
            )}

            {/* Legend */}
            {tree.length > 0 && (
              <div className="flex items-center gap-4 pt-1 flex-wrap">
                <span className="text-[10px] text-zinc-700 uppercase tracking-wider">States:</span>
                {[
                  { state: "running",  label: "running"  },
                  { state: "pending",  label: "pending"  },
                  { state: "completed",label: "done"     },
                  { state: "failed",   label: "failed"   },
                  { state: "paused",   label: "paused"   },
                  { state: "queued",   label: "queued"   },
                  { state: "leased",   label: "claimed"  },
                ].map(({ state, label }) => {
                  const cfg = STATE_PILL[state];
                  return (
                    <span key={state} className={clsx("flex items-center gap-1 text-[10px]", cfg?.text ?? "text-zinc-600")}>
                      <span className={clsx("w-1.5 h-1.5 rounded-full", cfg?.dot ?? "bg-zinc-600")} />
                      {label}
                    </span>
                  );
                })}
              </div>
            )}

            {/* SSE event count */}
            {streamEvents.length > 0 && (
              <div className="flex items-center gap-2 text-[10px] text-zinc-700">
                <CheckCircle2 size={10} className="text-emerald-700" />
                {streamEvents.length} live event{streamEvents.length !== 1 ? "s" : ""} received this session
              </div>
            )}
          </>
        )}
      </div>

      {/* Footer: update cadence */}
      <div className="flex items-center gap-2 px-4 py-2 border-t border-zinc-800 shrink-0 text-[10px] text-zinc-700">
        <Clock size={10} />
        Polls every 10 s · SSE triggers immediate refetch on session, run, and task events
        {freshIds.size > 0 && (
          <span className="ml-2 text-indigo-500 font-medium">
            {freshIds.size} new node{freshIds.size !== 1 ? "s" : ""} detected
          </span>
        )}
      </div>
    </div>
  );
}

export default OrchestrationPage;
