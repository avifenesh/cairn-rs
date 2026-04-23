import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Loader2, Inbox, Check, X } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { StatCard } from "../components/StatCard";
import { HelpTooltip } from "../components/HelpTooltip";
import { CopyButton } from "../components/CopyButton";
import { clsx } from "clsx";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import { table as tablePreset } from "../lib/design-system";
import type { ApprovalRecord, ApprovalDecision } from "../lib/types";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";

// ── Helpers ────────────────────────────────────────────────────────────────────

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit",
  });

const fmtRelative = (ms: number): string => {
  const d = Date.now() - ms;
  if (d < 60_000)      return "just now";
  if (d < 3_600_000)   return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)  return `${Math.floor(d / 3_600_000)}h ago`;
  if (d < 604_800_000) return `${Math.floor(d / 86_400_000)}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
};

// ── Decision badge ─────────────────────────────────────────────────────────────

function DecisionBadge({ decision }: { decision: ApprovalDecision | null }) {
  if (!decision) return (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-amber-400 bg-amber-950/50 border border-amber-800/40 rounded px-2 py-0.5">
      Pending
    </span>
  );
  return decision === "approved" ? (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-400 bg-emerald-950/50 border border-emerald-800/40 rounded px-2 py-0.5">
      <Check size={10} strokeWidth={2.5} /> Approved
    </span>
  ) : (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-red-400 bg-red-950/50 border border-red-800/40 rounded px-2 py-0.5">
      <X size={10} strokeWidth={2.5} /> Rejected
    </span>
  );
}

// ── Row actions ────────────────────────────────────────────────────────────────

function RowActions({ approval }: { approval: ApprovalRecord }) {
  const qc    = useQueryClient();
  const toast = useToast();

  const resolve = useMutation({
    mutationFn: (decision: ApprovalDecision) =>
      defaultApi.resolveApproval(approval.approval_id, decision),
    onSuccess: (_, decision) => {
      toast.success(decision === "approved" ? "Approval granted." : "Approval denied.");
      void qc.invalidateQueries({ queryKey: ["approvals"] });
    },
    onError: () => toast.error("Failed to resolve — try again."),
  });

  if (approval.decision !== null) return null;

  return (
    <div className="flex items-center gap-1.5 opacity-0 group-hover:opacity-100 transition-opacity">
      <HelpTooltip
        text="Approve: allows the run or task to continue past this gate."
        placement="top"
      />
      <button
        data-testid="approve-btn"
        onClick={e => {
          e.stopPropagation();
          if (!window.confirm(
            `Approve this request?\n\nApproving will allow the run or task to continue past this gate.\n\nApproval: ${approval.approval_id.slice(0, 16)}…`
          )) return;
          resolve.mutate("approved");
        }}
        disabled={resolve.isPending}
        className="px-2 py-0.5 rounded text-[11px] font-medium bg-emerald-900/50 text-emerald-300
                   hover:bg-emerald-900 border border-emerald-800/50 transition-colors disabled:opacity-40"
      >
        {resolve.isPending ? <Loader2 size={10} className="animate-spin inline" /> : "Approve"}
      </button>
      <button
        data-testid="reject-btn"
        onClick={e => {
          e.stopPropagation();
          if (!window.confirm(
            `Reject this request?\n\nRejecting will block the run and record a rejection decision. This cannot be undone.\n\nApproval: ${approval.approval_id.slice(0, 16)}…`
          )) return;
          resolve.mutate("rejected");
        }}
        disabled={resolve.isPending}
        className="px-2 py-0.5 rounded text-[11px] font-medium bg-red-900/40 text-red-400
                   hover:bg-red-900/70 border border-red-800/40 transition-colors disabled:opacity-40"
      >
        Reject
      </button>
    </div>
  );
}

// ── Table ─────────────────────────────────────────────────────────────────────

const TH = ({ ch, right, hide }: { ch: React.ReactNode; right?: boolean; hide?: string }) => (
  <th className={clsx(right ? tablePreset.thRight : tablePreset.th, hide)}>{ch}</th>
);

function ApprovalsTable({ approvals }: { approvals: ApprovalRecord[] }) {
  if (approvals.length === 0) return (
    <div className="flex flex-col items-center justify-center py-16 gap-2 text-center px-6">
      <Inbox size={26} className="text-gray-300 dark:text-zinc-600" />
      <p className="text-[13px] text-gray-400 dark:text-zinc-600 font-medium">Inbox clear</p>
      <p className="text-[11px] text-gray-300 dark:text-zinc-600 max-w-xs">
        No approvals match this filter. Approvals appear here when a run hits a human-in-the-loop gate
        that requires operator sign-off.
      </p>
    </div>
  );

  return (
    <table className="min-w-full text-[13px]">
      <thead className="bg-gray-50 dark:bg-zinc-900 sticky top-0 z-10">
        <tr>
          <TH ch="ID" />
          <TH ch="Run"          hide="hidden sm:table-cell" />
          <TH ch="Task"         hide="hidden md:table-cell" />
          <TH ch="Policy"       hide="hidden sm:table-cell" />
          <TH ch="Status" />
          <TH ch="Requested At" hide="hidden md:table-cell" />
          <TH ch="" right />
        </tr>
      </thead>
      <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
        {approvals.map((a, i) => (
          <tr key={a.approval_id}
            className={clsx(
              "group transition-colors",
              i % 2 === 0 ? tablePreset.rowEven : tablePreset.rowOdd,
              "hover:bg-gray-100/70 dark:hover:bg-gray-100/70 dark:bg-zinc-800/70",
            )}>
            <td className="px-3 py-1.5 font-mono text-gray-700 dark:text-zinc-300 whitespace-nowrap" title={a.approval_id}>
              <span className="flex items-center gap-1 group/id">{shortId(a.approval_id)}<CopyButton text={a.approval_id} label="Copy approval ID" size={10} className="opacity-0 group-hover/id:opacity-100" /></span>
            </td>
            <td className="px-3 py-1.5 font-mono text-gray-400 dark:text-zinc-500 whitespace-nowrap text-[12px] hidden sm:table-cell">
              {a.run_id ? <span title={a.run_id}>{shortId(a.run_id)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span>}
            </td>
            <td className="px-3 py-1.5 font-mono text-gray-400 dark:text-zinc-500 whitespace-nowrap text-[12px] hidden md:table-cell">
              {a.task_id ? <span title={a.task_id}>{shortId(a.task_id)}</span> : <span className="text-gray-300 dark:text-zinc-600">—</span>}
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap hidden sm:table-cell">
              <span className={clsx(
                "text-[11px] font-medium rounded px-1.5 py-0.5",
                a.requirement === "required"
                  ? "text-violet-300 bg-violet-950/40 border border-violet-800/40"
                  : "text-gray-500 dark:text-zinc-400 bg-gray-100/60 dark:bg-zinc-800/60 border border-gray-200 dark:border-zinc-700",
              )}>
                {a.requirement}
              </span>
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap">
              <DecisionBadge decision={a.decision} />
            </td>
            <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-500 whitespace-nowrap tabular-nums hidden md:table-cell" title={fmtTime(a.created_at)}>
              {fmtRelative(a.created_at)}
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap">
              <RowActions approval={a} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Filter tabs ────────────────────────────────────────────────────────────────

type Tab = "all" | "pending" | "resolved";

const TABS: { id: Tab; label: string }[] = [
  { id: "all",      label: "All"      },
  { id: "pending",  label: "Pending"  },
  { id: "resolved", label: "Resolved" },
];

// ── Page ──────────────────────────────────────────────────────────────────────

export function ApprovalsPage() {
  const { ms: refreshMs, setOption: setRefreshOption, interval: refreshInterval } = useAutoRefresh("approvals", "15s");

  const [tab, setTab] = useState<Tab>("all");

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["approvals"],
    queryFn: () => defaultApi.getAllApprovals(),
    refetchInterval: refreshMs,
  });

  const all      = data ?? [];
  const pending  = all.filter(a => a.decision === null);
  const resolved = all.filter(a => a.decision !== null);
  const displayed =
    tab === "pending"  ? pending  :
    tab === "resolved" ? resolved : all;

  // 24-hour window stats (resolved within the last 24h).
  // Key on updated_at — for resolved approvals this is the resolution timestamp.
  // Using created_at here double-counted pending requests and missed approvals
  // that were requested earlier but resolved recently (issue #176).
  const since24h   = Date.now() - 86_400_000;
  const approved24 = resolved.filter(a => a.decision === "approved" && a.updated_at >= since24h).length;
  const rejected24 = resolved.filter(a => a.decision === "rejected" && a.updated_at >= since24h).length;

  if (isError) return <ErrorFallback error={error} resource="approvals" onRetry={() => void refetch()} />;

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Stat strip */}
      {!isLoading && (
        <div className="grid grid-cols-3 gap-x-6 gap-y-3 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 shrink-0">
          <StatCard compact
            label="Pending"
            value={pending.length}
            description={pending.length > 0 ? "requires action" : "inbox clear"}
            variant={pending.length > 0 ? "warning" : "success"}
          />
          <StatCard compact label="Approved (24h)" value={approved24} variant="success" />
          <StatCard compact label="Rejected (24h)" value={rejected24} variant="danger" />
        </div>
      )}

      {/* Toolbar */}
      <div className="flex items-center gap-4 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        {/* Filter tabs */}
        <div className="flex items-center gap-0">
          {TABS.map(t => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className={clsx(
                "px-3 h-10 text-[12px] font-medium transition-colors border-b-2",
                tab === t.id
                  ? "text-gray-900 dark:text-zinc-100 border-indigo-500"
                  : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
              )}
            >
              {t.label}
              <span className={clsx(
                "ml-1.5 text-[10px] px-1 rounded",
                tab === t.id ? "text-gray-500 dark:text-zinc-400" : "text-gray-400 dark:text-zinc-600",
              )}>
                {t.id === "all" ? all.length : t.id === "pending" ? pending.length : resolved.length}
              </span>
            </button>
          ))}
        </div>

                {/* Auto-refresh control */}
        <div className="flex items-center gap-1">
          <div className="relative">
            <select
              value={refreshInterval.option}
              onChange={e => setRefreshOption(e.target.value as import('../hooks/useAutoRefresh').RefreshOption)}
              className="appearance-none rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] font-mono pl-5 pr-2 h-7 text-gray-500 dark:text-zinc-400 focus:outline-none focus:border-indigo-500 transition-colors hover:border-zinc-600"
              title="Auto-refresh interval"
            >
              {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
            </select>
            {isFetching
              ? <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none"><RefreshCw size={9} className="animate-spin text-indigo-400" /></span>
              : <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none text-gray-400 dark:text-zinc-600"><RefreshCw size={9} /></span>
            }
          </div>
          <button onClick={() => refetch()} disabled={isFetching}
            className="flex items-center gap-1 h-7 px-2 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600 disabled:opacity-40 transition-colors"
            title="Refresh now"
          >
            <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
            <span className="hidden sm:inline">Refresh</span>
          </button>
        </div>
      </div>

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading ? (
          <div className="divide-y divide-gray-200 dark:divide-zinc-800/40">
            {Array.from({ length: 5 }).map((_, i) => (
              <div key={i} className="flex items-center gap-4 px-4 h-9 animate-pulse">
                <div className="h-2.5 w-24 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="h-2.5 w-20 rounded bg-gray-100 dark:bg-zinc-800 hidden sm:block" />
                <div className="h-2.5 w-20 rounded bg-gray-100 dark:bg-zinc-800 hidden md:block" />
                <div className="h-4 w-14 rounded bg-gray-100 dark:bg-zinc-800 hidden sm:block" />
                <div className="h-5 w-16 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="ml-auto h-2.5 w-16 rounded bg-gray-100 dark:bg-zinc-800 hidden md:block" />
              </div>
            ))}
          </div>
        ) : <ApprovalsTable approvals={displayed} />}
      </div>
    </div>
  );
}

export default ApprovalsPage;
