import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Trash2, XCircle } from "lucide-react";
import { DataTable } from "../components/DataTable";
import { StatCard as SummaryStatCard } from "../components/StatCard";
import { CopyButton } from "../components/CopyButton";
import { HelpTooltip } from "../components/HelpTooltip";
import { ErrorFallback } from "../components/ErrorFallback";
import { useToast } from "../components/Toast";
import { clsx } from "clsx";

// ── Helpers ──────────────────────────────────────────────────────────────────

const authHeaders = () => ({ Authorization: `Bearer ${localStorage.getItem("cairn_token") || ""}` });

function fmtRelative(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000) return "just now";
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)}h ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function mono(s: string, max = 18): string {
  return s.length > max ? `${s.slice(0, max - 3)}…` : s;
}

// ── Types ────────────────────────────────────────────────────────────────────

interface Decision {
  decision_id: string;
  kind: Record<string, unknown> | string;
  outcome: { outcome: string; deny_reason?: string };
  created_at: number;
}

interface CacheEntry {
  key: string;
  decision_id: string;
  outcome: string;
  kind_tag: string;
  scope: string;
  expires_at: number;
  hit_count: number;
}

// ── Outcome pill (matches session/run state pill pattern) ────────────────────

const OUTCOME_PILL: Record<string, string> = {
  allowed: "bg-emerald-500/10 text-emerald-400 border-emerald-500/20",
  denied:  "bg-red-500/10 text-red-400 border-red-500/20",
};
const OUTCOME_DOT: Record<string, string> = {
  allowed: "bg-emerald-400",
  denied:  "bg-red-400",
};

function OutcomePill({ outcome }: { outcome: string }) {
  return (
    <span className={clsx(
      "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border whitespace-nowrap",
      OUTCOME_PILL[outcome] ?? OUTCOME_PILL.denied,
    )}>
      <span className={clsx("w-1 h-1 rounded-full shrink-0", OUTCOME_DOT[outcome] ?? OUTCOME_DOT.denied)} />
      {outcome}
    </span>
  );
}

// ── Main page ────────────────────────────────────────────────────────────────

export function DecisionsPage() {
  const [tab, setTab] = useState<"recent" | "cache">("recent");
  const qc = useQueryClient();
  const toast = useToast();

  const decisionsQ = useQuery<Decision[]>({
    queryKey: ["decisions"],
    queryFn: async () => {
      const res = await fetch("/v1/decisions", { headers: authHeaders() });
      const data = await res.json();
      return Array.isArray(data) ? data : (data.items ?? []);
    },
    refetchInterval: 30_000,
  });

  const cacheQ = useQuery<CacheEntry[]>({
    queryKey: ["decisions-cache"],
    queryFn: async () => {
      const res = await fetch("/v1/decisions/cache", { headers: authHeaders() });
      const data = await res.json();
      return Array.isArray(data) ? data : (data.items ?? []);
    },
    refetchInterval: 30_000,
  });

  const invalidateMut = useMutation({
    mutationFn: (id: string) => fetch(`/v1/decisions/${id}/invalidate`, {
      method: "POST", headers: { ...authHeaders(), "Content-Type": "application/json" },
      body: JSON.stringify({ reason: "operator-invalidated" }),
    }),
    onSuccess: () => { toast.success("Cache entry invalidated."); void qc.invalidateQueries({ queryKey: ["decisions-cache"] }); },
  });

  const bulkMut = useMutation({
    mutationFn: () => fetch("/v1/decisions/cache/invalidate-all", {
      method: "POST", headers: { ...authHeaders(), "Content-Type": "application/json" },
      body: JSON.stringify({ reason: "operator-bulk-clear" }),
    }),
    onSuccess: () => { toast.success("All cache entries invalidated."); void qc.invalidateQueries({ queryKey: ["decisions-cache"] }); },
  });

  const decisions = decisionsQ.data ?? [];
  const cacheEntries = cacheQ.data ?? [];
  const allowed = decisions.filter(d => d.outcome.outcome === "allowed").length;
  const denied = decisions.filter(d => d.outcome.outcome === "denied").length;

  if (decisionsQ.isError) return <ErrorFallback error={decisionsQ.error} resource="decisions" onRetry={() => void decisionsQ.refetch()} />;

  return (
    <div className="p-6 space-y-5">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Decisions</p>
            <HelpTooltip text="Unified decision layer (RFC 019). Every tool invocation, trigger fire, and plugin enablement goes through policy evaluation before proceeding." placement="right" />
          </div>
          <p className="text-[11px] text-gray-500 dark:text-zinc-400">Implements RFC 019 — Unified Decision Layer.</p>
        </div>
        <div className="flex items-center gap-2">
          {tab === "cache" && cacheEntries.length > 0 && (
            <button onClick={() => { if (window.confirm("Invalidate ALL cached decisions? Operators will be re-prompted.")) bulkMut.mutate(); }}
              className="flex items-center gap-1.5 rounded-md bg-red-500/10 border border-red-500/20 px-2.5 py-1.5 text-[11px] text-red-400 hover:bg-red-500/20 transition-colors">
              <XCircle size={11} /> Bulk Invalidate
            </button>
          )}
          <button onClick={() => decisionsQ.refetch()} className="flex items-center gap-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:bg-white/5 transition-colors">
            <RefreshCw size={11} className={clsx(decisionsQ.isFetching && "animate-spin")} /> Refresh
          </button>
        </div>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <SummaryStatCard label="Total Decisions" value={decisions.length} />
        <SummaryStatCard label="Allowed" value={allowed} variant="success" />
        <SummaryStatCard label="Denied" value={denied} variant="danger" />
        <SummaryStatCard label="Cached Rules" value={cacheEntries.length} variant="warning" />
      </div>

      {/* Tab bar */}
      <div className="flex items-center gap-1 border-b border-gray-200 dark:border-zinc-800">
        {([["recent", `Recent (${decisions.length})`], ["cache", `Cache (${cacheEntries.length})`]] as const).map(([t, label]) => (
          <button key={t} onClick={() => setTab(t as "recent" | "cache")}
            className={clsx(
              "px-3 py-1.5 text-[12px] font-medium border-b-2 -mb-px transition-colors",
              tab === t ? "text-gray-900 dark:text-zinc-100 border-indigo-500" : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
            )}>
            {label}
          </button>
        ))}
      </div>

      {/* Content */}
      {tab === "recent" ? (
        <DataTable<Decision>
          data={decisions}
          getRowId={d => d.decision_id}
          columns={[
            { key: "id", header: "ID", render: r => <span className="flex items-center gap-1 font-mono text-[11px] text-gray-500 dark:text-zinc-400 whitespace-nowrap group/id">{mono(r.decision_id)}<CopyButton text={r.decision_id} label="Copy decision ID" size={10} className="opacity-0 group-hover/id:opacity-100" /></span>, sortValue: r => r.decision_id },
            { key: "kind", header: "Kind", render: r => { const k = typeof r.kind === "object" && r.kind !== null ? String((r.kind as Record<string, unknown>).type ?? "unknown") : String(r.kind); return <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded font-mono">{k}</code>; } },
            { key: "outcome", header: "Outcome", render: r => <OutcomePill outcome={r.outcome.outcome} />, sortValue: r => r.outcome.outcome },
            { key: "created", header: "Created", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at },
          ]}
          filterFn={(r, q) => r.decision_id.includes(q) || String(r.kind).includes(q) || r.outcome.outcome.includes(q)}
          csvRow={r => [r.decision_id, String(r.kind), r.outcome.outcome, r.created_at]}
          csvHeaders={["ID", "Kind", "Outcome", "Created"]}
          filename="decisions"
          emptyText="No decisions yet — decisions appear when the unified decision layer evaluates tool invocations, trigger fires, or plugin enablements."
        />
      ) : (
        <DataTable<CacheEntry>
          data={cacheEntries}
          getRowId={e => e.key}
          columns={[
            { key: "kind", header: "Kind", render: r => <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded font-mono">{r.kind_tag}</code> },
            { key: "outcome", header: "Outcome", render: r => <OutcomePill outcome={r.outcome} />, sortValue: r => r.outcome },
            { key: "scope", header: "Scope", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500">{r.scope}</span> },
            { key: "hits", header: "Hits", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{r.hit_count}</span>, sortValue: r => r.hit_count },
            { key: "expires", header: "Expires", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.expires_at)}</span>, sortValue: r => r.expires_at },
            { key: "actions", header: "", render: r => (
              <button onClick={() => invalidateMut.mutate(r.decision_id)} title="Invalidate" className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-red-400 opacity-0 group-hover:opacity-100 transition-all"><Trash2 size={12} /></button>
            )},
          ]}
          filterFn={(r, q) => r.kind_tag.includes(q) || r.outcome.includes(q) || r.scope.includes(q)}
          csvRow={r => [r.key, r.kind_tag, r.outcome, r.scope, r.hit_count, r.expires_at]}
          csvHeaders={["Key", "Kind", "Outcome", "Scope", "Hits", "Expires"]}
          filename="decision-cache"
          emptyText="Cache is empty — no learned rules yet. Cached decisions reduce operator re-prompts."
        />
      )}
    </div>
  );
}
