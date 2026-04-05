import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { clsx } from "clsx";
import { MiniChart } from "../components/MiniChart";
import { BarChart } from "../components/BarChart";
import { defaultApi } from "../lib/api";

// ── Formatting ────────────────────────────────────────────────────────────────

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

// ── Stat card — left-border accent, no icon ───────────────────────────────────

interface StatCardProps {
  label: string;
  value: string;
  sub?: string;
  accent?: "default" | "emerald" | "blue" | "violet";
  loading?: boolean;
}

const ACCENT_BORDER: Record<NonNullable<StatCardProps["accent"]>, string> = {
  default: "border-l-zinc-700",
  emerald: "border-l-emerald-500",
  blue:    "border-l-blue-500",
  violet:  "border-l-violet-500",
};
const ACCENT_VALUE: Record<NonNullable<StatCardProps["accent"]>, string> = {
  default: "text-zinc-100",
  emerald: "text-emerald-400",
  blue:    "text-blue-400",
  violet:  "text-violet-400",
};

function StatCard({ label, value, sub, accent = "default", loading }: StatCardProps) {
  if (loading) {
    return (
      <div className={clsx("bg-zinc-900 border border-zinc-800 border-l-2 rounded-lg p-4 animate-pulse", ACCENT_BORDER[accent])}>
        <div className="h-2.5 w-20 rounded bg-zinc-800 mb-3" />
        <div className="h-6 w-16 rounded bg-zinc-800" />
      </div>
    );
  }
  return (
    <div className={clsx("bg-zinc-900 border border-zinc-800 border-l-2 rounded-lg p-4", ACCENT_BORDER[accent])}>
      <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2 truncate">{label}</p>
      <p className={clsx("text-2xl font-semibold tabular-nums", ACCENT_VALUE[accent])}>{value}</p>
      {sub && <p className="mt-1 text-[11px] text-zinc-600 truncate">{sub}</p>}
    </div>
  );
}

// ── Token split bar ───────────────────────────────────────────────────────────

function TokenBar({ input, output }: { input: number; output: number }) {
  const total = input + output;
  if (total === 0) return null;
  const inPct  = (input  / total) * 100;
  const outPct = (output / total) * 100;
  return (
    <div className="space-y-1.5">
      <div className="flex h-1.5 w-full overflow-hidden rounded-full bg-zinc-800">
        <div className="bg-blue-500"   style={{ width: `${inPct}%` }} />
        <div className="bg-violet-500" style={{ width: `${outPct}%` }} />
      </div>
      <div className="flex justify-between text-[11px] text-zinc-500">
        <span className="flex items-center gap-1">
          <span className="inline-block h-1.5 w-1.5 rounded-full bg-blue-500" />
          {formatTokens(input)} in ({inPct.toFixed(0)}%)
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-1.5 w-1.5 rounded-full bg-violet-500" />
          {formatTokens(output)} out ({outPct.toFixed(0)}%)
        </span>
      </div>
    </div>
  );
}

// ── Breakdown table row ───────────────────────────────────────────────────────

interface BreakdownRowProps { label: string; value: string; mono?: boolean; even?: boolean }
function BreakdownRow({ label, value, mono, even }: BreakdownRowProps) {
  return (
    <div className={clsx("flex items-center justify-between px-4 h-9", even ? "bg-zinc-900" : "bg-zinc-900/50")}>
      <span className="text-xs text-zinc-500">{label}</span>
      <span className={clsx("text-xs text-zinc-200", mono && "font-mono tabular-nums")}>{value}</span>
    </div>
  );
}

// ── Model color palette (matches DashboardPage) ───────────────────────────────

const MODEL_COLORS: Record<string, string> = {
  qwen3:    "#8b5cf6",
  llama:    "#f59e0b",
  mistral:  "#10b981",
  nomic:    "#06b6d4",
  gemma:    "#f97316",
  claude:   "#a855f7",
  gpt:      "#22d3ee",
  deepseek: "#e879f9",
};

function modelColor(id: string): string {
  const lower = id.toLowerCase();
  for (const [key, col] of Object.entries(MODEL_COLORS)) {
    if (lower.includes(key)) return col;
  }
  return "#6366f1";
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function CostsPage() {
  const { data: costs, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["costs"],
    queryFn: () => defaultApi.getCosts(),
    refetchInterval: 30_000,
  });

  // Traces — source of per-model cost breakdown and daily-trend sparkline.
  const { data: tracesData } = useQuery({
    queryKey: ["traces-costs"],
    queryFn:  () => defaultApi.getTraces(500),
    refetchInterval: 60_000,
    staleTime: 30_000,
    retry: false,
  });

  // Per-model cost aggregation for the bar chart.
  const modelCostItems = useMemo(() => {
    const traces = tracesData?.traces ?? [];
    const byModel: Record<string, number> = {};
    for (const t of traces) {
      byModel[t.model_id] = (byModel[t.model_id] ?? 0) + t.cost_micros;
    }
    return Object.entries(byModel)
      .map(([label, value]) => ({ label, value, color: modelColor(label) }))
      .sort((a, b) => b.value - a.value);
  }, [tracesData]);

  // Daily spend sparkline: bucket the last 7 days of traces by day.
  const dailySpend = useMemo((): number[] => {
    const traces = tracesData?.traces ?? [];
    const days = 7;
    const now  = Date.now();
    return Array.from({ length: days }, (_, i) => {
      const dayStart = now - (days - i) * 86_400_000;
      const dayEnd   = dayStart + 86_400_000;
      return traces
        .filter((t) => t.created_at_ms >= dayStart && t.created_at_ms < dayEnd)
        .reduce((sum, t) => sum + t.cost_micros, 0);
    });
  }, [tracesData]);

  if (isError) return <ErrorFallback error={error} resource="costs" onRetry={() => void refetch()} />;

  const totalTokens = (costs?.total_tokens_in ?? 0) + (costs?.total_tokens_out ?? 0);
  const avgPerCall  = (costs?.total_provider_calls ?? 0) > 0
    ? (costs?.total_cost_micros ?? 0) / costs!.total_provider_calls
    : 0;

  return (
    <div className="p-6 space-y-5">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Cost Tracking</p>
        <button onClick={() => refetch()} className="flex items-center gap-1.5 rounded-md bg-zinc-900 border border-zinc-800 px-2.5 py-1.5 text-[11px] text-zinc-500 hover:bg-white/5 transition-colors">
          <RefreshCw size={11} /> Refresh
        </button>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <StatCard label="Total Spend"     value={formatMicros(costs?.total_cost_micros ?? 0)} sub={`${(costs?.total_cost_micros ?? 0).toLocaleString()} µUSD`} accent="emerald" loading={isLoading} />
        <StatCard label="Provider Calls"  value={(costs?.total_provider_calls ?? 0).toLocaleString()} sub={`avg ${formatMicros(avgPerCall)} / call`} accent="blue" loading={isLoading} />
        <StatCard label="Input Tokens"    value={formatTokens(costs?.total_tokens_in ?? 0)}  sub="sent to providers"     accent="blue"    loading={isLoading} />
        <StatCard label="Output Tokens"   value={formatTokens(costs?.total_tokens_out ?? 0)} sub="received"              accent="violet"  loading={isLoading} />
      </div>

      {/* Token split + breakdown */}
      {!isLoading && (
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          {/* Token distribution */}
          <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
            <div className="px-4 h-9 flex items-center border-b border-zinc-800">
              <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Token Distribution</p>
            </div>
            <div className="p-4">
              {totalTokens > 0
                ? <TokenBar input={costs?.total_tokens_in ?? 0} output={costs?.total_tokens_out ?? 0} />
                : <p className="text-[11px] text-zinc-600 text-center py-3">No token data yet</p>
              }
            </div>
          </div>

          {/* Cost breakdown table */}
          <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
            <div className="px-4 h-9 flex items-center border-b border-zinc-800">
              <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Breakdown</p>
            </div>
            <div className="divide-y divide-zinc-800/50">
              <BreakdownRow label="Total spend (USD)"   value={formatMicros(costs?.total_cost_micros ?? 0)} mono  even />
              <BreakdownRow label="Total spend (µUSD)"  value={(costs?.total_cost_micros ?? 0).toLocaleString()} mono />
              <BreakdownRow label="Avg cost / call"     value={formatMicros(avgPerCall)} mono even />
              <BreakdownRow label="Total provider calls" value={(costs?.total_provider_calls ?? 0).toLocaleString()} mono />
              <BreakdownRow label="Total tokens"         value={formatTokens(totalTokens)} mono even />
              <BreakdownRow label="Input tokens"         value={formatTokens(costs?.total_tokens_in ?? 0)} mono />
              <BreakdownRow label="Output tokens"        value={formatTokens(costs?.total_tokens_out ?? 0)} mono even />
            </div>
          </div>
        </div>
      )}

      {/* Charts row — model breakdown bar chart + daily spend sparkline */}
      {!isLoading && (
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          {/* Cost by model */}
          <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
            <div className="px-4 h-9 flex items-center border-b border-zinc-800">
              <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Cost by Model</p>
            </div>
            <div className="p-4">
              {modelCostItems.length === 0 ? (
                <p className="text-[11px] text-zinc-600 text-center py-3 italic">
                  No trace data yet — costs appear after LLM calls.
                </p>
              ) : (
                <BarChart
                  items={modelCostItems}
                  formatValue={(v) => formatMicros(v)}
                  maxItems={6}
                  barHeight={7}
                  rowGap={9}
                />
              )}
            </div>
          </div>

          {/* Daily spend trend sparkline */}
          <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
            <div className="px-4 h-9 flex items-center justify-between border-b border-zinc-800">
              <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Daily Spend (7d)</p>
              <span className="text-[10px] text-zinc-700 font-mono">µUSD</span>
            </div>
            <div className="p-4 flex items-end gap-4">
              <div className="flex-1">
                <MiniChart
                  data={dailySpend}
                  height={52}
                  color="#10b981"
                  baseline
                  className="w-full"
                />
                {/* Day labels */}
                <div className="flex justify-between mt-1.5">
                  {["6d", "5d", "4d", "3d", "2d", "1d", "Now"].map((label) => (
                    <span key={label} className="text-[9px] text-zinc-700 font-mono">{label}</span>
                  ))}
                </div>
              </div>
              <div className="shrink-0 text-right">
                <p className="text-[11px] text-zinc-500">Today</p>
                <p className="text-[15px] font-semibold tabular-nums text-emerald-400">
                  {formatMicros(dailySpend[dailySpend.length - 1] ?? 0)}
                </p>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Zero state */}
      {!isLoading && (costs?.total_provider_calls ?? 0) === 0 && (
        <div className="flex flex-col items-center justify-center py-12 rounded-lg border border-zinc-800 text-center gap-2">
          <p className="text-sm text-zinc-500">No spend recorded yet</p>
          <p className="text-[11px] text-zinc-600">Costs appear once LLM calls are routed through a provider binding</p>
        </div>
      )}
    </div>
  );
}

export default CostsPage;
