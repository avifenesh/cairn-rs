import { useQuery } from "@tanstack/react-query";
import {
  DollarSign,
  ArrowDownLeft,
  ArrowUpRight,
  Zap,
  ServerCrash,
  RefreshCw,
  TrendingUp,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";

// ── Formatting helpers ────────────────────────────────────────────────────────

/** Convert USD micros (µUSD) to a human-readable dollar string. */
function formatMicros(micros: number): string {
  if (micros === 0) return "$0.00";
  const usd = micros / 1_000_000;
  if (usd < 0.01) return `$${usd.toFixed(6)}`;
  if (usd < 1)    return `$${usd.toFixed(4)}`;
  return `$${usd.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

function formatTokens(n: number): string {
  if (n === 0) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000)     return `${(n / 1_000).toFixed(1)}K`;
  return n.toLocaleString();
}

// ── Metric tile ───────────────────────────────────────────────────────────────

interface MetricTileProps {
  label: string;
  value: string;
  sub?: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  accent: string;   // Tailwind text colour class for icon
  iconBg: string;   // Tailwind bg class for icon wrapper
  loading?: boolean;
}

function MetricTile({ label, value, sub, icon: Icon, accent, iconBg, loading }: MetricTileProps) {
  if (loading) {
    return (
      <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5 animate-pulse">
        <div className="flex justify-between items-start">
          <div className="space-y-2">
            <div className="h-3 w-24 rounded bg-zinc-700" />
            <div className="h-8 w-20 rounded bg-zinc-700" />
            <div className="h-2.5 w-32 rounded bg-zinc-800" />
          </div>
          <div className="h-10 w-10 rounded-lg bg-zinc-800" />
        </div>
      </div>
    );
  }

  return (
    <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5 hover:ring-zinc-700 hover:shadow-lg hover:shadow-black/30 transition-all">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm font-medium text-zinc-400">{label}</p>
          <p className="mt-1 text-3xl font-semibold tracking-tight text-zinc-50">{value}</p>
          {sub && <p className="mt-1 text-xs text-zinc-500">{sub}</p>}
        </div>
        <div className={clsx("flex h-10 w-10 shrink-0 items-center justify-center rounded-lg", iconBg)}>
          <Icon size={18} className={accent} />
        </div>
      </div>
    </div>
  );
}

// ── Token bar ─────────────────────────────────────────────────────────────────

function TokenBar({ input, output }: { input: number; output: number }) {
  const total = input + output;
  if (total === 0) return null;
  const inputPct  = (input  / total) * 100;
  const outputPct = (output / total) * 100;

  return (
    <div className="space-y-2">
      <div className="flex justify-between text-xs text-zinc-500">
        <span>Input</span>
        <span>Output</span>
      </div>
      <div className="flex h-2 w-full overflow-hidden rounded-full bg-zinc-800">
        <div
          className="bg-blue-500 transition-all"
          style={{ width: `${inputPct}%` }}
        />
        <div
          className="bg-violet-500 transition-all"
          style={{ width: `${outputPct}%` }}
        />
      </div>
      <div className="flex justify-between text-xs text-zinc-400">
        <span className="flex items-center gap-1">
          <span className="inline-block h-2 w-2 rounded-full bg-blue-500" />
          {formatTokens(input)} in ({inputPct.toFixed(0)}%)
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-2 w-2 rounded-full bg-violet-500" />
          {formatTokens(output)} out ({outputPct.toFixed(0)}%)
        </span>
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function CostsPage() {
  const { data: costs, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["costs"],
    queryFn: () => defaultApi.getCosts(),
    refetchInterval: 30_000,
  });

  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
        <ServerCrash size={40} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load cost data</p>
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

  const totalTokens = (costs?.total_tokens_in ?? 0) + (costs?.total_tokens_out ?? 0);
  const avgCostPerCall =
    (costs?.total_provider_calls ?? 0) > 0
      ? (costs?.total_cost_micros ?? 0) / (costs?.total_provider_calls ?? 1)
      : 0;

  return (
    <div className="min-h-screen bg-zinc-950 p-6 space-y-6">
      {/* ── Header ─────────────────────────────────────────────────────── */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-zinc-100 flex items-center gap-2">
            <DollarSign size={20} className="text-emerald-400" />
            Cost Tracking
          </h1>
          <p className="text-xs text-zinc-500 mt-0.5">
            Aggregate LLM spend — USD micros precision
          </p>
        </div>
        <button
          onClick={() => refetch()}
          className="flex items-center gap-1.5 rounded-lg bg-zinc-800 px-2.5 py-1.5 text-xs text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200 transition-colors"
        >
          <RefreshCw size={12} /> Refresh
        </button>
      </div>

      {/* ── Hero card — total spend ──────────────────────────────────────── */}
      {isLoading ? (
        <div className="rounded-2xl bg-zinc-900 ring-1 ring-zinc-800 p-7 animate-pulse">
          <div className="h-5 w-32 rounded bg-zinc-700 mb-3" />
          <div className="h-12 w-48 rounded bg-zinc-700" />
        </div>
      ) : (
        <div className="rounded-2xl bg-zinc-900 ring-1 ring-emerald-900/40 p-7">
          <p className="text-sm font-medium text-zinc-400 flex items-center gap-2">
            <TrendingUp size={14} className="text-emerald-500" />
            Total Spend
          </p>
          <p className="mt-2 text-5xl font-bold tracking-tight text-emerald-400">
            {formatMicros(costs?.total_cost_micros ?? 0)}
          </p>
          <p className="mt-1 text-xs text-zinc-600">
            {(costs?.total_cost_micros ?? 0).toLocaleString()} µUSD · avg{" "}
            {formatMicros(avgCostPerCall)} / call
          </p>
        </div>
      )}

      {/* ── Metric grid ─────────────────────────────────────────────────── */}
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <MetricTile
          label="Provider Calls"
          value={(costs?.total_provider_calls ?? 0).toLocaleString()}
          sub="total LLM dispatches"
          icon={Zap}
          accent="text-blue-400"
          iconBg="bg-blue-950"
          loading={isLoading}
        />
        <MetricTile
          label="Input Tokens"
          value={formatTokens(costs?.total_tokens_in ?? 0)}
          sub="tokens sent to providers"
          icon={ArrowUpRight}
          accent="text-blue-300"
          iconBg="bg-blue-950/60"
          loading={isLoading}
        />
        <MetricTile
          label="Output Tokens"
          value={formatTokens(costs?.total_tokens_out ?? 0)}
          sub="tokens received from providers"
          icon={ArrowDownLeft}
          accent="text-violet-300"
          iconBg="bg-violet-950"
          loading={isLoading}
        />
        <MetricTile
          label="Total Tokens"
          value={formatTokens(totalTokens)}
          sub="input + output combined"
          icon={TrendingUp}
          accent="text-zinc-300"
          iconBg="bg-zinc-800"
          loading={isLoading}
        />
      </div>

      {/* ── Token split bar ─────────────────────────────────────────────── */}
      {!isLoading && totalTokens > 0 && (
        <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5">
          <h2 className="text-sm font-semibold text-zinc-300 mb-4">Token Distribution</h2>
          <TokenBar
            input={costs?.total_tokens_in ?? 0}
            output={costs?.total_tokens_out ?? 0}
          />
        </div>
      )}

      {/* ── Zero-state ──────────────────────────────────────────────────── */}
      {!isLoading && (costs?.total_provider_calls ?? 0) === 0 && (
        <div className="flex flex-col items-center justify-center py-16 text-center rounded-xl bg-zinc-900/50 ring-1 ring-zinc-800/50">
          <DollarSign size={36} className="text-zinc-700 mb-3" />
          <p className="text-zinc-400 font-medium">No spend recorded</p>
          <p className="text-sm text-zinc-600 mt-1">
            Costs will appear here once LLM calls are routed through a provider binding
          </p>
        </div>
      )}
    </div>
  );
}

export default CostsPage;
