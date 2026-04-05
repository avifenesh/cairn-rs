import { useQuery } from "@tanstack/react-query";
import {
  Cpu,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Clock,
  RefreshCw,
  ServerCrash,
  Plug,
  Activity,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";

// ── Provider health type (matches main.rs ProviderHealthEntry) ────────────────

interface ProviderHealthEntry {
  connection_id: string;
  status: string;
  healthy: boolean;
  last_checked_at: number; // unix ms
  consecutive_failures: number;
  error_message: string | null;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  if (ms === 0) return "Never";
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function shortId(id: string): string {
  return id.length > 28 ? `${id.slice(0, 10)}\u2026${id.slice(-8)}` : id;
}

// ── Status badge ─────────────────────────────────────────────────────────────

function StatusBadge({ healthy, status }: { healthy: boolean; status: string }) {
  return (
    <span
      className={clsx(
        "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-semibold ring-1",
        healthy
          ? "bg-emerald-950 text-emerald-400 ring-emerald-800"
          : "bg-red-950 text-red-400 ring-red-800"
      )}
    >
      {healthy ? (
        <CheckCircle2 size={11} strokeWidth={2.5} />
      ) : (
        <XCircle size={11} strokeWidth={2.5} />
      )}
      {status || (healthy ? "Healthy" : "Unhealthy")}
    </span>
  );
}

// ── Provider card ─────────────────────────────────────────────────────────────

function ProviderCard({ entry }: { entry: ProviderHealthEntry }) {
  const hasFail = entry.consecutive_failures > 0;

  return (
    <div
      className={clsx(
        "rounded-xl bg-zinc-900 ring-1 p-5 space-y-4 transition-all",
        entry.healthy
          ? "ring-zinc-800 hover:ring-zinc-700"
          : "ring-red-900/60 hover:ring-red-800/80 shadow-red-900/20 shadow-lg"
      )}
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <div
              className={clsx(
                "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg",
                entry.healthy ? "bg-emerald-950 text-emerald-400" : "bg-red-950 text-red-400"
              )}
            >
              <Cpu size={16} strokeWidth={2} />
            </div>
            <p className="font-mono text-sm font-medium text-zinc-200 truncate">
              {shortId(entry.connection_id)}
            </p>
          </div>
        </div>
        <StatusBadge healthy={entry.healthy} status={entry.status} />
      </div>

      {/* Detail rows */}
      <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
        <dt className="text-zinc-500 flex items-center gap-1">
          <Clock size={11} /> Last check
        </dt>
        <dd className="text-zinc-300 text-xs">{fmtTime(entry.last_checked_at)}</dd>

        <dt className="text-zinc-500 flex items-center gap-1">
          <Activity size={11} /> Failures
        </dt>
        <dd
          className={clsx(
            "text-xs font-semibold",
            entry.consecutive_failures === 0 ? "text-emerald-400" : "text-red-400"
          )}
        >
          {entry.consecutive_failures} consecutive
        </dd>
      </dl>

      {/* Error message banner */}
      {entry.error_message && (
        <div className="flex items-start gap-2 rounded-lg bg-red-950/40 px-3 py-2 text-xs ring-1 ring-red-900/40">
          <AlertTriangle size={12} className="mt-0.5 shrink-0 text-red-400" />
          <span className="text-red-300 break-words">{entry.error_message}</span>
        </div>
      )}

      {/* Consecutive failures warning */}
      {hasFail && !entry.error_message && (
        <div className="flex items-center gap-1.5 text-xs text-amber-400">
          <AlertTriangle size={12} />
          {entry.consecutive_failures} failure{entry.consecutive_failures > 1 ? "s" : ""} in a row
        </div>
      )}
    </div>
  );
}

// ── Summary strip ─────────────────────────────────────────────────────────────

function SummaryStrip({ entries }: { entries: ProviderHealthEntry[] }) {
  const healthy   = entries.filter((e) => e.healthy).length;
  const unhealthy = entries.length - healthy;

  return (
    <div className="flex items-center gap-6 rounded-xl bg-zinc-900 ring-1 ring-zinc-800 px-5 py-3">
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 rounded-full bg-emerald-400" />
        <span className="text-sm text-zinc-300">
          <span className="font-semibold text-emerald-400">{healthy}</span> healthy
        </span>
      </div>
      {unhealthy > 0 && (
        <div className="flex items-center gap-2">
          <span className="h-2 w-2 rounded-full bg-red-400" />
          <span className="text-sm text-zinc-300">
            <span className="font-semibold text-red-400">{unhealthy}</span> degraded
          </span>
        </div>
      )}
      <div className="ml-auto text-xs text-zinc-600">
        {entries.length} connection{entries.length !== 1 ? "s" : ""} total
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function ProvidersPage() {
  const { data, isLoading, isError, error, refetch, dataUpdatedAt } = useQuery({
    queryKey: ["providers-health"],
    queryFn: () => defaultApi.getProviderHealth() as Promise<ProviderHealthEntry[]>,
    refetchInterval: 20_000,
  });

  const entries: ProviderHealthEntry[] = Array.isArray(data) ? data : [];
  const lastUpdated = dataUpdatedAt ? new Date(dataUpdatedAt).toLocaleTimeString() : null;

  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
        <ServerCrash size={40} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load provider health</p>
        <p className="text-sm text-zinc-500">
          {error instanceof Error ? error.message : "Unknown error"}
        </p>
        <button
          onClick={() => refetch()}
          className="mt-2 flex items-center gap-1.5 rounded-lg bg-zinc-800 px-3 py-1.5 text-sm text-zinc-300 hover:bg-zinc-700 transition-colors"
        >
          <RefreshCw size={13} /> Retry
        </button>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-zinc-950 p-6 space-y-6">
      {/* ── Header ─────────────────────────────────────────────────────── */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-zinc-100 flex items-center gap-2">
            <Cpu size={20} className="text-blue-400" />
            Provider Health
          </h1>
          <p className="text-xs text-zinc-500 mt-0.5">
            Connectivity status for all registered LLM provider connections
          </p>
        </div>
        <div className="flex items-center gap-3">
          {lastUpdated && (
            <span className="text-xs text-zinc-600 flex items-center gap-1">
              <Clock size={11} /> {lastUpdated}
            </span>
          )}
          <button
            onClick={() => refetch()}
            className="flex items-center gap-1.5 rounded-lg bg-zinc-800 px-2.5 py-1.5 text-xs text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200 transition-colors"
          >
            <RefreshCw size={12} /> Refresh
          </button>
        </div>
      </div>

      {/* ── Loading ──────────────────────────────────────────────────────── */}
      {isLoading && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {[1, 2, 3].map((i) => (
            <div key={i} className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5 animate-pulse space-y-3">
              <div className="flex justify-between">
                <div className="h-8 w-8 rounded-lg bg-zinc-800" />
                <div className="h-6 w-20 rounded-full bg-zinc-800" />
              </div>
              <div className="h-3 w-40 rounded bg-zinc-800" />
              <div className="h-3 w-32 rounded bg-zinc-800" />
            </div>
          ))}
        </div>
      )}

      {/* ── Summary strip ─────────────────────────────────────────────────── */}
      {!isLoading && entries.length > 0 && <SummaryStrip entries={entries} />}

      {/* ── Provider cards ─────────────────────────────────────────────── */}
      {!isLoading && entries.length > 0 && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {entries.map((entry) => (
            <ProviderCard key={entry.connection_id} entry={entry} />
          ))}
        </div>
      )}

      {/* ── Empty state ───────────────────────────────────────────────── */}
      {!isLoading && entries.length === 0 && (
        <div className="flex flex-col items-center justify-center py-20 text-center rounded-xl bg-zinc-900/50 ring-1 ring-zinc-800/50">
          <Plug size={40} className="text-zinc-700 mb-4" />
          <p className="text-zinc-400 font-semibold">No providers configured</p>
          <p className="text-sm text-zinc-600 mt-1 max-w-sm">
            Register a provider connection and binding to start routing
            LLM calls through Cairn.
          </p>
        </div>
      )}
    </div>
  );
}

export default ProvidersPage;
