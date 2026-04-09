import { useQuery } from "@tanstack/react-query";
import { useState, useEffect, useRef } from "react";
import {
  ArrowLeft, Loader2, Clock, Hash, Cpu, Download,
  Brain, Search, Zap, CheckCircle2, Wrench, ChevronDown, ChevronRight,
  Play, AlertTriangle,
} from "lucide-react";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { GanttView } from "../components/TimelineView";
import { CopyButton } from "../components/CopyButton";
import { defaultApi } from "../lib/api";
import { useEventStream } from "../hooks/useEventStream";

// ── Helpers ────────────────────────────────────────────────────────────────────

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-7)}` : id;

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });

const fmtTimeShort = (ms: number) =>
  new Date(ms).toLocaleTimeString(undefined, {
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });

const fmtDuration = (startMs: number, endMs?: number) => {
  const ms = (endMs ?? Date.now()) - startMs;
  if (ms < 1_000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1_000)}s`;
};

const fmtMicros = (micros: number) => {
  if (micros === 0) return "—";
  return `$${(micros / 1_000_000).toFixed(6)}`;
};

const fmtTokens = (n: number) =>
  n >= 1_000 ? `${(n / 1_000).toFixed(1)}k` : String(n);

// ── Stat card ──────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub }: { label: string; value: string | number; sub?: string }) {
  return (
    <div className="border-l-2 border-indigo-500 pl-3 py-0.5">
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

// ── Orchestration timeline ────────────────────────────────────────────────────

interface OrchestrationEntry {
  id:        string;
  type:      string;
  ts:        number;
  payload:   Record<string, unknown>;
  expanded:  boolean;
}

const ORCH_TYPES = new Set([
  "orchestrate_started", "gather_completed", "decide_completed",
  "tool_called", "tool_result", "step_completed", "orchestrate_finished",
  "operator_notification",
]);

function orchIcon(type: string) {
  if (type === "orchestrate_started")  return <Play         size={12} className="text-emerald-400" />;
  if (type === "orchestrate_finished") return <CheckCircle2 size={12} className="text-emerald-400" />;
  if (type === "gather_completed")     return <Search        size={12} className="text-sky-400"     />;
  if (type === "decide_completed")     return <Brain         size={12} className="text-indigo-400"  />;
  if (type === "tool_called")          return <Wrench        size={12} className="text-amber-400"   />;
  if (type === "tool_result")          return <Zap           size={12} className="text-teal-400"    />;
  if (type === "step_completed")       return <ChevronRight  size={12} className="text-gray-500 dark:text-zinc-400"    />;
  if (type === "operator_notification")return <AlertTriangle size={12} className="text-orange-400"  />;
  return                                      <Hash          size={12} className="text-gray-400 dark:text-zinc-600"    />;
}

function orchColor(type: string): string {
  if (type === "orchestrate_started" || type === "orchestrate_finished")
    return "border-emerald-700/50 bg-emerald-950/20";
  if (type === "gather_completed")     return "border-sky-700/50 bg-sky-950/20";
  if (type === "decide_completed")     return "border-indigo-700/50 bg-indigo-950/20";
  if (type.startsWith("tool"))         return "border-amber-700/50 bg-amber-950/20";
  if (type === "operator_notification")return "border-orange-700/50 bg-orange-950/20";
  return "border-gray-200 dark:border-zinc-700/50 bg-gray-100/30 dark:bg-zinc-800/30";
}

function orchSummary(type: string, p: Record<string, unknown>): string {
  switch (type) {
    case "orchestrate_started":
      return typeof p.goal === "string" ? `Goal: "${p.goal.slice(0, 80)}${p.goal.length > 80 ? "…" : ""}"` : "Started";
    case "gather_completed": {
      const chunks = typeof p.memory_chunks === "number" ? p.memory_chunks : 0;
      const evts   = typeof p.recent_events === "number" ? p.recent_events : 0;
      return `${chunks} memory chunk${chunks !== 1 ? "s" : ""}, ${evts} recent event${evts !== 1 ? "s" : ""}`;
    }
    case "decide_completed": {
      const count = typeof p.proposals === "number" ? p.proposals : 0;
      const first = typeof p.first_action === "string" ? p.first_action : "";
      const conf  = typeof p.confidence  === "number" ? ` (${(p.confidence * 100).toFixed(0)}%)` : "";
      return `${count} proposal${count !== 1 ? "s" : ""}${first ? ` · ${first}` : ""}${conf}`;
    }
    case "tool_called": {
      const name = typeof p.tool_name === "string" ? p.tool_name : "unknown";
      return `→ ${name}`;
    }
    case "tool_result": {
      const name    = typeof p.tool_name === "string" ? p.tool_name : "unknown";
      const success = p.success !== false;
      return `${name} ${success ? "✓" : "✗"}`;
    }
    case "step_completed": {
      const iter = typeof p.iteration === "number" ? p.iteration + 1 : "?";
      const kind = typeof p.action_kind === "string" ? p.action_kind : "";
      return `Iteration ${iter}${kind ? ` · ${kind}` : ""}`;
    }
    case "orchestrate_finished": {
      const term = typeof p.termination === "string" ? p.termination : "unknown";
      const summary = typeof p.summary === "string" ? ` — ${p.summary.slice(0, 60)}` : "";
      return `${term}${summary}`;
    }
    case "operator_notification": {
      const sev = typeof p.severity === "string" ? `[${p.severity}] ` : "";
      const msg = typeof p.message  === "string" ? p.message.slice(0, 80) : "";
      return `${sev}${msg}`;
    }
    default: return "";
  }
}

function OrchestrationTimeline({ runId }: { runId: string }) {
  const { events: streamEvents } = useEventStream();
  const [entries, setEntries]     = useState<OrchestrationEntry[]>([]);
  const [finished, setFinished]   = useState(false);
  const seenRef = useRef(new Set<string>());

  // Filter and accumulate orchestration events for this run
  useEffect(() => {
    const newEntries: OrchestrationEntry[] = [];

    for (const ev of streamEvents) {
      if (!ORCH_TYPES.has(ev.type)) continue;

      const p = (ev.payload ?? {}) as Record<string, unknown>;
      const evRunId = (p.run_id ?? p.runId) as string | undefined;
      if (evRunId && evRunId !== runId) continue;

      const uid = `${ev.type}-${ev.id}`;
      if (seenRef.current.has(uid)) continue;
      seenRef.current.add(uid);

      newEntries.push({
        id:       uid,
        type:     ev.type,
        ts:       ev.receivedAt,
        payload:  p,
        expanded: false,
      });

      if (ev.type === "orchestrate_finished") {
        setFinished(true);
      }
    }

    if (newEntries.length > 0) {
      // streamEvents are newest-first; insert new entries at the end (chronological)
      setEntries(prev => {
        const merged = [...prev, ...newEntries];
        merged.sort((a, b) => a.ts - b.ts);
        return merged;
      });
    }
  }, [streamEvents, runId]);

  if (entries.length === 0) return null;

  const toggleExpand = (id: string) =>
    setEntries(prev => prev.map(e => e.id === id ? { ...e, expanded: !e.expanded } : e));

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-3">
        <p className="text-[11px] font-semibold text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
          Orchestration Timeline
        </p>
        <div className="flex items-center gap-2">
          {finished ? (
            <span className="flex items-center gap-1 text-[10px] text-emerald-400 font-medium">
              <CheckCircle2 size={11} /> Complete
            </span>
          ) : (
            <span className="flex items-center gap-1.5 text-[10px] text-indigo-400 font-medium">
              <span className="w-1.5 h-1.5 rounded-full bg-indigo-400 animate-pulse" />
              Live
            </span>
          )}
          <span className="text-[10px] text-gray-400 dark:text-zinc-600">{entries.length} event{entries.length !== 1 ? "s" : ""}</span>
        </div>
      </div>

      {/* Timeline */}
      <div className="relative">
        {/* Vertical track */}
        <div className="absolute left-[18px] top-3 bottom-3 w-px bg-gray-100 dark:bg-zinc-800" />

        <div className="space-y-1">
          {entries.map((entry) => {
            const summary = orchSummary(entry.type, entry.payload);
            const hasDetail = Object.keys(entry.payload).length > 0;

            return (
              <div key={entry.id} className="relative flex items-start gap-3">
                {/* Icon dot */}
                <div className="w-9 h-9 shrink-0 flex items-center justify-center relative z-10">
                  <div className={clsx(
                    "w-6 h-6 rounded-full border flex items-center justify-center",
                    orchColor(entry.type),
                  )}>
                    {orchIcon(entry.type)}
                  </div>
                </div>

                {/* Card */}
                <div className={clsx(
                  "flex-1 rounded-lg border px-3 py-2 min-w-0 transition-colors",
                  orchColor(entry.type),
                  hasDetail && "cursor-pointer hover:brightness-110",
                )}
                  onClick={() => hasDetail && toggleExpand(entry.id)}
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="text-[11px] font-mono text-gray-700 dark:text-zinc-300 shrink-0">
                        {entry.type.replace(/_/g, "\u00A0")}
                      </span>
                      {summary && (
                        <span className="text-[11px] text-gray-400 dark:text-zinc-500 truncate">{summary}</span>
                      )}
                    </div>
                    <div className="flex items-center gap-1.5 shrink-0">
                      <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono tabular-nums">
                        {new Date(entry.ts).toLocaleTimeString(undefined, {
                          hour: "2-digit", minute: "2-digit", second: "2-digit",
                        })}
                      </span>
                      {hasDetail && (
                        entry.expanded
                          ? <ChevronDown size={11} className="text-gray-400 dark:text-zinc-600" />
                          : <ChevronRight size={11} className="text-gray-400 dark:text-zinc-600" />
                      )}
                    </div>
                  </div>

                  {/* Expanded payload */}
                  {entry.expanded && (
                    <pre className="mt-2 text-[10px] text-gray-500 dark:text-zinc-400 font-mono bg-white dark:bg-zinc-950/60 rounded p-2 overflow-x-auto max-h-48 whitespace-pre-wrap break-all">
                      {JSON.stringify(entry.payload, null, 2)}
                    </pre>
                  )}
                </div>
              </div>
            );
          })}

          {/* Live indicator at the bottom when still running */}
          {!finished && (
            <div className="relative flex items-center gap-3">
              <div className="w-9 shrink-0 flex justify-center">
                <div className="w-2 h-2 rounded-full bg-indigo-500 animate-pulse relative z-10" />
              </div>
              <span className="text-[11px] text-gray-400 dark:text-zinc-600 italic">Waiting for next event…</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Section wrapper ────────────────────────────────────────────────────────────

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[11px] font-semibold text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">
        {title}
      </p>
      {children}
    </div>
  );
}

const TH = ({ ch, right, hide }: { ch: React.ReactNode; right?: boolean; hide?: string }) => (
  <th className={clsx(
    "px-3 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-gray-200 dark:border-zinc-800",
    right ? "text-right" : "text-left",
    hide,
  )}>{ch}</th>
);

// ── Event type badge ──────────────────────────────────────────────────────────

function eventTypeColor(type: string): string {
  if (type.includes("run"))        return "bg-blue-950  text-blue-300  ring-blue-800";
  if (type.includes("task"))       return "bg-indigo-950 text-indigo-300 ring-indigo-800";
  if (type.includes("approval"))   return "bg-violet-950 text-violet-300 ring-violet-800";
  if (type.includes("checkpoint")) return "bg-amber-950  text-amber-300 ring-amber-800";
  if (type.includes("tool") || type.includes("provider"))
                                   return "bg-teal-950   text-teal-300  ring-teal-800";
  return "bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 ring-gray-300 dark:ring-zinc-700";
}

// ── Page ──────────────────────────────────────────────────────────────────────

interface RunDetailPageProps {
  runId: string;
  onBack?: () => void;
}

export function RunDetailPage({ runId, onBack }: RunDetailPageProps) {
  // Fetch run metadata from the runs list (no dedicated GET /v1/runs/:id in the main.rs client).
  const { data: runs } = useQuery({
    queryKey: ["runs"],
    queryFn: () => defaultApi.getRuns({ limit: 200 }),
    staleTime: 30_000,
  });
  const run = runs?.find(r => r.run_id === runId);

  const { data: tasks, isLoading: tasksLoading } = useQuery({
    queryKey: ["run-tasks", runId],
    queryFn: () => defaultApi.getRunTasks(runId),
    retry: false,
  });

  const { data: events, isLoading: eventsLoading } = useQuery({
    queryKey: ["run-events", runId],
    queryFn: () => defaultApi.getRunEvents(runId, 100),
    refetchInterval: 15_000,
    retry: false,
  });

  const { data: cost } = useQuery({
    queryKey: ["run-cost", runId],
    queryFn: () => defaultApi.getRunCost(runId),
    refetchInterval: 15_000,
    retry: false,
  });

  // Defensive: ensure list responses are arrays even if the API returns an unexpected shape.
  const safeTasks  = Array.isArray(tasks)  ? tasks  : undefined;
  const safeEvents = Array.isArray(events) ? events : undefined;

  const isTerminal = run && ["completed", "failed", "canceled"].includes(run.state);
  const duration = run ? fmtDuration(run.created_at, isTerminal ? run.updated_at : undefined) : "—";

  return (
    <div className="h-full overflow-y-auto bg-gray-50 dark:bg-zinc-900">
      <div className="max-w-4xl mx-auto px-5 py-5 space-y-6">

        {/* Back + header */}
        <div className="space-y-3">
          <button
            onClick={onBack ?? (() => { window.location.hash = "runs"; })}
            className="flex items-center gap-1.5 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 transition-colors"
          >
            <ArrowLeft size={13} /> Back to Runs
          </button>

          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-1">Run</p>
              <p className="flex items-center gap-2 text-[15px] font-mono font-medium text-gray-900 dark:text-zinc-100 break-all">
                {runId}
                <CopyButton text={runId} label="Copy run ID" size={12} />
              </p>
              {run && (
                <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-1 font-mono">
                  {run.project.project_id} · {fmtTime(run.created_at)}
                </p>
              )}
            </div>
            <div className="flex items-center gap-3 shrink-0">
              {run && <StateBadge state={run.state} />}
              <button
                onClick={() => {
                  void defaultApi.exportRun(runId).then(data => {
                    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
                    const url  = URL.createObjectURL(blob);
                    const a    = document.createElement('a');
                    a.href     = url;
                    a.download = `run-${runId}.json`;
                    a.click();
                    URL.revokeObjectURL(url);
                  });
                }}
                title="Export run as JSON"
                className="flex items-center gap-1.5 rounded px-2.5 py-1.5 text-[12px] font-medium
                           border border-gray-200 dark:border-zinc-700 text-gray-500 dark:text-zinc-400 hover:text-gray-800 dark:text-zinc-200 hover:border-zinc-600
                           bg-gray-50 dark:bg-zinc-900 transition-colors"
              >
                <Download size={12} /> Export
              </button>
            </div>
          </div>
        </div>

        {/* Stat cards */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-x-6 gap-y-4 py-3 px-4 rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60">
          <StatCard
            label="Duration"
            value={duration}
            sub={isTerminal ? "total" : "running"}
          />
          <StatCard
            label="Tasks"
            value={safeTasks?.length ?? "—"}
            sub={safeTasks ? `${safeTasks.filter(t => t.state === "completed").length} completed` : undefined}
          />
          <StatCard
            label="Events"
            value={safeEvents?.length ?? "—"}
          />
          <StatCard
            label="Cost"
            value={cost ? fmtMicros(cost.total_cost_micros) : "—"}
            sub={cost && cost.provider_calls > 0 ? `${cost.provider_calls} provider call${cost.provider_calls !== 1 ? "s" : ""}` : undefined}
          />
        </div>

        {/* Orchestration live timeline — visible when SSE events arrive */}
        <OrchestrationTimeline runId={runId} />

        {/* Task Gantt chart */}
        {safeTasks && safeTasks.length > 0 && run && (
          <Section title="Task Execution Timeline">
            <GanttView
              runStart={run.created_at}
              runEnd={run && ["completed","failed","canceled"].includes(run.state) ? run.updated_at : undefined}
              tasks={safeTasks}
            />
          </Section>
        )}

        {/* Tasks table */}
        <Section title="Tasks">
          {tasksLoading ? (
            <div className="flex items-center gap-2 text-gray-400 dark:text-zinc-600 text-[13px] py-4">
              <Loader2 size={14} className="animate-spin" /> Loading tasks…
            </div>
          ) : !safeTasks || safeTasks.length === 0 ? (
            <p className="text-[13px] text-gray-400 dark:text-zinc-600 py-4 text-center">No tasks for this run.</p>
          ) : (
            <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-x-auto">
              <table className="min-w-full text-[13px]">
                <thead className="bg-gray-50 dark:bg-zinc-900">
                  <tr>
                    <TH ch="Task ID" />
                    <TH ch="Status" />
                    <TH ch="Worker"  hide="hidden sm:table-cell" />
                    <TH ch="Started" hide="hidden sm:table-cell" />
                    <TH ch="Updated" />
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
                  {safeTasks.map((t, i) => (
                    <tr key={t.task_id} className={clsx(
                      "transition-colors",
                      i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-[#111113]",
                      "hover:bg-gray-100/60 dark:hover:bg-gray-100/60 dark:bg-zinc-800/60",
                    )}>
                      <td className="px-3 py-1.5 font-mono text-gray-700 dark:text-zinc-300 whitespace-nowrap" title={t.task_id}>
                        {shortId(t.task_id)}
                      </td>
                      <td className="px-3 py-1.5 whitespace-nowrap">
                        <StateBadge state={t.state as Parameters<typeof StateBadge>[0]["state"]} compact />
                      </td>
                      <td className="px-3 py-1.5 font-mono text-gray-400 dark:text-zinc-500 text-[12px] whitespace-nowrap hidden sm:table-cell">
                        {t.lease_owner ? <span title={t.lease_owner}>{shortId(t.lease_owner)}</span> : <span className="text-gray-300 dark:text-zinc-700">—</span>}
                      </td>
                      <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap tabular-nums hidden sm:table-cell">
                        {fmtTime(t.created_at)}
                      </td>
                      <td className="px-3 py-1.5 whitespace-nowrap tabular-nums">
                        <span className="text-gray-500 dark:text-zinc-400">{fmtDuration(t.created_at, t.updated_at)}</span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Section>

        {/* Cost breakdown */}
        {cost && (
          <Section title="Cost Breakdown">
            <div className="rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 overflow-hidden">
              <div className="grid grid-cols-2 divide-x divide-gray-200 dark:divide-zinc-800">
                {[
                  { icon: Hash,  label: "Tokens in",       value: fmtTokens(cost.total_tokens_in) },
                  { icon: Hash,  label: "Tokens out",      value: fmtTokens(cost.total_tokens_out) },
                  { icon: Cpu,   label: "Provider calls",  value: String(cost.provider_calls) },
                  { icon: Clock, label: "Total cost (USD)", value: fmtMicros(cost.total_cost_micros) },
                ].map(({ icon: Icon, label, value }) => (
                  <div key={label} className="flex items-center gap-3 px-4 py-3 border-b border-gray-200 dark:border-zinc-800 last:border-0">
                    <Icon size={13} className="text-gray-400 dark:text-zinc-600 shrink-0" />
                    <div>
                      <p className="text-[11px] text-gray-400 dark:text-zinc-500">{label}</p>
                      <p className="text-[13px] font-mono text-gray-800 dark:text-zinc-200">{value}</p>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </Section>
        )}

        {/* Events timeline */}
        <Section title={`Event Timeline${safeEvents ? ` (${safeEvents.length})` : ""}`}>
          {eventsLoading ? (
            <div className="flex items-center gap-2 text-gray-400 dark:text-zinc-600 text-[13px] py-4">
              <Loader2 size={14} className="animate-spin" /> Loading events…
            </div>
          ) : !safeEvents || safeEvents.length === 0 ? (
            <p className="text-[13px] text-gray-400 dark:text-zinc-600 italic py-4">No events recorded.</p>
          ) : (
            <div className="relative">
              {/* Vertical line */}
              <div className="absolute left-[19px] top-2 bottom-2 w-px bg-gray-100 dark:bg-zinc-800" />

              <div className="space-y-0">
                {safeEvents.map((ev, i) => (
                  <div key={`${ev.position}-${i}`} className="flex items-start gap-3 relative py-1.5 group">
                    {/* Dot */}
                    <div className={clsx(
                      "w-2.5 h-2.5 rounded-full shrink-0 mt-1 z-10 ring-2 ring-zinc-900",
                      ev.event_type.includes("fail") || ev.event_type.includes("error")
                        ? "bg-red-500"
                        : ev.event_type.includes("complet")
                        ? "bg-emerald-400"
                        : ev.event_type.includes("start") || ev.event_type.includes("creat")
                        ? "bg-indigo-400"
                        : "bg-zinc-600",
                    )} />

                    {/* Content */}
                    <div className="flex-1 flex items-center gap-2.5 min-w-0 pb-1 border-b border-gray-200/40 dark:border-zinc-800/40 group-last:border-0">
                      <span className={clsx(
                        "shrink-0 rounded px-1.5 py-0.5 text-[10px] font-mono font-medium ring-1 whitespace-nowrap",
                        eventTypeColor(ev.event_type),
                      )}>
                        {ev.event_type.replace(/_/g, "\u202F")}
                      </span>
                      <span className="flex-1" />
                      <span className="shrink-0 text-[11px] text-gray-400 dark:text-zinc-600 font-mono tabular-nums">
                        {fmtTimeShort(ev.stored_at)}
                      </span>
                      <span className="shrink-0 text-[10px] text-gray-300 dark:text-zinc-700 font-mono tabular-nums">
                        #{ev.position}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </Section>

      </div>
    </div>
  );
}

export default RunDetailPage;
