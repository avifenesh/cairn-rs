import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2, Inbox, Scale, Trash2, XCircle } from "lucide-react";
import { clsx } from "clsx";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import { useAutoRefresh } from "../hooks/useAutoRefresh";

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });

// ── Types ────────────────────────────────────────────────────────────────────

interface Decision {
  decision_id: string;
  kind: unknown;
  outcome: { outcome: string; deny_reason?: string };
  scope_ref: unknown;
  created_at: number;
  expires_at?: number;
  hit_count?: number;
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

// ── Stat card ────────────────────────────────────────────────────────────────

function StatCard({ label, value, accent }: { label: string; value: string | number; accent?: string }) {
  return (
    <div className={clsx("border-l-2 pl-3 py-0.5", accent ?? "border-indigo-500")}>
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[22px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
    </div>
  );
}

// ── Outcome badge ────────────────────────────────────────────────────────────

function OutcomeBadge({ outcome }: { outcome: string }) {
  if (outcome === "allowed") return (
    <span className="text-[11px] font-medium text-emerald-400 bg-emerald-950/50 border border-emerald-800/40 rounded px-2 py-0.5">
      Allowed
    </span>
  );
  return (
    <span className="text-[11px] font-medium text-red-400 bg-red-950/50 border border-red-800/40 rounded px-2 py-0.5">
      Denied
    </span>
  );
}

// ── Table header ─────────────────────────────────────────────────────────────

const TH = ({ ch, right, hide }: { ch: React.ReactNode; right?: boolean; hide?: string }) => (
  <th className={clsx(
    "px-3 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-gray-200 dark:border-zinc-800",
    right ? "text-right" : "text-left", hide,
  )}>{ch}</th>
);

// ── Decisions table ──────────────────────────────────────────────────────────

function DecisionsTable({ decisions }: { decisions: Decision[] }) {
  if (decisions.length === 0) return (
    <div className="flex flex-col items-center justify-center py-16 gap-2 text-center px-6">
      <Inbox size={26} className="text-gray-300 dark:text-zinc-700" />
      <p className="text-[13px] text-gray-400 dark:text-zinc-600 font-medium">No decisions yet</p>
      <p className="text-[11px] text-gray-300 dark:text-zinc-700 max-w-xs">
        Decisions appear when the unified decision layer (RFC 019) evaluates tool invocations, trigger fires, or plugin enablements.
      </p>
    </div>
  );

  return (
    <table className="min-w-full text-[13px]">
      <thead className="bg-gray-50 dark:bg-zinc-900 sticky top-0 z-10">
        <tr>
          <TH ch="ID" />
          <TH ch="Kind" />
          <TH ch="Outcome" />
          <TH ch="Created" hide="hidden md:table-cell" />
        </tr>
      </thead>
      <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
        {decisions.map(d => (
          <tr key={d.decision_id} className="group hover:bg-gray-50/50 dark:hover:bg-zinc-800/30 transition-colors">
            <td className="px-3 py-2.5 font-mono text-[11px] text-gray-500 dark:text-zinc-500">{shortId(d.decision_id)}</td>
            <td className="px-3 py-2.5">
              <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded">
                {typeof d.kind === "object" && d.kind !== null ? (d.kind as Record<string, unknown>).type as string ?? "unknown" : String(d.kind)}
              </code>
            </td>
            <td className="px-3 py-2.5"><OutcomeBadge outcome={d.outcome.outcome} /></td>
            <td className="px-3 py-2.5 hidden md:table-cell text-gray-400 dark:text-zinc-600 text-[11px]">{fmtTime(d.created_at)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Cache table ──────────────────────────────────────────────────────────────

function CacheTable({ entries }: { entries: CacheEntry[] }) {
  const qc = useQueryClient();
  const toast = useToast();

  const invalidateMut = useMutation({
    mutationFn: (id: string) => defaultApi.post(`/v1/decisions/${id}/invalidate`, { reason: "operator-invalidated" }),
    onSuccess: () => { toast.success("Cache entry invalidated."); void qc.invalidateQueries({ queryKey: ["decisions-cache"] }); },
    onError: () => toast.error("Failed to invalidate."),
  });

  const bulkInvalidateMut = useMutation({
    mutationFn: () => defaultApi.post("/v1/decisions/cache/invalidate-all", { reason: "operator-bulk-clear" }),
    onSuccess: () => { toast.success("All cache entries invalidated."); void qc.invalidateQueries({ queryKey: ["decisions-cache"] }); },
    onError: () => toast.error("Failed to bulk invalidate."),
  });

  if (entries.length === 0) return (
    <div className="flex flex-col items-center justify-center py-12 gap-2 text-center">
      <p className="text-[12px] text-gray-400 dark:text-zinc-600">Cache is empty — no learned rules yet.</p>
    </div>
  );

  return (
    <div>
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-zinc-800">
        <span className="text-[11px] text-gray-400 dark:text-zinc-600">{entries.length} cached rules</span>
        <button onClick={() => { if (window.confirm("Invalidate ALL cached decisions? Operators will be re-prompted.")) bulkInvalidateMut.mutate(); }}
          className="px-2 py-0.5 rounded text-[11px] font-medium bg-red-900/30 text-red-400 hover:bg-red-900/60 border border-red-800/40 flex items-center gap-1">
          <XCircle size={10} /> Bulk Invalidate
        </button>
      </div>
      <table className="min-w-full text-[13px]">
        <thead className="bg-gray-50 dark:bg-zinc-900 sticky top-0 z-10">
          <tr>
            <TH ch="Kind" />
            <TH ch="Outcome" />
            <TH ch="Scope" hide="hidden sm:table-cell" />
            <TH ch="Hits" />
            <TH ch="Expires" hide="hidden md:table-cell" />
            <TH ch="" right />
          </tr>
        </thead>
        <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
          {entries.map(e => (
            <tr key={e.key} className="group hover:bg-gray-50/50 dark:hover:bg-zinc-800/30 transition-colors">
              <td className="px-3 py-2.5">
                <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded">{e.kind_tag}</code>
              </td>
              <td className="px-3 py-2.5"><OutcomeBadge outcome={e.outcome} /></td>
              <td className="px-3 py-2.5 hidden sm:table-cell text-gray-500 dark:text-zinc-500 text-[11px]">{e.scope}</td>
              <td className="px-3 py-2.5 tabular-nums text-gray-500 dark:text-zinc-500">{e.hit_count}</td>
              <td className="px-3 py-2.5 hidden md:table-cell text-gray-400 dark:text-zinc-600 text-[11px]">{fmtTime(e.expires_at)}</td>
              <td className="px-3 py-2.5 text-right">
                <button onClick={() => invalidateMut.mutate(e.decision_id)}
                  className="opacity-0 group-hover:opacity-100 transition-opacity px-2 py-0.5 rounded text-[11px] bg-red-900/30 text-red-400 hover:bg-red-900/60 border border-red-800/40">
                  <Trash2 size={11} />
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── Main page ────────────────────────────────────────────────────────────────

export function DecisionsPage() {
  const [tab, setTab] = useState<"recent" | "cache">("recent");
  const { interval, RefreshSelect } = useAutoRefresh("decisions-refresh");

  const decisionsQ = useQuery<Decision[]>({
    queryKey: ["decisions"],
    queryFn: () => defaultApi.get("/v1/decisions"),
    refetchInterval: interval,
  });

  const cacheQ = useQuery<CacheEntry[]>({
    queryKey: ["decisions-cache"],
    queryFn: () => defaultApi.get("/v1/decisions/cache"),
    refetchInterval: interval,
  });

  const decisions = decisionsQ.data ?? [];
  const cacheEntries = cacheQ.data ?? [];
  const allowed = decisions.filter(d => d.outcome.outcome === "allowed").length;
  const denied = decisions.filter(d => d.outcome.outcome === "denied").length;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-gray-900 dark:text-zinc-100 flex items-center gap-2">
            <Scale size={18} className="text-indigo-400" /> Decisions
          </h1>
          <p className="text-[12px] text-gray-400 dark:text-zinc-600 mt-0.5">Unified decision layer (RFC 019)</p>
        </div>
        <RefreshSelect />
      </div>

      <div className="flex gap-6">
        <StatCard label="Total Decisions" value={decisions.length} />
        <StatCard label="Allowed" value={allowed} accent="border-emerald-500" />
        <StatCard label="Denied" value={denied} accent="border-red-500" />
        <StatCard label="Cached Rules" value={cacheEntries.length} accent="border-amber-500" />
      </div>

      <div className="flex gap-2 border-b border-gray-200 dark:border-zinc-800">
        <button onClick={() => setTab("recent")}
          className={clsx("px-3 py-1.5 text-[12px] font-medium border-b-2 transition-colors", tab === "recent" ? "border-indigo-500 text-indigo-400" : "border-transparent text-gray-400 hover:text-gray-300")}>
          Recent ({decisions.length})
        </button>
        <button onClick={() => setTab("cache")}
          className={clsx("px-3 py-1.5 text-[12px] font-medium border-b-2 transition-colors", tab === "cache" ? "border-indigo-500 text-indigo-400" : "border-transparent text-gray-400 hover:text-gray-300")}>
          Cache ({cacheEntries.length})
        </button>
      </div>

      <div className="bg-white dark:bg-zinc-900/50 rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden">
        {tab === "recent" ? (
          decisionsQ.isLoading ? (
            <div className="flex items-center justify-center py-12"><Loader2 className="animate-spin text-gray-400" /></div>
          ) : (
            <DecisionsTable decisions={decisions} />
          )
        ) : (
          cacheQ.isLoading ? (
            <div className="flex items-center justify-center py-12"><Loader2 className="animate-spin text-gray-400" /></div>
          ) : (
            <CacheTable entries={cacheEntries} />
          )
        )}
      </div>
    </div>
  );
}
