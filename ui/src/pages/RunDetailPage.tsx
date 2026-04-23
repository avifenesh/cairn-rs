import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useEffect, useRef } from "react";
import {
  ArrowLeft, Loader2, Clock, Hash, Cpu, Download,
  Brain, Search, Zap, CheckCircle2, Wrench, ChevronDown, ChevronRight,
  Play, AlertTriangle, FileText, ThumbsUp, ThumbsDown, RotateCcw,
  Bolt, Box, Pause, Stethoscope, LifeBuoy, Lock, GitBranch,
  MessageSquare, Sparkles,
} from "lucide-react";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { StateBadge } from "../components/StateBadge";
import { GanttView } from "../components/TimelineView";
import { CopyButton } from "../components/CopyButton";
import { Drawer } from "../components/Drawer";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import { mapRunActionError, stateGateTooltip } from "../lib/runStateErrors";
import { useEventStream } from "../hooks/useEventStream";
import { table as tablePreset } from "../lib/design-system";
import type {
  RunRecord, InterveneRequest, InterventionAction,
} from "../lib/types";

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

// ── Plan Artifact Panel (RFC 018) ─────────────────────────────────────────────

function PlanArtifactPanel({ runId, run }: { runId: string; run?: import("../lib/types").RunRecord }) {
  const queryClient = useQueryClient();
  const [rejectReason, setRejectReason] = useState("");
  const [reviseComments, setReviseComments] = useState("");
  const [showReject, setShowReject] = useState(false);
  const [showRevise, setShowRevise] = useState(false);

  // Check if this run has a plan artifact via events
  const { data: planEvents } = useQuery({
    queryKey: ["run-plan", runId],
    queryFn: async () => {
      const events = await defaultApi.getRunEvents(runId, 200);
      return events.filter(e =>
        e.event_type === "plan_proposed" ||
        e.event_type === "plan_approved" ||
        e.event_type === "plan_rejected"
      );
    },
    staleTime: 10_000,
  });

  const hasPlan = planEvents && planEvents.length > 0;
  const planProposed = planEvents?.find(e => e.event_type === "plan_proposed");
  const planApproved = planEvents?.find(e => e.event_type === "plan_approved");
  const planRejected = planEvents?.find(e => e.event_type === "plan_rejected");
  const runModeType =
    !run?.mode ? "" :
    typeof run.mode === "string" ? run.mode :
    typeof run.mode.type === "string" ? run.mode.type :
    "";

  // Check if mode is plan by looking at run metadata.
  // Use an exact match on the RunMode discriminator — never a substring check
  // ("deploy-plan", "reviewplan" would match spuriously). Rust's
  // cairn_domain::RunMode is a tagged enum that serializes as `{"type":"plan"}`;
  // the API layer may also surface a bare `"plan"` string for legacy/compat
  // rows. `runModeType` (extracted above) normalizes both into a single string
  // so `runModeType === "plan"` covers both shapes. `hasPlan` keeps
  // plan-artifact-only legacy rows rendering correctly.
  const isPlanMode = run && (
    runModeType === "plan" ||
    hasPlan
  );

  // Approve/reject/revise must also invalidate `run-events` (so the timeline
  // rerenders with the plan-state event) and `approvals` (so any pending
  // approval row on the Approvals tab disappears immediately instead of
  // waiting for the next poll).
  const approveMut = useMutation({
    mutationFn: () => defaultApi.approvePlan(runId, { approved_by: "operator" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["run-plan", runId] });
      queryClient.invalidateQueries({ queryKey: ["runs"] });
      queryClient.invalidateQueries({ queryKey: ["run-events", runId] });
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
    },
  });

  const rejectMut = useMutation({
    mutationFn: () => defaultApi.rejectPlan(runId, { rejected_by: "operator", reason: rejectReason }),
    onSuccess: () => {
      setShowReject(false);
      queryClient.invalidateQueries({ queryKey: ["run-plan", runId] });
      queryClient.invalidateQueries({ queryKey: ["runs"] });
      queryClient.invalidateQueries({ queryKey: ["run-events", runId] });
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
    },
  });

  const reviseMut = useMutation({
    mutationFn: () => defaultApi.revisePlan(runId, { reviewer_comments: reviseComments }),
    onSuccess: () => {
      setShowRevise(false);
      queryClient.invalidateQueries({ queryKey: ["run-plan", runId] });
      queryClient.invalidateQueries({ queryKey: ["runs"] });
      queryClient.invalidateQueries({ queryKey: ["run-events", runId] });
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
    },
  });

  if (!isPlanMode) return null;

  const isPending = !planApproved && !planRejected;

  return (
    <div className="rounded-lg border border-indigo-800/40 bg-indigo-950/20 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-indigo-800/30">
        <div className="flex items-center gap-2">
          <FileText size={14} className="text-indigo-400" />
          <span className="text-[13px] font-medium text-indigo-200">Plan Mode</span>
          <span className="text-[10px] text-indigo-300/70">RFC 018 — Plan/Execute Review</span>
          {planApproved && (
            <span className="px-1.5 py-0.5 rounded text-[10px] font-medium bg-emerald-950 text-emerald-300 border border-emerald-800/50">
              Approved
            </span>
          )}
          {planRejected && (
            <span className="px-1.5 py-0.5 rounded text-[10px] font-medium bg-red-950 text-red-300 border border-red-800/50">
              Rejected
            </span>
          )}
          {isPending && hasPlan && (
            <span className="px-1.5 py-0.5 rounded text-[10px] font-medium bg-amber-950 text-amber-300 border border-amber-800/50">
              Awaiting Review
            </span>
          )}
        </div>
      </div>

      {/* Plan artifact content */}
      {planProposed && (
        <div className="px-4 py-3">
          <p className="text-[11px] text-indigo-400/70 uppercase tracking-wider mb-2">Plan Artifact</p>
          <div className="bg-gray-100 dark:bg-zinc-950/60 rounded-md p-3 max-h-64 overflow-y-auto">
            <pre className="text-[12px] text-gray-700 dark:text-zinc-300 font-mono whitespace-pre-wrap leading-relaxed">
              {(() => {
                const ev = planProposed as unknown as Record<string, unknown>;
                return typeof ev.plan_markdown === "string"
                  ? ev.plan_markdown
                  : JSON.stringify(planProposed, null, 2);
              })()}
            </pre>
          </div>
        </div>
      )}

      {/* Action buttons — only when plan is pending */}
      {isPending && hasPlan && (
        <div className="px-4 py-3 border-t border-indigo-800/30 space-y-3">
          {!showReject && !showRevise && (
            <div className="flex items-center gap-2">
              <button
                onClick={() => approveMut.mutate()}
                disabled={approveMut.isPending}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-emerald-600 text-white text-[12px] font-medium hover:bg-emerald-500 disabled:opacity-50 transition-colors"
              >
                {approveMut.isPending ? <Loader2 size={11} className="animate-spin" /> : <ThumbsUp size={11} />}
                Approve
              </button>
              <button
                onClick={() => setShowReject(true)}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-red-600/80 text-white text-[12px] font-medium hover:bg-red-500 transition-colors"
              >
                <ThumbsDown size={11} /> Reject
              </button>
              <button
                onClick={() => setShowRevise(true)}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-gray-200 dark:bg-zinc-700 text-gray-700 dark:text-zinc-200 text-[12px] font-medium hover:bg-zinc-600 transition-colors"
              >
                <RotateCcw size={11} /> Request Revision
              </button>
            </div>
          )}

          {/* Reject form */}
          {showReject && (
            <div className="space-y-2">
              <textarea
                value={rejectReason}
                onChange={e => setRejectReason(e.target.value)}
                placeholder="Reason for rejection…"
                className="w-full h-20 bg-gray-100 dark:bg-zinc-950 border border-gray-300 dark:border-zinc-700 rounded-md px-3 py-2 text-[12px] text-gray-700 dark:text-zinc-300 resize-none focus:outline-none focus:border-red-500"
              />
              <div className="flex items-center gap-2">
                <button
                  onClick={() => rejectMut.mutate()}
                  disabled={rejectMut.isPending || !rejectReason.trim()}
                  className="px-3 py-1.5 rounded bg-red-600 text-white text-[12px] hover:bg-red-500 disabled:opacity-50 transition-colors"
                >
                  {rejectMut.isPending ? "Rejecting…" : "Confirm Reject"}
                </button>
                <button
                  onClick={() => setShowReject(false)}
                  className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}

          {/* Revise form */}
          {showRevise && (
            <div className="space-y-2">
              <textarea
                value={reviseComments}
                onChange={e => setReviseComments(e.target.value)}
                placeholder="What should be changed in the plan?"
                className="w-full h-20 bg-gray-100 dark:bg-zinc-950 border border-gray-300 dark:border-zinc-700 rounded-md px-3 py-2 text-[12px] text-gray-700 dark:text-zinc-300 resize-none focus:outline-none focus:border-indigo-500"
              />
              <div className="flex items-center gap-2">
                <button
                  onClick={() => reviseMut.mutate()}
                  disabled={reviseMut.isPending || !reviseComments.trim()}
                  className="px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 disabled:opacity-50 transition-colors"
                >
                  {reviseMut.isPending ? "Submitting…" : "Request Revision"}
                </button>
                <button
                  onClick={() => setShowRevise(false)}
                  className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── Operator actions (issues #166/#173) ──────────────────────────────────────

const RUNNING_STATES = new Set([
  "pending", "running", "waiting_approval", "waiting_dependency",
]);
// Mirrors `cairn_domain::RunState::is_terminal()` (completed|failed|canceled)
// and defensively includes `dead_lettered` — the backend never emits it for a
// Run today, but if a DLQ'd task bubbles up as a run-level state string we
// must treat it as terminal, not "still running". `retryable_failed` is
// intentionally excluded: it is semantically pending-retry, not terminal,
// matching `TaskState::is_terminal()` in crates/cairn-domain/src/lifecycle.rs.
const TERMINAL_STATES = new Set([
  "completed",
  "failed",
  "canceled",
  "dead_lettered",
]);

function confirmAction(label: string, runId: string): boolean {
  // Match the existing "confirm" pattern used by cancelRun above — keep it
  // consistent with TriggersPage/ApprovalsPage which also use window.confirm
  // rather than a modal for destructive actions.
  return window.confirm(`${label} run ${runId}?`);
}

interface ActionBtnProps {
  onClick: () => void;
  disabled?: boolean;
  pending?: boolean;
  icon: React.ReactNode;
  label: string;
  variant?: "default" | "danger" | "primary";
  title?: string;
}

function ActionBtn({ onClick, disabled, pending, icon, label, variant = "default", title }: ActionBtnProps) {
  const base = "flex items-center gap-1.5 rounded px-2.5 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40 disabled:cursor-not-allowed";
  const variants = {
    default: "border border-gray-200 dark:border-zinc-700 text-gray-600 dark:text-zinc-300 hover:text-gray-900 dark:hover:text-zinc-100 hover:border-zinc-500 bg-gray-50 dark:bg-zinc-900",
    primary: "border border-indigo-300 dark:border-indigo-700 text-indigo-700 dark:text-indigo-300 hover:text-indigo-900 dark:hover:text-indigo-100 bg-indigo-50 dark:bg-indigo-950/30",
    danger:  "border border-red-200 dark:border-red-800/60 text-red-600 dark:text-red-300 hover:text-red-700 dark:hover:text-red-200 bg-red-50 dark:bg-red-950/30",
  };
  return (
    <button onClick={onClick} disabled={disabled || pending} title={title ?? label} className={clsx(base, variants[variant])}>
      {pending ? <Loader2 size={12} className="animate-spin" /> : icon}
      {label}
    </button>
  );
}

function JsonDrawer({
  open, onClose, title, data,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  data: unknown;
}) {
  return (
    <Drawer open={open} onClose={onClose} title={title} width="w-[28rem]">
      <pre className="text-[11px] font-mono text-gray-700 dark:text-zinc-300 bg-gray-50 dark:bg-zinc-950/50 rounded-md p-3 overflow-auto whitespace-pre-wrap break-all">
        {data === undefined ? "—" : JSON.stringify(data, null, 2)}
      </pre>
    </Drawer>
  );
}

interface SpawnFormState {
  session_id: string;
  child_run_id: string;
  parent_task_id: string;
}

function SpawnSubagentModal({
  runId, defaultSessionId, open, onClose, onSuccess,
}: {
  runId: string;
  defaultSessionId: string;
  open: boolean;
  onClose: () => void;
  onSuccess: (childRunId: string) => void;
}) {
  const [form, setForm] = useState<SpawnFormState>({
    session_id: defaultSessionId,
    child_run_id: "",
    parent_task_id: "",
  });
  const toast = useToast();
  useEffect(() => {
    if (open) setForm({ session_id: defaultSessionId, child_run_id: "", parent_task_id: "" });
  }, [open, defaultSessionId]);

  const mut = useMutation({
    mutationFn: () => defaultApi.spawnSubagentRun(runId, {
      session_id: form.session_id,
      child_run_id: form.child_run_id || undefined,
      parent_task_id: form.parent_task_id || undefined,
    }),
    onSuccess: (r) => {
      toast.success(`Subagent run ${r.child_run_id} spawned.`);
      onSuccess(r.child_run_id);
      onClose();
    },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : "Spawn failed."),
  });

  return (
    <Drawer open={open} onClose={onClose} title="Spawn subagent" width="w-[28rem]">
      <div className="space-y-3">
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 uppercase tracking-wider">Session ID (required)</span>
          <input
            value={form.session_id}
            onChange={e => setForm({ ...form, session_id: e.target.value })}
            placeholder="sess_..."
            className="mt-1 w-full bg-gray-50 dark:bg-zinc-950 border border-gray-300 dark:border-zinc-700 rounded-md px-3 py-1.5 text-[12px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500"
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 uppercase tracking-wider">Parent task ID (optional)</span>
          <input
            value={form.parent_task_id}
            onChange={e => setForm({ ...form, parent_task_id: e.target.value })}
            placeholder="task_..."
            className="mt-1 w-full bg-gray-50 dark:bg-zinc-950 border border-gray-300 dark:border-zinc-700 rounded-md px-3 py-1.5 text-[12px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500"
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 uppercase tracking-wider">Child run ID (optional)</span>
          <input
            value={form.child_run_id}
            onChange={e => setForm({ ...form, child_run_id: e.target.value })}
            placeholder="run_subagent_... (auto)"
            className="mt-1 w-full bg-gray-50 dark:bg-zinc-950 border border-gray-300 dark:border-zinc-700 rounded-md px-3 py-1.5 text-[12px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500"
          />
        </label>
        <div className="flex items-center gap-2 pt-2">
          <button
            onClick={() => mut.mutate()}
            disabled={mut.isPending || !form.session_id.trim()}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] font-medium hover:bg-indigo-500 disabled:opacity-50"
          >
            {mut.isPending ? <Loader2 size={11} className="animate-spin" /> : <GitBranch size={11} />}
            Spawn
          </button>
          <button onClick={onClose} className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700">
            Cancel
          </button>
        </div>
      </div>
    </Drawer>
  );
}

function InterveneModal({
  runId, open, onClose, onSuccess,
}: {
  runId: string;
  open: boolean;
  onClose: () => void;
  onSuccess: () => void;
}) {
  const [action, setAction] = useState<InterventionAction>("force_restart");
  const [reason, setReason] = useState("");
  const [messageBody, setMessageBody] = useState("");
  const toast = useToast();

  useEffect(() => { if (open) { setReason(""); setMessageBody(""); setAction("force_restart"); } }, [open]);

  const mut = useMutation({
    mutationFn: () => {
      const body: InterveneRequest = { action, reason };
      if (action === "inject_message") body.message_body = messageBody;
      return defaultApi.interveneRun(runId, body);
    },
    onSuccess: () => {
      toast.success(`Intervention "${action}" recorded.`);
      onSuccess();
      onClose();
    },
    onError: (e: unknown) => toast.error(mapRunActionError(e, "Intervene failed.", "intervene")),
  });

  const ACTIONS: { id: InterventionAction; label: string; hint: string }[] = [
    { id: "force_complete", label: "Force complete",  hint: "Mark the run completed regardless of state." },
    { id: "force_fail",     label: "Force fail",      hint: "Mark the run failed with the given reason." },
    { id: "force_restart",  label: "Force restart",   hint: "Cancel + restart the run." },
    { id: "inject_message", label: "Inject message",  hint: "Deliver an operator message to the run." },
  ];

  return (
    <Drawer open={open} onClose={onClose} title="Intervene" width="w-[28rem]">
      <div className="space-y-3">
        <div className="space-y-1">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 uppercase tracking-wider">Action</span>
          <div className="grid grid-cols-2 gap-1">
            {ACTIONS.map(a => (
              <button
                key={a.id}
                onClick={() => setAction(a.id)}
                title={a.hint}
                className={clsx(
                  "rounded border px-2 py-1.5 text-[11px] text-left",
                  action === a.id
                    ? "border-indigo-500 bg-indigo-50 dark:bg-indigo-950/40 text-indigo-800 dark:text-indigo-200"
                    : "border-gray-200 dark:border-zinc-700 text-gray-500 dark:text-zinc-400 hover:border-zinc-500",
                )}
              >
                {a.label}
              </button>
            ))}
          </div>
        </div>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 uppercase tracking-wider">Reason</span>
          <textarea
            value={reason}
            onChange={e => setReason(e.target.value)}
            placeholder="Why is this intervention needed?"
            className="mt-1 w-full h-20 bg-gray-50 dark:bg-zinc-950 border border-gray-300 dark:border-zinc-700 rounded-md px-3 py-2 text-[12px] text-gray-700 dark:text-zinc-300 resize-none focus:outline-none focus:border-indigo-500"
          />
        </label>
        {action === "inject_message" && (
          <label className="block">
            <span className="text-[11px] text-gray-500 dark:text-zinc-400 uppercase tracking-wider">Message body</span>
            <textarea
              value={messageBody}
              onChange={e => setMessageBody(e.target.value)}
              placeholder="Operator message to inject…"
              className="mt-1 w-full h-20 bg-gray-50 dark:bg-zinc-950 border border-gray-300 dark:border-zinc-700 rounded-md px-3 py-2 text-[12px] text-gray-700 dark:text-zinc-300 resize-none focus:outline-none focus:border-indigo-500"
            />
          </label>
        )}
        <div className="flex items-center gap-2 pt-2">
          <button
            onClick={() => mut.mutate()}
            disabled={mut.isPending || !reason.trim() || (action === "inject_message" && !messageBody.trim())}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] font-medium hover:bg-indigo-500 disabled:opacity-50"
          >
            {mut.isPending ? <Loader2 size={11} className="animate-spin" /> : <MessageSquare size={11} />}
            Submit intervention
          </button>
          <button onClick={onClose} className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700">
            Cancel
          </button>
        </div>
      </div>
    </Drawer>
  );
}

function OperatorActions({ runId, run }: { runId: string; run?: RunRecord }) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const [spawnOpen, setSpawnOpen] = useState(false);
  const [interveneOpen, setInterveneOpen] = useState(false);
  const [diagnoseDrawer, setDiagnoseDrawer] = useState<{ open: boolean; data?: unknown }>({ open: false });
  const [interventionsOpen, setInterventionsOpen] = useState(false);

  const invalidateRun = () => {
    void queryClient.invalidateQueries({ queryKey: ["run-detail", runId] });
    void queryClient.invalidateQueries({ queryKey: ["run-events", runId] });
    void queryClient.invalidateQueries({ queryKey: ["runs"] });
  };

  const pauseMut = useMutation({
    mutationFn: () => defaultApi.pauseRun(runId, { reason_kind: "operator_pause", actor: "operator" }),
    onSuccess: () => { toast.success("Run paused."); invalidateRun(); },
    onError: (e: unknown) => toast.error(mapRunActionError(e, "Pause failed.", "pause")),
  });
  const resumeMut = useMutation({
    mutationFn: () => defaultApi.resumeRun(runId, { trigger: "operator_resume", target: "running" }),
    onSuccess: () => { toast.success("Run resumed."); invalidateRun(); },
    onError: (e: unknown) => toast.error(mapRunActionError(e, "Resume failed.", "resume")),
  });
  const recoverMut = useMutation({
    mutationFn: () => defaultApi.recoverRun(runId),
    onSuccess: (r) => {
      toast.success(r.deprecated ? "Recover acknowledged (handled by background scanners)." : "Recovery requested.");
      invalidateRun();
    },
    onError: (e: unknown) => toast.error(mapRunActionError(e, "Recover failed.", "recover")),
  });
  const claimMut = useMutation({
    mutationFn: () => defaultApi.claimRun(runId),
    onSuccess: () => { toast.success("Run claimed for inspection."); invalidateRun(); },
    onError: (e: unknown) => toast.error(mapRunActionError(e, "Claim failed.", "claim")),
  });
  const orchestrateMut = useMutation({
    mutationFn: () => defaultApi.orchestrateRun(runId, {}),
    onSuccess: () => { toast.success("Orchestration step triggered."); invalidateRun(); },
    onError: (e: unknown) => toast.error(mapRunActionError(e, "Orchestrate failed.", "orchestrate")),
  });
  const diagnoseMut = useMutation({
    mutationFn: () => defaultApi.diagnoseRun(runId),
    onSuccess: (data) => { setDiagnoseDrawer({ open: true, data }); },
    onError: (e: unknown) => toast.error(mapRunActionError(e, "Diagnose failed.", "diagnose")),
  });

  const { data: interventions } = useQuery({
    queryKey: ["run-interventions", runId],
    queryFn: () => defaultApi.listRunInterventions(runId),
    enabled: interventionsOpen,
    staleTime: 5_000,
  });

  const state = run?.state;
  const canPause  = state !== undefined && RUNNING_STATES.has(state) && state !== "paused";
  const canResume = state === "paused";
  const isTerminal = state !== undefined && TERMINAL_STATES.has(state);

  return (
    <div className="rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60 px-4 py-3">
      <div className="flex items-center justify-between mb-2">
        <p className="text-[11px] font-semibold text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
          Operator Actions
        </p>
        <button
          onClick={() => setInterventionsOpen(true)}
          className="text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 underline-offset-2 hover:underline"
        >
          History
        </button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <ActionBtn
          icon={<Pause size={12} />}
          label="Pause"
          onClick={() => pauseMut.mutate()}
          disabled={!canPause}
          pending={pauseMut.isPending}
          title={canPause ? "Pause this run" : stateGateTooltip("pause", state)}
        />
        <ActionBtn
          icon={<Play size={12} />}
          label="Resume"
          variant="primary"
          onClick={() => resumeMut.mutate()}
          disabled={!canResume}
          pending={resumeMut.isPending}
          title={canResume ? "Resume paused run" : stateGateTooltip("resume", state)}
        />
        <ActionBtn
          icon={<Sparkles size={12} />}
          label="Orchestrate"
          onClick={() => orchestrateMut.mutate()}
          disabled={isTerminal}
          pending={orchestrateMut.isPending}
          title={isTerminal ? stateGateTooltip("orchestrate", state) : "Drive the orchestration loop one step"}
        />
        <ActionBtn
          icon={<Stethoscope size={12} />}
          label="Diagnose"
          onClick={() => diagnoseMut.mutate()}
          pending={diagnoseMut.isPending}
          title="Run the diagnosis report"
        />
        <ActionBtn
          icon={<MessageSquare size={12} />}
          label="Intervene"
          variant="primary"
          onClick={() => setInterveneOpen(true)}
          disabled={isTerminal}
          title={isTerminal ? stateGateTooltip("intervene", state) : "Operator intervention (force complete/fail/restart or inject message)"}
        />
        <ActionBtn
          icon={<GitBranch size={12} />}
          label="Spawn subagent"
          onClick={() => setSpawnOpen(true)}
          disabled={!run}
          title="Spawn a child run"
        />
        <ActionBtn
          icon={<LifeBuoy size={12} />}
          label="Recover"
          variant="danger"
          onClick={() => {
            if (!confirmAction("Recover (re-trigger scanners on)", runId)) return;
            recoverMut.mutate();
          }}
          pending={recoverMut.isPending}
          title="Re-trigger the recovery scanners (legacy no-op in v1)"
        />
        <ActionBtn
          icon={<Lock size={12} />}
          label="Claim"
          variant="danger"
          onClick={() => {
            if (!confirmAction("Take operator claim on", runId)) return;
            claimMut.mutate();
          }}
          pending={claimMut.isPending}
          title="Take an admin claim on this run for inspection"
        />
      </div>

      {run && (
        <SpawnSubagentModal
          runId={runId}
          defaultSessionId={run.session_id}
          open={spawnOpen}
          onClose={() => setSpawnOpen(false)}
          onSuccess={() => {
            invalidateRun();
            void queryClient.invalidateQueries({ queryKey: ["run-children", runId] });
          }}
        />
      )}
      <InterveneModal
        runId={runId}
        open={interveneOpen}
        onClose={() => setInterveneOpen(false)}
        onSuccess={() => {
          invalidateRun();
          void queryClient.invalidateQueries({ queryKey: ["run-interventions", runId] });
        }}
      />
      <JsonDrawer
        open={diagnoseDrawer.open}
        onClose={() => setDiagnoseDrawer({ open: false })}
        title="Diagnosis report"
        data={diagnoseDrawer.data}
      />
      <Drawer
        open={interventionsOpen}
        onClose={() => setInterventionsOpen(false)}
        title={`Interventions${interventions ? ` (${interventions.length})` : ""}`}
        width="w-[28rem]"
      >
        {!interventions ? (
          <p className="text-[12px] text-gray-400 dark:text-zinc-500">Loading…</p>
        ) : interventions.length === 0 ? (
          <p className="text-[12px] text-gray-400 dark:text-zinc-500">No interventions recorded.</p>
        ) : (
          <ul className="space-y-2">
            {interventions.map((iv, i) => (
              <li key={`${iv.intervened_at_ms}-${i}`} className="rounded border border-gray-200 dark:border-zinc-800 p-2">
                <div className="flex items-center gap-2 text-[11px]">
                  <span className="font-mono font-medium text-indigo-700 dark:text-indigo-300">{iv.action}</span>
                  <span className="text-gray-400 dark:text-zinc-600 tabular-nums">
                    {new Date(iv.intervened_at_ms).toLocaleString()}
                  </span>
                </div>
                <p className="mt-1 text-[12px] text-gray-700 dark:text-zinc-300 whitespace-pre-wrap">{iv.reason}</p>
              </li>
            ))}
          </ul>
        )}
      </Drawer>
    </div>
  );
}

function ChildRunsSection({ runId }: { runId: string }) {
  const { data: children, isLoading } = useQuery({
    queryKey: ["run-children", runId],
    queryFn: () => defaultApi.listChildRuns(runId),
    refetchInterval: 15_000,
    retry: false,
  });

  if (isLoading) {
    return (
      <Section title="Children runs">
        <div className="flex items-center gap-2 text-gray-400 dark:text-zinc-600 text-[13px] py-4">
          <Loader2 size={14} className="animate-spin" /> Loading children…
        </div>
      </Section>
    );
  }
  if (!children || children.length === 0) return null;

  return (
    <Section title={`Children runs (${children.length})`}>
      <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-x-auto">
        <table className="min-w-full text-[13px]">
          <thead className="bg-gray-50 dark:bg-zinc-900">
            <tr>
              <TH ch="Run ID" />
              <TH ch="State" />
              <TH ch="Created" />
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
            {children.map((c, i) => (
              <tr
                key={c.run_id}
                className={clsx(
                  "transition-colors cursor-pointer",
                  i % 2 === 0 ? tablePreset.rowEven : tablePreset.rowOdd,
                  "hover:bg-gray-100/60 dark:hover:bg-zinc-800/60",
                )}
                onClick={() => { window.location.hash = `run/${c.run_id}`; }}
              >
                <td className="px-3 py-1.5 font-mono text-gray-700 dark:text-zinc-300 whitespace-nowrap" title={c.run_id}>
                  {shortId(c.run_id)}
                </td>
                <td className="px-3 py-1.5 whitespace-nowrap">
                  <StateBadge state={c.state} compact />
                </td>
                <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap tabular-nums">
                  {fmtTime(c.created_at)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Section>
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
    right ? tablePreset.thRight : tablePreset.th,
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
  const queryClient = useQueryClient();
  const toast = useToast();

  const { data: run } = useQuery({
    queryKey: ["run-detail", runId],
    queryFn: () => defaultApi.getRun(runId),
    staleTime: 10_000,
  });

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

  const cancelRunMut = useMutation({
    mutationFn: () => defaultApi.cancelRun(runId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["runs"] });
      void queryClient.invalidateQueries({ queryKey: ["run-detail", runId] });
      void queryClient.invalidateQueries({ queryKey: ["run-events", runId] });
      toast.success(`Run ${runId} canceled.`);
    },
    onError: (error: unknown) => {
      toast.error(error instanceof Error ? error.message : "Failed to cancel run.");
    },
  });

  const isTerminal = run && TERMINAL_STATES.has(run.state);
  const duration = run ? fmtDuration(run.created_at, isTerminal ? run.updated_at : undefined) : "—";

  // The backend returns a zero-valued RunCostRecord (HTTP 200) for runs with no cost
  // data instead of 404, so we treat "no provider calls AND zero cost" as "no cost
  // data yet" and render an em-dash instead of a misleading "$0.000000".
  const hasCostData = !!cost && (cost.provider_calls > 0 || cost.total_cost_micros > 0);
  const costValue = hasCostData ? fmtMicros(cost!.total_cost_micros) : "—";
  const costDescription = hasCostData
    ? `${cost!.provider_calls} provider call${cost!.provider_calls !== 1 ? "s" : ""}`
    : undefined;

  return (
    <div className="h-full overflow-y-auto bg-gray-50 dark:bg-zinc-900">
      <div className="max-w-4xl mx-auto px-5 py-5 space-y-6">

        {/* Back + header */}
        <div className="space-y-3">
          <button
            onClick={onBack ?? (() => { window.location.hash = "runs"; })}
            className="flex items-center gap-1.5 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
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
              {run && !isTerminal && (
                <button
                  onClick={() => {
                    if (!window.confirm(`Cancel run ${runId}?`)) return;
                    cancelRunMut.mutate();
                  }}
                  disabled={cancelRunMut.isPending}
                  title="Cancel this run"
                  className="flex items-center gap-1.5 rounded px-2.5 py-1.5 text-[12px] font-medium
                             border border-red-200 dark:border-red-800/60 text-red-600 dark:text-red-300 hover:text-red-700 dark:hover:text-red-200
                             bg-red-50 dark:bg-red-950/30 transition-colors disabled:opacity-50"
                >
                  {cancelRunMut.isPending ? <Loader2 size={12} className="animate-spin" /> : <AlertTriangle size={12} />}
                  Cancel Run
                </button>
              )}
              <button
                onClick={() => {
                  void defaultApi.exportRun(runId)
                    .then(data => {
                      const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
                      const url  = URL.createObjectURL(blob);
                      const a    = document.createElement('a');
                      a.href     = url;
                      a.download = `run-${runId}.json`;
                      a.click();
                      URL.revokeObjectURL(url);
                    })
                    .catch(e => toast.error(`Export failed: ${e instanceof Error ? e.message : String(e)}`));
                }}
                title="Export run as JSON"
                className="flex items-center gap-1.5 rounded px-2.5 py-1.5 text-[12px] font-medium
                           border border-gray-200 dark:border-zinc-700 text-gray-500 dark:text-zinc-400 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600
                           bg-gray-50 dark:bg-zinc-900 transition-colors"
              >
                <Download size={12} /> Export
              </button>
            </div>
          </div>
        </div>

        {/* Stat cards */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-x-6 gap-y-4 py-3 px-4 rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60">
          <StatCard compact variant="info"
            label="Duration"
            value={duration}
            description={isTerminal ? "total" : "running"}
          />
          <StatCard compact variant="info"
            label="Tasks"
            value={safeTasks?.length ?? "—"}
            description={safeTasks ? `${safeTasks.filter(t => t.state === "completed").length} completed` : undefined}
          />
          <StatCard compact variant="info"
            label="Events"
            value={safeEvents?.length ?? "—"}
          />
          <StatCard compact variant="info"
            label="Cost"
            value={costValue}
            description={costDescription}
          />
        </div>

        {/* Trigger origin badge */}
        {run?.created_by_trigger_id && (
          <div className="flex items-center gap-2 px-4 py-2.5 rounded-lg border border-amber-800/40 bg-amber-950/20">
            <Bolt size={13} className="text-amber-400 shrink-0" />
            <span className="text-[12px] text-amber-300">
              Created by trigger: <span className="font-mono font-medium">{run.created_by_trigger_id}</span>
            </span>
          </div>
        )}

        {/* Sandbox status */}
        {run?.sandbox_id && (
          <div className="flex items-center gap-2 px-4 py-2.5 rounded-lg border border-teal-800/40 bg-teal-950/20">
            <Box size={13} className="text-teal-400 shrink-0" />
            <span className="text-[12px] text-teal-300">
              Sandbox: <span className="font-mono font-medium">{run.sandbox_id}</span>
              {run.sandbox_path && (
                <span className="text-teal-500 ml-2">{run.sandbox_path}</span>
              )}
            </span>
          </div>
        )}

        {/* Operator actions (issues #166/#173) */}
        <OperatorActions runId={runId} run={run} />

        {/* Plan artifact (RFC 018 — Plan mode) */}
        <PlanArtifactPanel runId={runId} run={run} />

        {/* Orchestration live timeline — visible when SSE events arrive */}
        <OrchestrationTimeline runId={runId} />

        {/* Task Gantt chart */}
        {safeTasks && safeTasks.length > 0 && run && (
          <Section title="Task Execution Timeline">
            <GanttView
              runStart={run.created_at}
              runEnd={run && TERMINAL_STATES.has(run.state) ? run.updated_at : undefined}
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
                      i % 2 === 0 ? tablePreset.rowEven : tablePreset.rowOdd,
                      "hover:bg-gray-100/60 dark:hover:bg-gray-100/60 dark:bg-zinc-800/60",
                    )}>
                      <td className="px-3 py-1.5 font-mono text-gray-700 dark:text-zinc-300 whitespace-nowrap" title={t.task_id}>
                        {shortId(t.task_id)}
                      </td>
                      <td className="px-3 py-1.5 whitespace-nowrap">
                        <StateBadge state={t.state as Parameters<typeof StateBadge>[0]["state"]} compact />
                      </td>
                      <td className="px-3 py-1.5 font-mono text-gray-400 dark:text-zinc-500 text-[12px] whitespace-nowrap hidden sm:table-cell">
                        {t.lease_owner ? <span title={t.lease_owner}>{shortId(t.lease_owner)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span>}
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

        {/* Children subagent runs (issues #166/#173) */}
        <ChildRunsSection runId={runId} />

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
                      <span className="shrink-0 text-[10px] text-gray-300 dark:text-zinc-600 font-mono tabular-nums">
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
