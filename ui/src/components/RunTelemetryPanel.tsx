/**
 * RunTelemetryPanel — F29 CE.
 *
 * Consumes `GET /v1/runs/:id/telemetry` (shipped in PR CD) and renders
 * per-run observability for operators:
 *
 *   - Provider calls table (model, tokens, cost, latency, status, error)
 *   - Tool invocations table (tool, status, duration)
 *   - Totals card ($cost, tokens, calls, errors, wall-ms)
 *   - "Stuck since X" badge when telemetry.stuck = true
 *
 * Polling: every 5s while the run is pending/running, never after
 * terminal. The `refetchInterval` reads the latest `data.state` so
 * TanStack stops polling the moment the run resolves without
 * requiring a component re-mount.
 *
 * Extracted as a separate component per CE scope — RunDetailPage is
 * already ~1376 LOC and this panel would push it over 1700.
 */

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, ChevronDown, ChevronRight } from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { table as tablePreset } from "../lib/design-system";
import {
  formatUsd,
  formatTokens,
  formatDurationMs,
  formatRelativePast,
  truncate,
} from "../lib/formatters";
import type {
  RunTelemetry,
  RunTelemetryProviderCall,
  RunTelemetryToolInvocation,
} from "../lib/types";

// ── Constants ──────────────────────────────────────────────────────────────

/** Max chars of error_message rendered inline in the Provider Calls table.
 *  Full text is available on row-expand. */
const ERROR_MESSAGE_TRUNCATE_CHARS = 240;

/** F55: JSON-stringify tool args for the expand-on-click row without
 *  crashing on cyclic or non-serializable values. Falls back to
 *  `String(value)` when `JSON.stringify` throws. */
function safeStringifyJson(value: unknown): string {
  try {
    if (typeof value === "string") return value;
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

const LIVE_STATES = new Set(["pending", "running"]);

// ── Status pill ────────────────────────────────────────────────────────────

const STATUS_STYLES: Record<string, string> = {
  succeeded:  "bg-emerald-500/10 text-emerald-400",
  failed:     "bg-red-500/10     text-red-400",
  cancelled:  "bg-zinc-500/10    text-zinc-400",
  canceled:   "bg-zinc-500/10    text-zinc-400",
  requested:  "bg-sky-500/10     text-sky-400",
  started:    "bg-indigo-500/10  text-indigo-400",
  completed:  "bg-emerald-500/10 text-emerald-400",
};

function StatusPill({ status }: { status: string }) {
  const cls = STATUS_STYLES[status] ?? "bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400";
  return (
    <span className={clsx("inline-flex items-center rounded px-1.5 py-0.5 text-[11px] font-medium whitespace-nowrap", cls)}>
      {status}
    </span>
  );
}

// ── Totals card ────────────────────────────────────────────────────────────

function TotalsCard({ telemetry }: { telemetry: RunTelemetry }) {
  const t = telemetry.totals;
  return (
    <div
      className="grid grid-cols-2 sm:grid-cols-4 gap-x-6 gap-y-3 py-3 px-4 rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60"
      data-testid="run-telemetry-totals"
    >
      <Stat label="Cost"           value={formatUsd(t.cost_micros)} />
      <Stat label="Tokens"         value={`${formatTokens(t.input_tokens)} / ${formatTokens(t.output_tokens)}`} description="in / out" />
      <Stat label="Provider calls" value={String(t.provider_calls)} description={`${t.errors} error${t.errors === 1 ? "" : "s"}`} />
      <Stat label="Tool calls"     value={String(t.tool_calls)} description={`wall ${formatDurationMs(t.wall_ms)}`} />
    </div>
  );
}

function Stat({ label, value, description }: { label: string; value: string; description?: string }) {
  return (
    <div>
      <p className="text-[10px] uppercase tracking-wider text-gray-400 dark:text-zinc-600">{label}</p>
      <p className="text-[14px] font-medium text-gray-900 dark:text-zinc-100 tabular-nums">{value}</p>
      {description && <p className="text-[10px] text-gray-400 dark:text-zinc-600">{description}</p>}
    </div>
  );
}

// ── Provider Calls table ───────────────────────────────────────────────────

function ProviderCallRow({ call }: { call: RunTelemetryProviderCall }) {
  const [expanded, setExpanded] = useState(false);
  const hasDetails = call.error_message != null && call.error_message.length > 0;

  return (
    <>
      <tr
        className={clsx(
          "transition-colors",
          hasDetails ? "cursor-pointer hover:bg-gray-100/60 dark:hover:bg-zinc-800/60" : "",
        )}
        onClick={() => hasDetails && setExpanded(v => !v)}
      >
        <td className="px-3 py-1.5 font-mono text-[12px] text-gray-700 dark:text-zinc-300 whitespace-nowrap">
          {hasDetails && (
            <span className="inline-block mr-1 text-gray-400 dark:text-zinc-500">
              {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            </span>
          )}
          {call.model}
        </td>
        <td className="px-3 py-1.5 whitespace-nowrap"><StatusPill status={call.status} /></td>
        <td className="px-3 py-1.5 text-right tabular-nums text-[12px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">
          {formatTokens(call.input_tokens)} / {formatTokens(call.output_tokens)}
        </td>
        <td className="px-3 py-1.5 text-right tabular-nums text-[12px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">
          {formatUsd(call.cost_micros)}
        </td>
        <td className="px-3 py-1.5 text-right tabular-nums text-[12px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">
          {formatDurationMs(call.latency_ms)}
        </td>
        <td className="px-3 py-1.5 text-[12px] text-red-500 dark:text-red-400 max-w-[18rem] truncate" title={call.error_message ?? undefined}>
          {call.error_class && (
            <span className="font-mono text-red-400 dark:text-red-300">{call.error_class}</span>
          )}
          {call.error_class && call.error_message && <span className="mx-1 text-red-300 dark:text-red-600">·</span>}
          {call.error_message && <span>{truncate(call.error_message, ERROR_MESSAGE_TRUNCATE_CHARS)}</span>}
        </td>
      </tr>
      {expanded && hasDetails && (
        <tr>
          <td colSpan={6} className="px-4 py-3 bg-gray-50 dark:bg-zinc-900/60 border-y border-gray-200 dark:border-zinc-800">
            <p className="text-[10px] uppercase tracking-wider text-gray-400 dark:text-zinc-600 mb-1">Full error</p>
            <pre className="text-[11px] text-red-500 dark:text-red-300 whitespace-pre-wrap font-mono leading-snug">
              {call.error_message}
            </pre>
            <p className="mt-2 text-[10px] text-gray-400 dark:text-zinc-600 font-mono">
              call {call.provider_call_id}
            </p>
          </td>
        </tr>
      )}
    </>
  );
}

function ProviderCallsTable({ calls }: { calls: RunTelemetryProviderCall[] }) {
  if (calls.length === 0) {
    return (
      <p className="text-[12px] text-gray-400 dark:text-zinc-600 py-3 text-center border border-dashed border-gray-200 dark:border-zinc-800 rounded">
        No provider calls yet.
      </p>
    );
  }
  return (
    <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-x-auto">
      <table className="min-w-full text-[13px]" data-testid="run-telemetry-provider-calls">
        <thead className="bg-gray-50 dark:bg-zinc-900">
          <tr>
            <th className={tablePreset.th}>Model</th>
            <th className={tablePreset.th}>Status</th>
            <th className={tablePreset.thRight}>Tokens in/out</th>
            <th className={tablePreset.thRight}>Cost</th>
            <th className={tablePreset.thRight}>Latency</th>
            <th className={tablePreset.th}>Error</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
          {calls.map(c => <ProviderCallRow key={c.provider_call_id} call={c} />)}
        </tbody>
      </table>
    </div>
  );
}

// ── Tool Invocations table ─────────────────────────────────────────────────

function ToolInvocationRow({ inv }: { inv: RunTelemetryToolInvocation }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <>
      <tr
        className="cursor-pointer transition-colors hover:bg-gray-100/60 dark:hover:bg-zinc-800/60"
        onClick={() => setExpanded(v => !v)}
      >
        <td className="px-3 py-1.5 font-mono text-[12px] text-gray-700 dark:text-zinc-300 whitespace-nowrap">
          <span className="inline-block mr-1 text-gray-400 dark:text-zinc-500">
            {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          </span>
          {inv.tool_name}
        </td>
        <td className="px-3 py-1.5 whitespace-nowrap"><StatusPill status={inv.status} /></td>
        <td className="px-3 py-1.5 text-right tabular-nums text-[12px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">
          {formatDurationMs(inv.duration_ms)}
        </td>
      </tr>
      {expanded && (
        <tr>
          <td
            colSpan={3}
            className="px-4 py-3 bg-gray-50 dark:bg-zinc-900/60 border-y border-gray-200 dark:border-zinc-800 text-[11px] text-gray-500 dark:text-zinc-400 font-mono"
            data-testid={`tool-invocation-expanded-${inv.invocation_id}`}
          >
            <div>invocation {inv.invocation_id}</div>
            {inv.started_at_ms > 0 && (
              <div className="mt-1">started {new Date(inv.started_at_ms).toISOString()}</div>
            )}
            {inv.finished_at_ms > 0 && (
              <div>finished {new Date(inv.finished_at_ms).toISOString()}</div>
            )}
            {/* F55 / F48: show the args + captured output inline so operators
                can see what cairn ran and what it got back without leaving
                the run-detail page. */}
            {inv.args !== undefined && inv.args !== null && (
              <div className="mt-3">
                <div className="text-[10px] uppercase tracking-wide text-gray-400 dark:text-zinc-500 mb-1">
                  args
                </div>
                <pre
                  className="whitespace-pre-wrap break-all text-[11px] text-gray-700 dark:text-zinc-300 bg-white/60 dark:bg-zinc-950/60 border border-gray-200 dark:border-zinc-800 rounded px-2 py-1"
                  data-testid={`tool-invocation-args-${inv.invocation_id}`}
                >
                  {safeStringifyJson(inv.args)}
                </pre>
              </div>
            )}
            {inv.output_preview && (
              <div className="mt-3">
                <div className="text-[10px] uppercase tracking-wide text-gray-400 dark:text-zinc-500 mb-1">
                  output
                </div>
                <pre
                  className="whitespace-pre-wrap break-all text-[11px] text-gray-700 dark:text-zinc-300 bg-white/60 dark:bg-zinc-950/60 border border-gray-200 dark:border-zinc-800 rounded px-2 py-1"
                  data-testid={`tool-invocation-output-${inv.invocation_id}`}
                >
                  {inv.output_preview}
                </pre>
                {inv.output_truncated && (
                  <div className="mt-1 text-[10px] text-amber-500 dark:text-amber-400">
                    (output truncated at backend cap)
                  </div>
                )}
              </div>
            )}
            {inv.error_message && (
              <div className="mt-3">
                <div className="text-[10px] uppercase tracking-wide text-red-400 mb-1">error</div>
                <pre className="whitespace-pre-wrap break-all text-[11px] text-red-400 bg-white/60 dark:bg-zinc-950/60 border border-red-500/30 rounded px-2 py-1">
                  {inv.error_message}
                </pre>
              </div>
            )}
          </td>
        </tr>
      )}
    </>
  );
}

function ToolInvocationsTable({ invocations }: { invocations: RunTelemetryToolInvocation[] }) {
  if (invocations.length === 0) {
    return (
      <p className="text-[12px] text-gray-400 dark:text-zinc-600 py-3 text-center border border-dashed border-gray-200 dark:border-zinc-800 rounded">
        No tool invocations yet.
      </p>
    );
  }
  return (
    <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-x-auto">
      <table className="min-w-full text-[13px]" data-testid="run-telemetry-tool-invocations">
        <thead className="bg-gray-50 dark:bg-zinc-900">
          <tr>
            <th className={tablePreset.th}>Tool</th>
            <th className={tablePreset.th}>Status</th>
            <th className={tablePreset.thRight}>Duration</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
          {invocations.map(i => <ToolInvocationRow key={i.invocation_id} inv={i} />)}
        </tbody>
      </table>
    </div>
  );
}

// ── Stuck banner ───────────────────────────────────────────────────────────

function StuckBanner({ telemetry }: { telemetry: RunTelemetry }) {
  if (!telemetry.stuck) return null;
  const since = telemetry.stuck_since_ms
    ? formatRelativePast(telemetry.stuck_since_ms)
    : "recently";
  return (
    <div
      data-testid="run-telemetry-stuck-banner"
      className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[12px] text-amber-500 dark:text-amber-400"
    >
      <AlertTriangle size={13} className="mt-0.5 shrink-0" />
      <span>
        Run appears stuck — last activity {since}. Check the tasks table or
        trigger a recovery via the operator actions above.
      </span>
    </div>
  );
}

// ── Panel ──────────────────────────────────────────────────────────────────

interface RunTelemetryPanelProps {
  runId: string;
}

export function RunTelemetryPanel({ runId }: RunTelemetryPanelProps) {
  const { data, isLoading, isError, error } = useQuery<RunTelemetry>({
    queryKey: ["run-telemetry", runId],
    queryFn: () => defaultApi.getRunTelemetry(runId),
    // Poll every 5s while the run is live. `refetchInterval` receives the
    // latest query result so we stop polling as soon as the backend flips
    // the run into a terminal state.
    refetchInterval: query => {
      const d = query.state.data as RunTelemetry | undefined;
      return d && LIVE_STATES.has(d.state) ? 5_000 : false;
    },
    staleTime: 2_000,
    // 404 is possible when the run is very fresh (projection lag) —
    // fall through to the loading skeleton and let poll pick it up.
    retry: (failureCount, err) =>
      !(err as { status?: number })?.status || (err as { status?: number }).status !== 404
        ? failureCount < 3
        : false,
  });

  if (isLoading) {
    return (
      <div data-testid="run-telemetry-panel">
        <p className="text-[11px] font-semibold text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">
          Telemetry
        </p>
        <p className="text-[12px] text-gray-400 dark:text-zinc-600 py-3">Loading telemetry…</p>
      </div>
    );
  }

  if (isError) {
    return (
      <div data-testid="run-telemetry-panel">
        <p className="text-[11px] font-semibold text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">
          Telemetry
        </p>
        <p className="text-[12px] text-red-500 dark:text-red-400 py-3">
          Failed to load telemetry: {error instanceof Error ? error.message : "unknown"}
        </p>
      </div>
    );
  }

  if (!data) return null;

  return (
    <div className="space-y-4" data-testid="run-telemetry-panel">
      <p className="text-[11px] font-semibold text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
        Telemetry
      </p>
      <StuckBanner telemetry={data} />
      <TotalsCard telemetry={data} />

      <div>
        <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 mb-1.5">Provider calls</p>
        <ProviderCallsTable calls={data.provider_calls} />
      </div>

      <div>
        <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 mb-1.5">Tool invocations</p>
        <ToolInvocationsTable invocations={data.tool_invocations} />
      </div>
    </div>
  );
}

export default RunTelemetryPanel;
