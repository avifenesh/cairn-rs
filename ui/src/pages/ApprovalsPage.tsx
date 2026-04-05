import { useState } from "react";
import { useToast } from "../components/Toast";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2,
  XCircle,
  Inbox,
  ServerCrash,
  Clock,
  AlertTriangle,
  Loader2,
  CheckCheck,
  Ban,
  RefreshCw,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import type { ApprovalRecord, ApprovalDecision } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

function shortId(id: string): string {
  return id.length > 20 ? `${id.slice(0, 8)}\u2026${id.slice(-6)}` : id;
}

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// ── Requirement badge ─────────────────────────────────────────────────────────

function RequirementBadge({ requirement }: { requirement: string }) {
  const isRequired = requirement === "required";
  return (
    <span
      className={clsx(
        "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border",
        isRequired
          ? "bg-violet-500/10 text-violet-400 border-violet-500/20"
          : "bg-zinc-800 text-zinc-500 border-zinc-700"
      )}
    >
      {isRequired ? (
        <AlertTriangle size={10} strokeWidth={2.5} />
      ) : (
        <Clock size={10} strokeWidth={2.5} />
      )}
      {isRequired ? "Required" : "Advisory"}
    </span>
  );
}

// ── Decision badge ────────────────────────────────────────────────────────────

function DecisionBadge({ decision }: { decision: ApprovalDecision }) {
  return (
    <span
      className={clsx(
        "inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-semibold ring-1",
        decision === "approved"
          ? "bg-emerald-950 text-emerald-400 ring-emerald-800"
          : "bg-red-950 text-red-400 ring-red-800"
      )}
    >
      {decision === "approved" ? (
        <CheckCircle2 size={11} strokeWidth={2.5} />
      ) : (
        <XCircle size={11} strokeWidth={2.5} />
      )}
      {decision === "approved" ? "Approved" : "Rejected"}
    </span>
  );
}

// ── Approval card (pending) ───────────────────────────────────────────────────

interface PendingCardProps {
  approval: ApprovalRecord;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
  isPending: boolean;
}

function PendingCard({ approval, onApprove, onReject, isPending }: PendingCardProps) {
  return (
    <div className="rounded-lg bg-zinc-900 border border-zinc-800 p-4 flex flex-col gap-3 transition-colors hover:border-zinc-700">
      {/* Header row */}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-mono text-sm font-medium text-zinc-200">
              {shortId(approval.approval_id)}
            </span>
            <RequirementBadge requirement={approval.requirement} />
          </div>
          <p className="text-xs text-zinc-500 mt-1 flex items-center gap-1">
            <Clock size={11} />
            {fmtTime(approval.created_at)}
          </p>
        </div>

        {/* Pending indicator */}
        <span className="shrink-0 inline-flex items-center gap-1.5 rounded bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-400 border border-amber-500/20">
          <span className="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse" />
          Pending
        </span>
      </div>

      {/* Detail fields */}
      <dl className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs">
        {approval.run_id && (
          <>
            <dt className="text-zinc-500">Run</dt>
            <dd className="font-mono text-zinc-300 truncate">{shortId(approval.run_id)}</dd>
          </>
        )}
        {approval.task_id && (
          <>
            <dt className="text-zinc-500">Task</dt>
            <dd className="font-mono text-zinc-300 truncate">{shortId(approval.task_id)}</dd>
          </>
        )}
        <dt className="text-zinc-500">Project</dt>
        <dd className="text-zinc-400 truncate text-xs">
          {approval.project.tenant_id}/{approval.project.workspace_id}
        </dd>
      </dl>

      {/* Action buttons */}
      <div className="flex items-center gap-2 pt-1 border-t border-zinc-800">
        <button
          onClick={() => onApprove(approval.approval_id)}
          disabled={isPending}
          className={clsx(
            "flex-1 flex items-center justify-center gap-1.5 rounded-md px-3 h-8 text-xs font-medium",
            "bg-emerald-500/10 text-emerald-400 border border-emerald-500/20",
            "hover:bg-emerald-500/20 transition-colors",
            "disabled:opacity-40 disabled:cursor-not-allowed"
          )}
        >
          {isPending ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <CheckCheck size={14} strokeWidth={2.5} />
          )}
          Approve
        </button>
        <button
          onClick={() => onReject(approval.approval_id)}
          disabled={isPending}
          className={clsx(
            "flex-1 flex items-center justify-center gap-1.5 rounded-md px-3 h-8 text-xs font-medium",
            "bg-red-500/10 text-red-400 border border-red-500/20",
            "hover:bg-red-500/20 transition-colors",
            "disabled:opacity-40 disabled:cursor-not-allowed"
          )}
        >
          {isPending ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <Ban size={14} strokeWidth={2.5} />
          )}
          Reject
        </button>
      </div>
    </div>
  );
}

// ── Resolved card ─────────────────────────────────────────────────────────────

function ResolvedCard({ approval }: { approval: ApprovalRecord }) {
  return (
    <div className="rounded-lg bg-zinc-950 border border-zinc-800/60 px-4 h-10 flex items-center justify-between gap-4 opacity-60">
      <div className="min-w-0 flex items-center gap-3">
        <span className="font-mono text-sm text-zinc-400 truncate">
          {shortId(approval.approval_id)}
        </span>
        {approval.run_id && (
          <span className="text-xs text-zinc-600 font-mono hidden sm:block">
            run: {shortId(approval.run_id)}
          </span>
        )}
      </div>
      <div className="shrink-0 flex items-center gap-2">
        <span className="text-xs text-zinc-600">{fmtTime(approval.created_at)}</span>
        {approval.decision && <DecisionBadge decision={approval.decision} />}
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function ApprovalsPage() {
  const queryClient = useQueryClient();
  const toast = useToast();
  const [optimisticResolved, setOptimisticResolved] = useState<
    Record<string, ApprovalDecision>
  >({});

  const { data: approvals = [], isLoading, isError, error, refetch } = useQuery({
    queryKey: ["approvals"],
    queryFn: () => defaultApi.getPendingApprovals(),
    refetchInterval: 10_000,
  });

  const { mutate: resolve, isPending: isResolving, variables: resolvingVars } =
    useMutation({
      mutationFn: ({ id, decision }: { id: string; decision: ApprovalDecision }) =>
        defaultApi.resolveApproval(id, decision),
      onMutate: ({ id, decision }) => {
        setOptimisticResolved((prev) => ({ ...prev, [id]: decision }));
      },
      onSuccess: (_data, { decision }) => {
        toast.success(decision === 'approved' ? 'Approval granted.' : 'Approval denied.');
      },
      onSettled: () => {
        queryClient.invalidateQueries({ queryKey: ["approvals"] });
        queryClient.invalidateQueries({ queryKey: ["dashboard"] });
      },
      onError: (_err, { id }) => {
        toast.error('Failed to resolve approval — please try again.');
        setOptimisticResolved((prev) => {
          const next = { ...prev };
          delete next[id];
          return next;
        });
      },
    });

  // ── Split into pending vs resolved ────────────────────────────────────────
  const pending = approvals.filter(
    (a) => !a.decision && !optimisticResolved[a.approval_id]
  );
  const resolved = approvals.filter(
    (a) => a.decision || optimisticResolved[a.approval_id]
  );

  // Merge optimistic decision into resolved records for display
  const resolvedWithDecision: ApprovalRecord[] = resolved.map((a) => ({
    ...a,
    decision: a.decision ?? optimisticResolved[a.approval_id] ?? null,
  }));

  // ── Error state ───────────────────────────────────────────────────────────
  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center p-8">
        <ServerCrash size={40} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load approvals</p>
        <p className="text-sm text-zinc-500">
          {error instanceof Error ? error.message : "Unknown error"}
        </p>
        <button
          onClick={() => refetch()}
          className="mt-2 flex items-center gap-1.5 rounded-md bg-zinc-900 border border-zinc-800 px-3 py-1.5 text-xs text-zinc-400 hover:bg-white/5 transition-colors"
        >
          <RefreshCw size={13} /> Retry
        </button>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      {/* ── Header ─────────────────────────────────────────────────────── */}
      <div className="flex items-center justify-between">
        <h1 className="text-sm font-medium text-zinc-200">Approvals</h1>
        <div className="flex items-center gap-3">
          {pending.length > 0 && (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-950 px-2.5 py-1 text-xs font-semibold text-amber-300 ring-1 ring-amber-800">
              <span className="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse" />
              {pending.length} pending
            </span>
          )}
          <button
            onClick={() => refetch()}
            className="flex items-center gap-1.5 rounded-md bg-zinc-900 border border-zinc-800 px-2.5 py-1.5 text-[11px] text-zinc-500 hover:bg-white/5 hover:text-zinc-300 transition-colors"
          >
            <RefreshCw size={12} /> Refresh
          </button>
        </div>
      </div>

      {/* ── Pending approvals ───────────────────────────────────────────── */}
      <section>
        <h2 className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-3">
          Pending ({pending.length})
        </h2>

        {isLoading ? (
          /* Loading skeletons */
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {[1, 2, 3].map((i) => (
              <div
                key={i}
                className="rounded-lg bg-zinc-900 border border-zinc-800 p-4 animate-pulse"
              >
                <div className="flex justify-between mb-4">
                  <div className="h-4 w-32 rounded bg-zinc-700" />
                  <div className="h-5 w-16 rounded-full bg-zinc-800" />
                </div>
                <div className="space-y-2 mb-4">
                  <div className="h-3 w-48 rounded bg-zinc-800" />
                  <div className="h-3 w-40 rounded bg-zinc-800" />
                </div>
                <div className="flex gap-2 pt-3 border-t border-zinc-800">
                  <div className="h-9 flex-1 rounded-lg bg-zinc-800" />
                  <div className="h-9 flex-1 rounded-lg bg-zinc-800" />
                </div>
              </div>
            ))}
          </div>
        ) : pending.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 text-center rounded-lg border border-zinc-800">
            <Inbox size={36} className="text-zinc-700 mb-3" />
            <p className="text-zinc-400 font-medium">Inbox clear</p>
            <p className="text-sm text-zinc-600 mt-1">
              No approvals waiting for your action
            </p>
          </div>
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {pending.map((approval) => (
              <PendingCard
                key={approval.approval_id}
                approval={approval}
                onApprove={(id) => resolve({ id, decision: "approved" })}
                onReject={(id) => resolve({ id, decision: "rejected" })}
                isPending={
                  isResolving && resolvingVars?.id === approval.approval_id
                }
              />
            ))}
          </div>
        )}
      </section>

      {/* ── Resolved approvals ──────────────────────────────────────────── */}
      {resolvedWithDecision.length > 0 && (
        <section>
          <h2 className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-3">
            Resolved ({resolvedWithDecision.length})
          </h2>
          <div className="space-y-2">
            {resolvedWithDecision.map((approval) => (
              <ResolvedCard key={approval.approval_id} approval={approval} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}

export default ApprovalsPage;
