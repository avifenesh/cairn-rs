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
import { sectionLabel } from "../lib/design-system";
import { ApiError } from "../lib/api";
import { EmptyScopeHint } from "../components/EmptyScopeHint";

// ── Helpers ──────────────────────────────────────────────────────────────────

const authHeaders = () => ({ Authorization: `Bearer ${localStorage.getItem("cairn_token") || ""}` });

/** Fetch wrapper that throws on non-2xx. The raw-`fetch` call sites below
 *  previously ignored HTTP errors entirely — 4xx/5xx returned undefined and
 *  the success toast fired as if the server had honored the request.
 *
 *  Throws `ApiError` (not a generic `Error`) so the global 401 interceptor
 *  in `main.tsx` recognizes auth-expired failures and routes the operator
 *  back to the LoginPage. Mirrors the behavior of `apiFetch` in `api.ts`
 *  so 401 handling stays consistent across the UI. */
async function assertOk(path: string, init: RequestInit): Promise<Response> {
  const res = await fetch(path, init);
  if (!res.ok) {
    let code = 'unknown_error';
    let message = `HTTP ${res.status}`;
    try {
      const body = await res.json();
      code = body?.code ?? code;
      message = body?.message ?? message;
    } catch {
      // Non-JSON body — fall back to the default message above.
    }
    throw new ApiError(res.status, code, message);
  }
  return res;
}

/** Normalize list responses to `T[]`. Mirrors the `getList` helper used by
 *  `createApiClient` in `api.ts` so DecisionsPage handles both the bare
 *  array and `{items, hasMore}` envelope shapes consistently. */
function unwrapList<T>(data: unknown): T[] {
  if (Array.isArray(data)) return data as T[];
  if (data && typeof data === 'object' && 'items' in data && Array.isArray((data as { items: unknown }).items)) {
    return (data as { items: T[] }).items;
  }
  return [];
}

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
  // Optional — not every decision row carries a `kind` field (cache-hit
  // rows emit `decision_key` instead). Treat as optional so the render
  // path doesn't turn `undefined` into the literal string "undefined".
  kind?: Record<string, unknown> | string;
  outcome?: { outcome?: string; deny_reason?: string };
  created_at: number;
}

interface CacheScope {
  level: string;
  tenant_id: string;
  workspace_id: string;
  project_id: string;
}

interface CacheEntry {
  decision_id: string;
  // Backend emits a nested `{outcome, deny_reason?}` struct (same shape as
  // `Decision.outcome`), not a bare string — rendering the object directly
  // crashed OutcomePill with "Objects are not valid as a React child".
  // Fields marked optional because backend occasionally emits empty/missing
  // values; render paths must handle absent gracefully (em-dash fallback)
  // instead of leaking "undefined" / blank cells onto the operator UI.
  outcome?: { outcome?: string; deny_reason?: string };
  kind_tag?: string;
  // Backend emits a `ProjectScope` object, not a string — rendering the
  // object directly would crash React with "Objects are not valid as a
  // React child".
  scope: CacheScope;
  expires_at: number;
  hit_count: number;
}

function scopeLabel(s: CacheScope): string {
  return `${s.tenant_id}/${s.workspace_id}/${s.project_id}`;
}

// ── Outcome pill (matches session/run state pill pattern) ────────────────────

const OUTCOME_PILL: Record<string, string> = {
  allowed: "bg-emerald-500/10 text-emerald-400 border-emerald-500/20",
  denied:  "bg-red-500/10 text-red-400 border-red-500/20",
  pending: "bg-gray-500/10 text-gray-400 border-gray-500/20 dark:bg-zinc-500/10 dark:text-zinc-400 dark:border-zinc-500/20",
};
const OUTCOME_DOT: Record<string, string> = {
  allowed: "bg-emerald-400",
  denied:  "bg-red-400",
  pending: "bg-gray-400 dark:bg-zinc-400",
};

/** Renders the decision outcome as a colored pill. Handles missing/empty
 *  outcome gracefully — the backend occasionally emits rows with an empty
 *  `outcome` string (e.g. cache-hit rows mid-materialization) and the
 *  earlier impl leaked the literal "undefined" onto the operator UI. */
function OutcomePill({ outcome }: { outcome?: string | null }) {
  const key = outcome && outcome.length > 0 ? outcome : "pending";
  const label = outcome && outcome.length > 0 ? outcome : "pending";
  return (
    <span className={clsx(
      "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border whitespace-nowrap",
      OUTCOME_PILL[key] ?? OUTCOME_PILL.pending,
    )}>
      <span className={clsx("w-1 h-1 rounded-full shrink-0", OUTCOME_DOT[key] ?? OUTCOME_DOT.pending)} />
      {label}
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
      const res = await assertOk("/v1/decisions", { headers: authHeaders() });
      return unwrapList<Decision>(await res.json());
    },
    refetchInterval: 30_000,
  });

  const cacheQ = useQuery<CacheEntry[]>({
    queryKey: ["decisions-cache"],
    queryFn: async () => {
      const res = await assertOk("/v1/decisions/cache", { headers: authHeaders() });
      return unwrapList<CacheEntry>(await res.json());
    },
    refetchInterval: 30_000,
  });

  const invalidateMut = useMutation({
    mutationFn: (id: string) => assertOk(`/v1/decisions/${id}/invalidate`, {
      method: "POST", headers: { ...authHeaders(), "Content-Type": "application/json" },
      body: JSON.stringify({ reason: "operator-invalidated" }),
    }),
    onSuccess: () => { toast.success("Cache entry invalidated."); void qc.invalidateQueries({ queryKey: ["decisions-cache"] }); },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : "Failed to invalidate cache entry."),
  });

  // Bulk-invalidate the decision cache. The real endpoint is the bulk form
  // of `/v1/decisions/invalidate` (see crates/cairn-app/src/router.rs:1271).
  // The previous URL (`/v1/decisions/cache/invalidate-all`) does not exist;
  // the raw `fetch` also ignored the 404 and fired the success toast.
  const bulkMut = useMutation({
    mutationFn: () => assertOk("/v1/decisions/invalidate", {
      method: "POST", headers: { ...authHeaders(), "Content-Type": "application/json" },
      body: JSON.stringify({ reason: "operator-bulk-clear" }),
    }),
    onSuccess: () => { toast.success("All cache entries invalidated."); void qc.invalidateQueries({ queryKey: ["decisions-cache"] }); },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : "Failed to bulk-invalidate cache."),
  });

  const decisions = decisionsQ.data ?? [];
  const cacheEntries = cacheQ.data ?? [];
  const allowed = decisions.filter(d => d.outcome?.outcome === "allowed").length;
  const denied = decisions.filter(d => d.outcome?.outcome === "denied").length;

  if (decisionsQ.isError) return <ErrorFallback error={decisionsQ.error} resource="decisions" onRetry={() => void decisionsQ.refetch()} />;

  return (
    <div className="p-6 space-y-5">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <p className={clsx(sectionLabel, "mb-0")}>Decisions</p>
            <HelpTooltip text="Every tool call, trigger, and plugin action is policy-checked before running. Audit the allow/deny decisions below." placement="right" />
          </div>
          <p className="text-[11px] text-gray-500 dark:text-zinc-400">Audit tool, trigger, and plugin policy decisions across your workspace.</p>
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
            { key: "kind", header: "Kind", render: r => {
              // Some rows (e.g. cache-hit rows) omit `kind` entirely —
              // render an em-dash instead of the literal "undefined".
              const k = typeof r.kind === "object" && r.kind !== null
                ? String((r.kind as Record<string, unknown>).type ?? "unknown")
                : typeof r.kind === "string" && r.kind.length > 0
                  ? r.kind
                  : "—";
              return <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded font-mono">{k}</code>;
            } },
            { key: "outcome", header: "Outcome", render: r => <OutcomePill outcome={r.outcome?.outcome} />, sortValue: r => r.outcome?.outcome ?? "" },
            { key: "created", header: "Created", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at },
          ]}
          filterFn={(r, q) => r.decision_id.includes(q) || (r.kind !== undefined && String(r.kind).includes(q)) || (r.outcome?.outcome ?? "").includes(q)}
          csvRow={r => [r.decision_id, r.kind !== undefined ? String(r.kind) : "", r.outcome?.outcome ?? "", r.created_at]}
          csvHeaders={["ID", "Kind", "Outcome", "Created"]}
          filename="decisions"
          emptyText="No decisions yet. Decisions appear here when a tool call, trigger, or plugin action is policy-checked."
        />
      ) : (
        <DataTable<CacheEntry>
          data={cacheEntries}
          getRowId={e => e.decision_id}
          rowClassName="group"
          columns={[
            { key: "kind", header: "Kind", render: r => <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded font-mono">{r.kind_tag && r.kind_tag.length > 0 ? r.kind_tag : "—"}</code> },
            { key: "outcome", header: "Outcome", render: r => <OutcomePill outcome={r.outcome?.outcome} />, sortValue: r => r.outcome?.outcome ?? "" },
            { key: "scope", header: "Scope", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500">{scopeLabel(r.scope)}</span>, sortValue: r => scopeLabel(r.scope) },
            { key: "hits", header: "Hits", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{r.hit_count}</span>, sortValue: r => r.hit_count },
            { key: "expires", header: "Expires", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.expires_at)}</span>, sortValue: r => r.expires_at },
            { key: "actions", header: "", render: r => (
              <button onClick={() => invalidateMut.mutate(r.decision_id)} title="Invalidate" className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-red-400 opacity-0 group-hover:opacity-100 transition-all"><Trash2 size={12} /></button>
            )},
          ]}
          filterFn={(r, q) => (r.kind_tag ?? "").includes(q) || (r.outcome?.outcome ?? "").includes(q) || scopeLabel(r.scope).includes(q)}
          csvRow={r => [r.decision_id, r.kind_tag ?? "", r.outcome?.outcome ?? "", scopeLabel(r.scope), r.hit_count, r.expires_at]}
          csvHeaders={["Decision ID", "Kind", "Outcome", "Scope", "Hits", "Expires"]}
          filename="decision-cache"
          emptyText="No cached decisions yet. Cached rules skip repeat operator prompts for the same action."
        />
      )}
      <EmptyScopeHint empty={decisions.length === 0 && cacheEntries.length === 0} />
    </div>
  );
}
