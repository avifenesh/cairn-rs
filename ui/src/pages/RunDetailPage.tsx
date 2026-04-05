import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeft, Loader2, Clock, Hash, Cpu,
} from "lucide-react";
import { clsx } from "clsx";
import { StateBadge } from "../components/StateBadge";
import { defaultApi } from "../lib/api";

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
      <p className="text-[11px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

// ── Section wrapper ────────────────────────────────────────────────────────────

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[11px] font-semibold text-zinc-500 uppercase tracking-wider mb-2">
        {title}
      </p>
      {children}
    </div>
  );
}

const TH = ({ ch, right, hide }: { ch: React.ReactNode; right?: boolean; hide?: string }) => (
  <th className={clsx(
    "px-3 py-2 text-[11px] font-medium text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-zinc-800",
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
  return "bg-zinc-800 text-zinc-400 ring-zinc-700";
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

  const isTerminal = run && ["completed", "failed", "canceled"].includes(run.state);
  const duration = run ? fmtDuration(run.created_at, isTerminal ? run.updated_at : undefined) : "—";

  return (
    <div className="h-full overflow-y-auto bg-zinc-900">
      <div className="max-w-4xl mx-auto px-5 py-5 space-y-6">

        {/* Back + header */}
        <div className="space-y-3">
          <button
            onClick={onBack ?? (() => { window.location.hash = "runs"; })}
            className="flex items-center gap-1.5 text-[12px] text-zinc-500 hover:text-zinc-300 transition-colors"
          >
            <ArrowLeft size={13} /> Back to Runs
          </button>

          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <p className="text-[11px] text-zinc-500 uppercase tracking-wider mb-1">Run</p>
              <p className="text-[15px] font-mono font-medium text-zinc-100 break-all">{runId}</p>
              {run && (
                <p className="text-[12px] text-zinc-500 mt-1 font-mono">
                  {run.project.project_id} · {fmtTime(run.created_at)}
                </p>
              )}
            </div>
            <div className="flex items-center gap-3 shrink-0">
              {run && <StateBadge state={run.state} />}
            </div>
          </div>
        </div>

        {/* Stat cards */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-x-6 gap-y-4 py-3 px-4 rounded-lg border border-zinc-800 bg-zinc-900/60">
          <StatCard
            label="Duration"
            value={duration}
            sub={isTerminal ? "total" : "running"}
          />
          <StatCard
            label="Tasks"
            value={tasks?.length ?? "—"}
            sub={tasks ? `${tasks.filter(t => t.state === "completed").length} completed` : undefined}
          />
          <StatCard
            label="Events"
            value={events?.length ?? "—"}
          />
          <StatCard
            label="Cost"
            value={cost ? fmtMicros(cost.total_cost_micros) : "—"}
            sub={cost && cost.provider_calls > 0 ? `${cost.provider_calls} provider call${cost.provider_calls !== 1 ? "s" : ""}` : undefined}
          />
        </div>

        {/* Tasks table */}
        <Section title="Tasks">
          {tasksLoading ? (
            <div className="flex items-center gap-2 text-zinc-600 text-[13px] py-4">
              <Loader2 size={14} className="animate-spin" /> Loading tasks…
            </div>
          ) : !tasks || tasks.length === 0 ? (
            <p className="text-[13px] text-zinc-600 italic py-4">No tasks for this run.</p>
          ) : (
            <div className="rounded-lg border border-zinc-800 overflow-hidden">
              <table className="min-w-full text-[13px]">
                <thead className="bg-zinc-900">
                  <tr>
                    <TH ch="Task ID" />
                    <TH ch="Status" />
                    <TH ch="Worker"  hide="hidden sm:table-cell" />
                    <TH ch="Started" hide="hidden sm:table-cell" />
                    <TH ch="Updated" />
                  </tr>
                </thead>
                <tbody className="divide-y divide-zinc-800/50">
                  {tasks.map((t, i) => (
                    <tr key={t.task_id} className={clsx(
                      "transition-colors",
                      i % 2 === 0 ? "bg-zinc-900" : "bg-[#111113]",
                      "hover:bg-zinc-800/60",
                    )}>
                      <td className="px-3 py-1.5 font-mono text-zinc-300 whitespace-nowrap">
                        {shortId(t.task_id)}
                      </td>
                      <td className="px-3 py-1.5 whitespace-nowrap">
                        <StateBadge state={t.state as Parameters<typeof StateBadge>[0]["state"]} compact />
                      </td>
                      <td className="px-3 py-1.5 font-mono text-zinc-500 text-[12px] whitespace-nowrap hidden sm:table-cell">
                        {t.lease_owner ? shortId(t.lease_owner) : <span className="text-zinc-700">—</span>}
                      </td>
                      <td className="px-3 py-1.5 text-zinc-500 whitespace-nowrap tabular-nums hidden sm:table-cell">
                        {fmtTime(t.created_at)}
                      </td>
                      <td className="px-3 py-1.5 whitespace-nowrap tabular-nums">
                        <span className="text-zinc-400">{fmtDuration(t.created_at, t.updated_at)}</span>
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
            <div className="rounded-lg border border-zinc-800 bg-zinc-900 overflow-hidden">
              <div className="grid grid-cols-2 divide-x divide-zinc-800">
                {[
                  { icon: Hash,  label: "Tokens in",       value: fmtTokens(cost.total_tokens_in) },
                  { icon: Hash,  label: "Tokens out",      value: fmtTokens(cost.total_tokens_out) },
                  { icon: Cpu,   label: "Provider calls",  value: String(cost.provider_calls) },
                  { icon: Clock, label: "Total cost (USD)", value: fmtMicros(cost.total_cost_micros) },
                ].map(({ icon: Icon, label, value }) => (
                  <div key={label} className="flex items-center gap-3 px-4 py-3 border-b border-zinc-800 last:border-0">
                    <Icon size={13} className="text-zinc-600 shrink-0" />
                    <div>
                      <p className="text-[11px] text-zinc-500">{label}</p>
                      <p className="text-[13px] font-mono text-zinc-200">{value}</p>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </Section>
        )}

        {/* Events timeline */}
        <Section title={`Event Timeline${events ? ` (${events.length})` : ""}`}>
          {eventsLoading ? (
            <div className="flex items-center gap-2 text-zinc-600 text-[13px] py-4">
              <Loader2 size={14} className="animate-spin" /> Loading events…
            </div>
          ) : !events || events.length === 0 ? (
            <p className="text-[13px] text-zinc-600 italic py-4">No events recorded.</p>
          ) : (
            <div className="relative">
              {/* Vertical line */}
              <div className="absolute left-[19px] top-2 bottom-2 w-px bg-zinc-800" />

              <div className="space-y-0">
                {events.map((ev, i) => (
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
                    <div className="flex-1 flex items-center gap-2.5 min-w-0 pb-1 border-b border-zinc-800/40 group-last:border-0">
                      <span className={clsx(
                        "shrink-0 rounded px-1.5 py-0.5 text-[10px] font-mono font-medium ring-1 whitespace-nowrap",
                        eventTypeColor(ev.event_type),
                      )}>
                        {ev.event_type.replace(/_/g, "\u202F")}
                      </span>
                      <span className="flex-1" />
                      <span className="shrink-0 text-[11px] text-zinc-600 font-mono tabular-nums">
                        {fmtTimeShort(ev.stored_at)}
                      </span>
                      <span className="shrink-0 text-[10px] text-zinc-700 font-mono tabular-nums">
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
