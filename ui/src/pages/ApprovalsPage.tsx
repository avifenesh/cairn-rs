import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Loader2, ServerCrash, Inbox, Check, X } from "lucide-react";
import { clsx } from "clsx";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import type { ApprovalRecord, ApprovalDecision } from "../lib/types";

// ── Helpers ────────────────────────────────────────────────────────────────────

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit",
  });

// ── Stat card ──────────────────────────────────────────────────────────────────

function StatCard({
  label, value, sub, accent,
}: { label: string; value: string | number; sub?: string; accent?: string }) {
  return (
    <div className={clsx("border-l-2 pl-3 py-0.5", accent ?? "border-indigo-500")}>
      <p className="text-[11px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[22px] font-semibold text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

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
      <button
        onClick={e => { e.stopPropagation(); resolve.mutate("approved"); }}
        disabled={resolve.isPending}
        className="px-2 py-0.5 rounded text-[11px] font-medium bg-emerald-900/50 text-emerald-300
                   hover:bg-emerald-900 border border-emerald-800/50 transition-colors disabled:opacity-40"
      >
        {resolve.isPending ? <Loader2 size={10} className="animate-spin inline" /> : "Approve"}
      </button>
      <button
        onClick={e => { e.stopPropagation(); resolve.mutate("rejected"); }}
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

const TH = ({ ch, right }: { ch: React.ReactNode; right?: boolean }) => (
  <th className={clsx(
    "px-3 py-2 text-[11px] font-medium text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-zinc-800",
    right ? "text-right" : "text-left",
  )}>{ch}</th>
);

function ApprovalsTable({ approvals }: { approvals: ApprovalRecord[] }) {
  if (approvals.length === 0) return (
    <div className="flex flex-col items-center justify-center py-16 gap-2 text-zinc-700">
      <Inbox size={26} />
      <p className="text-[13px]">No approvals match this filter</p>
    </div>
  );

  return (
    <table className="min-w-full text-[13px]">
      <thead className="bg-zinc-900 sticky top-0 z-10">
        <tr>
          <TH ch="ID" />
          <TH ch="Run" />
          <TH ch="Task" />
          <TH ch="Policy" />
          <TH ch="Status" />
          <TH ch="Requested At" />
          <TH ch="" right />
        </tr>
      </thead>
      <tbody className="divide-y divide-zinc-800/50">
        {approvals.map((a, i) => (
          <tr key={a.approval_id}
            className={clsx(
              "group transition-colors",
              i % 2 === 0 ? "bg-zinc-900" : "bg-[#111113]",
              "hover:bg-zinc-800/70",
            )}>
            <td className="px-3 py-1.5 font-mono text-zinc-300 whitespace-nowrap">
              {shortId(a.approval_id)}
            </td>
            <td className="px-3 py-1.5 font-mono text-zinc-500 whitespace-nowrap text-[12px]">
              {a.run_id ? shortId(a.run_id) : <span className="text-zinc-700">—</span>}
            </td>
            <td className="px-3 py-1.5 font-mono text-zinc-500 whitespace-nowrap text-[12px]">
              {a.task_id ? shortId(a.task_id) : <span className="text-zinc-700">—</span>}
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap">
              <span className={clsx(
                "text-[11px] font-medium rounded px-1.5 py-0.5",
                a.requirement === "required"
                  ? "text-violet-300 bg-violet-950/40 border border-violet-800/40"
                  : "text-zinc-400 bg-zinc-800/60 border border-zinc-700",
              )}>
                {a.requirement}
              </span>
            </td>
            <td className="px-3 py-1.5 whitespace-nowrap">
              <DecisionBadge decision={a.decision} />
            </td>
            <td className="px-3 py-1.5 text-zinc-500 whitespace-nowrap tabular-nums">
              {fmtTime(a.created_at)}
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
  const [tab, setTab] = useState<Tab>("all");

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["approvals"],
    queryFn: () => defaultApi.getPendingApprovals(),
    refetchInterval: 15_000,
  });

  const all      = data ?? [];
  const pending  = all.filter(a => a.decision === null);
  const resolved = all.filter(a => a.decision !== null);
  const displayed =
    tab === "pending"  ? pending  :
    tab === "resolved" ? resolved : all;

  // 24-hour window stats (created_at within last 24h and resolved).
  const since24h   = Date.now() - 86_400_000;
  const approved24 = resolved.filter(a => a.decision === "approved" && a.created_at >= since24h).length;
  const rejected24 = resolved.filter(a => a.decision === "rejected" && a.created_at >= since24h).length;

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-zinc-300 font-medium">Failed to load approvals</p>
      <p className="text-[12px] text-zinc-500">{error instanceof Error ? error.message : "Unknown"}</p>
      <button onClick={() => refetch()}
        className="mt-1 px-3 py-1.5 rounded bg-zinc-800 text-zinc-300 text-[12px] hover:bg-zinc-700 transition-colors">
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-zinc-900">
      {/* Stat strip */}
      {!isLoading && (
        <div className="flex items-center gap-8 px-5 py-3 border-b border-zinc-800 bg-zinc-900 shrink-0">
          <StatCard
            label="Pending"
            value={pending.length}
            sub={pending.length > 0 ? "requires action" : "inbox clear"}
            accent={pending.length > 0 ? "border-amber-500" : "border-emerald-500"}
          />
          <StatCard label="Approved (24h)" value={approved24} accent="border-emerald-500" />
          <StatCard label="Rejected (24h)" value={rejected24} accent="border-red-500" />
        </div>
      )}

      {/* Toolbar */}
      <div className="flex items-center gap-4 px-4 h-10 border-b border-zinc-800 shrink-0 bg-zinc-900">
        {/* Filter tabs */}
        <div className="flex items-center gap-0">
          {TABS.map(t => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className={clsx(
                "px-3 h-10 text-[12px] font-medium transition-colors border-b-2",
                tab === t.id
                  ? "text-zinc-100 border-indigo-500"
                  : "text-zinc-500 border-transparent hover:text-zinc-300",
              )}
            >
              {t.label}
              <span className={clsx(
                "ml-1.5 text-[10px] px-1 rounded",
                tab === t.id ? "text-zinc-400" : "text-zinc-600",
              )}>
                {t.id === "all" ? all.length : t.id === "pending" ? pending.length : resolved.length}
              </span>
            </button>
          ))}
        </div>

        <button onClick={() => refetch()} disabled={isFetching}
          className="ml-auto flex items-center gap-1 text-[12px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40 transition-colors">
          <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading
          ? <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-600">
              <Loader2 size={16} className="animate-spin" />
              <span className="text-[13px]">Loading…</span>
            </div>
          : <ApprovalsTable approvals={displayed} />
        }
      </div>
    </div>
  );
}

export default ApprovalsPage;
