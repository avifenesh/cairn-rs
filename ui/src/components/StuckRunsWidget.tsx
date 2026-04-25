/**
 * StuckRunsWidget — F29 CE.
 *
 * Dashboard widget summarising `GET /v1/runs/stalled`. Renders a
 * compact count + top-5 list. Hidden entirely when the count is zero
 * so a healthy deployment doesn't show dead chrome.
 *
 * Row click → navigate to RunDetailPage (via the global hash router
 * used across the app — same pattern as RunsPage / DashboardPage).
 * Refetches every 30s.
 */

import { useQuery } from "@tanstack/react-query";
import { AlertTriangle } from "lucide-react";
import { defaultApi } from "../lib/api";
import { Card } from "./Card";
import { formatRelativePast } from "../lib/formatters";
import type { StuckRunReport } from "../lib/types";

const TOP_N = 5;

function shortId(id: string): string {
  return id.length > 16 ? `${id.slice(0, 8)}…${id.slice(-4)}` : id;
}

/** Best-effort "stuck since" timestamp. Prefer the last observed event
 *  (more accurate than the run's own duration) but fall back to the
 *  run creation when last_event_ms is missing. */
function stuckSince(report: StuckRunReport): number {
  if (report.last_event_ms > 0) return report.last_event_ms;
  return Date.now() - report.duration_ms;
}

export function StuckRunsWidget() {
  const { data, isLoading } = useQuery<StuckRunReport[]>({
    queryKey: ["stalled-runs"],
    queryFn: () => defaultApi.getStalledRuns(),
    refetchInterval: 30_000,
    staleTime: 10_000,
    retry: false,
  });

  const reports = data ?? [];
  if (isLoading) return null;
  // Hide the entire widget when there's nothing to warn about —
  // operator dashboards should stay quiet during healthy operation.
  if (reports.length === 0) return null;

  const top = reports.slice(0, TOP_N);
  const remainder = reports.length - top.length;

  return (
    <div data-testid="stuck-runs-widget">
    <Card variant="shell" className="flex flex-col">
      <div className="flex items-center gap-2 px-4 h-10 border-b border-gray-200 dark:border-zinc-800">
        <AlertTriangle size={13} className="text-amber-400" />
        <p className="text-[11px] font-semibold text-amber-500 dark:text-amber-400 uppercase tracking-wider">
          Stalled runs
        </p>
        <span
          className="ml-auto text-[11px] font-medium text-amber-500 dark:text-amber-400"
          data-testid="stuck-runs-count"
        >
          {reports.length}
        </span>
      </div>
      <ul className="divide-y divide-gray-200 dark:divide-zinc-800/60">
        {top.map(report => (
          <li key={report.run_id}>
            <button
              onClick={() => {
                window.location.hash = `run/${encodeURIComponent(report.run_id)}`;
              }}
              className="w-full flex items-center gap-3 px-4 py-2 text-left hover:bg-gray-100/60 dark:hover:bg-zinc-800/60 transition-colors"
              data-testid="stuck-runs-row"
            >
              <span className="font-mono text-[12px] text-gray-700 dark:text-zinc-200 truncate" title={report.run_id}>
                {shortId(report.run_id)}
              </span>
              <span className="text-[11px] text-gray-400 dark:text-zinc-500 capitalize">
                {report.state.replace(/_/g, " ")}
              </span>
              <span className="ml-auto text-[11px] text-amber-500 dark:text-amber-400 tabular-nums whitespace-nowrap">
                {formatRelativePast(stuckSince(report))}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {remainder > 0 && (
        <div className="px-4 py-2 border-t border-gray-200 dark:border-zinc-800 text-[11px] text-gray-400 dark:text-zinc-500">
          + {remainder} more —{" "}
          <button
            onClick={() => { window.location.hash = "runs?stalled=1"; }}
            className="text-indigo-500 dark:text-indigo-400 hover:underline"
          >
            view all
          </button>
        </div>
      )}
    </Card>
    </div>
  );
}

export default StuckRunsWidget;
