import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeft, Loader2, AlertTriangle, CheckCircle2, Inbox, Download,
} from "lucide-react";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { StateBadge } from "../components/StateBadge";
import { CopyButton } from "../components/CopyButton";
import { defaultApi } from "../lib/api";
import type { SessionState } from "../lib/types";

// ── Helpers ────────────────────────────────────────────────────────────────────

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-7)}` : id;

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  });

const fmtTokens = (n: number) =>
  n >= 1_000 ? `${(n / 1_000).toFixed(1)}k` : String(n);

const fmtLatency = (ms: number) =>
  ms >= 1_000 ? `${(ms / 1_000).toFixed(2)}s` : `${ms}ms`;

const fmtCost = (micros: number) =>
  micros === 0 ? "—" : `$${(micros / 1_000_000).toFixed(5)}`;

function inferProvider(modelId: string): string {
  const m = modelId.toLowerCase();
  if (m.startsWith("gpt") || m.startsWith("o1") || m.startsWith("o3")) return "OpenAI";
  if (m.startsWith("claude"))  return "Anthropic";
  if (m.startsWith("gemini"))  return "Google";
  if (m.startsWith("llama") || m.startsWith("qwen") || m.startsWith("mistral") ||
      m.startsWith("nomic"))   return "Open-Weight";
  if (m.startsWith("titan") || m.startsWith("nova")) return "Bedrock";
  return "—";
}

// ── Session state pill ────────────────────────────────────────────────────────

const SESSION_PILL: Record<SessionState, string> = {
  open:      "bg-blue-500/10 text-blue-400 border-blue-500/20",
  completed: "bg-emerald-500/10 text-emerald-400 border-emerald-500/20",
  failed:    "bg-red-500/10 text-red-400 border-red-500/20",
  archived:  "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500 border-gray-200 dark:border-zinc-700",
};
const SESSION_DOT: Record<SessionState, string> = {
  open:      "bg-blue-400 animate-pulse",
  completed: "bg-emerald-400",
  failed:    "bg-red-400",
  archived:  "bg-zinc-600",
};

function SessionPill({ state }: { state: SessionState }) {
  return (
    <span className={clsx(
      "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border whitespace-nowrap",
      SESSION_PILL[state],
    )}>
      <span className={clsx("w-1 h-1 rounded-full shrink-0", SESSION_DOT[state])} />
      {state}
    </span>
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

// ── Page ──────────────────────────────────────────────────────────────────────

interface SessionDetailPageProps {
  sessionId: string;
  onBack?: () => void;
}

export function SessionDetailPage({ sessionId, onBack }: SessionDetailPageProps) {
  // Fetch session metadata from the list.
  const { data: sessions } = useQuery({
    queryKey: ["sessions"],
    queryFn: () => defaultApi.getSessions({ limit: 200 }),
    staleTime: 30_000,
  });
  const session = sessions?.find(s => s.session_id === sessionId);

  // All runs — filter client-side for this session.
  const { data: allRuns, isLoading: runsLoading } = useQuery({
    queryKey: ["runs"],
    queryFn: () => defaultApi.getRuns({ limit: 500 }),
    staleTime: 30_000,
  });
  const runs = (allRuns ?? []).filter(r => r.session_id === sessionId);
  const activeRuns = runs.filter(r => r.state === "running" || r.state === "pending").length;

  // LLM traces for this session.
  const { data: tracesData, isLoading: tracesLoading } = useQuery({
    queryKey: ["session-traces", sessionId],
    queryFn: () => defaultApi.getSessionTraces(sessionId, 200),
    refetchInterval: 30_000,
    retry: false,
  });
  const traces = tracesData?.traces ?? [];

  return (
    <div className="h-full overflow-y-auto bg-gray-50 dark:bg-zinc-900">
      <div className="max-w-4xl mx-auto px-5 py-5 space-y-6">

        {/* Back + header */}
        <div className="space-y-3">
          <button
            onClick={onBack ?? (() => { window.location.hash = "sessions"; })}
            className="flex items-center gap-1.5 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
          >
            <ArrowLeft size={13} /> Back to Sessions
          </button>

          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-1">Session</p>
              <p className="flex items-center gap-2 text-[15px] font-mono font-medium text-gray-900 dark:text-zinc-100 break-all">
                {sessionId}
                <CopyButton text={sessionId} label="Copy session ID" size={12} />
              </p>
              {session && (
                <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-1 font-mono">
                  {session.project.tenant_id}
                  <span className="text-gray-300 dark:text-zinc-600 mx-1">/</span>
                  {session.project.workspace_id}
                  <span className="text-gray-300 dark:text-zinc-600 mx-1">/</span>
                  {session.project.project_id}
                  <span className="text-gray-300 dark:text-zinc-600 ml-2 mr-1">·</span>
                  {fmtTime(session.created_at)}
                </p>
              )}
            </div>
            <div className="flex items-center gap-3 shrink-0">
              {session && <SessionPill state={session.state} />}
              <button
                onClick={() => {
                  void defaultApi.exportSession(sessionId).then(data => {
                    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
                    const url  = URL.createObjectURL(blob);
                    const a    = document.createElement('a');
                    a.href     = url;
                    a.download = `session-${sessionId}.json`;
                    a.click();
                    URL.revokeObjectURL(url);
                  });
                }}
                title="Export session as JSON"
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
            label="Runs"
            value={runsLoading ? "—" : runs.length}
            description={runsLoading ? undefined : `${activeRuns} active`}
          />
          <StatCard compact variant="info"
            label="Traces"
            value={tracesLoading ? "—" : traces.length}
            description={tracesLoading ? undefined : `${traces.filter(t => t.is_error).length} errors`}
          />
          <StatCard compact variant="info"
            label="Tokens"
            value={tracesLoading ? "—" : fmtTokens(
              traces.reduce((s, t) => s + t.prompt_tokens + t.completion_tokens, 0)
            )}
            description="prompt + completion"
          />
          <StatCard compact variant="info"
            label="Cost"
            value={tracesLoading ? "—" : fmtCost(
              traces.reduce((s, t) => s + t.cost_micros, 0)
            )}
          />
        </div>

        {/* Runs table */}
        <Section title="Runs">
          {runsLoading ? (
            <div className="flex items-center gap-2 text-gray-400 dark:text-zinc-600 text-[13px] py-4">
              <Loader2 size={14} className="animate-spin" /> Loading runs…
            </div>
          ) : runs.length === 0 ? (
            <p className="text-[13px] text-gray-400 dark:text-zinc-600 italic py-4">No runs in this session.</p>
          ) : (
            <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-x-auto">
              <table className="min-w-full text-[13px]">
                <thead className="bg-gray-50 dark:bg-zinc-900">
                  <tr>
                    <TH ch="Run ID" />
                    <TH ch="State" />
                    <TH ch="Parent"  hide="hidden sm:table-cell" />
                    <TH ch="Prompt"  hide="hidden md:table-cell" />
                    <TH ch="Created" />
                    <TH ch="Updated" hide="hidden md:table-cell" />
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
                  {runs.map((run, i) => (
                    <tr
                      key={run.run_id}
                      onClick={() => { window.location.hash = `run/${run.run_id}`; }}
                      className={clsx(
                        "cursor-pointer transition-colors",
                        i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-[#111113]",
                        "hover:bg-gray-100/60 dark:hover:bg-gray-100/60 dark:bg-zinc-800/60",
                      )}
                    >
                      <td className="px-3 py-1.5 font-mono text-gray-700 dark:text-zinc-300 whitespace-nowrap text-[12px]" title={run.run_id}>
                        {shortId(run.run_id)}
                      </td>
                      <td className="px-3 py-1.5 whitespace-nowrap">
                        <StateBadge state={run.state} compact />
                      </td>
                      <td className="px-3 py-1.5 font-mono text-gray-400 dark:text-zinc-600 text-[11px] whitespace-nowrap hidden sm:table-cell">
                        {run.parent_run_id ? <span title={run.parent_run_id}>{shortId(run.parent_run_id)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span>}
                      </td>
                      <td className="px-3 py-1.5 font-mono text-gray-400 dark:text-zinc-600 text-[11px] whitespace-nowrap hidden md:table-cell">
                        {run.prompt_release_id ? <span title={run.prompt_release_id}>{shortId(run.prompt_release_id)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span>}
                      </td>
                      <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap text-[12px] tabular-nums">
                        {fmtTime(run.created_at)}
                      </td>
                      <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap text-[12px] tabular-nums hidden md:table-cell">
                        {fmtTime(run.updated_at)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Section>

        {/* LLM Traces table */}
        <Section title={`LLM Traces${traces.length > 0 ? ` (${traces.length})` : ""}`}>
          {tracesLoading ? (
            <div className="flex items-center gap-2 text-gray-400 dark:text-zinc-600 text-[13px] py-4">
              <Loader2 size={14} className="animate-spin" /> Loading traces…
            </div>
          ) : traces.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-12 gap-2 text-gray-300 dark:text-zinc-600">
              <Inbox size={22} />
              <p className="text-[13px]">No traces for this session</p>
            </div>
          ) : (
            <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden overflow-x-auto">
              <table className="min-w-full text-[13px]">
                <thead className="bg-gray-50 dark:bg-zinc-900">
                  <tr>
                    <TH ch="Trace ID" hide="hidden sm:table-cell" />
                    <TH ch="Model" />
                    <TH ch="Provider" hide="hidden sm:table-cell" />
                    <TH ch="Status" />
                    <TH ch="In"      right hide="hidden md:table-cell" />
                    <TH ch="Out"     right hide="hidden md:table-cell" />
                    <TH ch="Latency" right />
                    <TH ch="Cost"    right hide="hidden sm:table-cell" />
                    <TH ch="Time"          hide="hidden sm:table-cell" />
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
                  {traces.map((trace, i) => (
                    <tr key={trace.trace_id} className={clsx(
                      "transition-colors",
                      i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-[#111113]",
                      "hover:bg-gray-100/60 dark:hover:bg-gray-100/60 dark:bg-zinc-800/60",
                    )}>
                      <td className="px-3 py-1.5 font-mono text-gray-500 dark:text-zinc-400 whitespace-nowrap text-[12px] hidden sm:table-cell">
                        {shortId(trace.trace_id)}
                      </td>
                      <td className="px-3 py-1.5 font-mono text-gray-700 dark:text-zinc-300 whitespace-nowrap text-[12px]">
                        {trace.model_id}
                      </td>
                      <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap text-[12px] hidden sm:table-cell">
                        {inferProvider(trace.model_id)}
                      </td>
                      <td className="px-3 py-1.5 whitespace-nowrap">
                        {trace.is_error ? (
                          <span className="inline-flex items-center gap-1 text-[11px] text-red-400">
                            <AlertTriangle size={10} /> Error
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 text-[11px] text-emerald-400">
                            <CheckCircle2 size={10} /> OK
                          </span>
                        )}
                      </td>
                      <td className="px-3 py-1.5 text-gray-500 dark:text-zinc-400 whitespace-nowrap tabular-nums text-right font-mono text-[12px] hidden md:table-cell">
                        {fmtTokens(trace.prompt_tokens)}
                      </td>
                      <td className="px-3 py-1.5 text-gray-500 dark:text-zinc-400 whitespace-nowrap tabular-nums text-right font-mono text-[12px] hidden md:table-cell">
                        {fmtTokens(trace.completion_tokens)}
                      </td>
                      <td className={clsx(
                        "px-3 py-1.5 whitespace-nowrap tabular-nums text-right font-mono text-[12px]",
                        trace.latency_ms > 5_000 ? "text-amber-400" : "text-gray-500 dark:text-zinc-400",
                      )}>
                        {fmtLatency(trace.latency_ms)}
                      </td>
                      <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap tabular-nums text-right font-mono text-[12px] hidden sm:table-cell">
                        {fmtCost(trace.cost_micros)}
                      </td>
                      <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap tabular-nums text-[12px] hidden sm:table-cell">
                        {fmtTime(trace.created_at_ms)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Section>

      </div>
    </div>
  );
}

export default SessionDetailPage;
