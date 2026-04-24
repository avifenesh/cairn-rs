/**
 * ApprovalsPage — unified operator inbox for BOTH approval kinds:
 *
 *   1. Legacy `ApprovalRecord` — plan review, release gates, run-level
 *      pauses. Uses `/v1/approvals` + `/v1/approvals/:id/{approve,reject}`.
 *   2. `ToolCallApprovalRecord` (PR BP-6) — per-tool-call gating with
 *      amend-before-approve, session widening, and match policies.
 *      Uses `/v1/tool-call-approvals/*`.
 *
 * Layout is list-on-left + side drawer on right (the shape the user
 * picked during PR BP-6 design review). Row kinds render uniform badge
 * + metadata rows; clicking a row opens the matching drawer:
 *
 *   - ApprovalDrawer       — legacy approve/reject with confirmation.
 *   - ToolCallDrawer       — view args, amend inline, pick scope (Once |
 *                            Session + optional match policy), reject
 *                            with reason, approve.
 */

import { useMemo, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import {
  Check,
  Inbox,
  Loader2,
  Pencil,
  RefreshCw,
  Search,
  Wrench,
  X,
} from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { StatCard } from "../components/StatCard";
import { CopyButton } from "../components/CopyButton";
import { Drawer } from "../components/Drawer";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import type {
  ApprovalDecision,
  ApprovalMatchPolicy,
  ApprovalRecord,
  ToolCallApprovalRecord,
} from "../lib/types";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";
import { EmptyScopeHint } from "../components/EmptyScopeHint";

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

// ── Unified row model ──────────────────────────────────────────────────────────
//
// The two projections have different shapes on the wire. Rather than
// sprinkle narrowing throughout the render tree we lift them into a
// single `Row` and keep a tagged back-reference to the original record
// so the drawer can render a kind-specific panel.

type Row =
  | {
      kind: "legacy";
      id: string;
      tool: string;            // label ("plan" | "pause")
      runId: string | null;
      createdAt: number;
      resolved: boolean;
      decision: ApprovalDecision | null;
      source: ApprovalRecord;
    }
  | {
      kind: "tool";
      id: string;
      tool: string;            // actual tool name
      runId: string | null;
      createdAt: number;
      resolved: boolean;
      decision: "approved" | "rejected" | null;
      source: ToolCallApprovalRecord;
    };

const legacyLabel = (a: ApprovalRecord): string =>
  a.task_id ? "plan" : "pause";

const toolCallDecision = (r: ToolCallApprovalRecord): "approved" | "rejected" | null => {
  if (r.state === "approved") return "approved";
  if (r.state === "rejected" || r.state === "timeout") return "rejected";
  return null;
};

const toRow = (r: ApprovalRecord | ToolCallApprovalRecord): Row =>
  "call_id" in r
    ? {
        kind: "tool",
        id: r.call_id,
        tool: r.tool_name,
        runId: r.run_id,
        createdAt: r.proposed_at_ms,
        resolved: r.state !== "pending",
        decision: toolCallDecision(r),
        source: r,
      }
    : {
        kind: "legacy",
        id: r.approval_id,
        tool: legacyLabel(r),
        runId: r.run_id,
        createdAt: r.created_at,
        resolved: r.decision !== null,
        decision: r.decision,
        source: r,
      };

// ── Badges ─────────────────────────────────────────────────────────────────────

function KindBadge({ kind, label }: { kind: Row["kind"]; label: string }) {
  // Colour by kind; text is the specific tool/label.
  if (kind === "tool") {
    return (
      <span
        title={`Tool call: ${label}`}
        className="inline-flex items-center gap-1 text-[10px] font-mono font-semibold text-sky-300 bg-sky-950/60 border border-sky-800/50 rounded px-1.5 py-0.5"
      >
        <Wrench size={9} strokeWidth={2.5} />
        {label}
      </span>
    );
  }
  return (
    <span
      title={`Approval kind: ${label}`}
      className="inline-flex items-center gap-1 text-[10px] font-mono font-semibold text-amber-300 bg-amber-950/60 border border-amber-800/50 rounded px-1.5 py-0.5"
    >
      {label}
    </span>
  );
}

function StatusDot({ resolved, decision }: { resolved: boolean; decision: Row["decision"] }) {
  const cls = !resolved
    ? "bg-amber-400 shadow-[0_0_6px_rgba(251,191,36,0.6)]"
    : decision === "approved"
      ? "bg-emerald-500"
      : "bg-red-500";
  const label = !resolved
    ? "pending"
    : decision === "approved"
      ? "approved"
      : "rejected";
  return (
    <span
      title={label}
      className={clsx(
        "inline-block rounded-full",
        resolved ? "size-2" : "size-2.5",
        cls,
      )}
      aria-label={label}
    />
  );
}

// ── Row ────────────────────────────────────────────────────────────────────────

function RowItem({
  row,
  selected,
  onClick,
}: {
  row: Row;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        "w-full flex items-center gap-3 px-3 h-9 text-left transition-colors border-l-2",
        selected
          ? "bg-indigo-500/10 border-indigo-500"
          : "border-transparent hover:bg-gray-100/60 dark:hover:bg-zinc-800/60",
      )}
    >
      <KindBadge kind={row.kind} label={row.tool} />
      <span className="font-mono text-[11px] text-gray-500 dark:text-zinc-400 truncate" title={row.id}>
        {shortId(row.id)}
      </span>
      <span
        className="ml-auto tabular-nums text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap"
        title={fmtTime(row.createdAt)}
      >
        {fmtRelative(row.createdAt)}
      </span>
      <StatusDot resolved={row.resolved} decision={row.decision} />
    </button>
  );
}

// ── Legacy approval drawer ────────────────────────────────────────────────────

function LegacyDrawerBody({
  approval,
  onClose,
}: {
  approval: ApprovalRecord;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();

  const resolve = useMutation({
    mutationFn: (decision: ApprovalDecision) =>
      defaultApi.resolveApproval(approval.approval_id, decision),
    onSuccess: (_, decision) => {
      toast.success(decision === "approved" ? "Approval granted." : "Approval denied.");
      void qc.invalidateQueries({ queryKey: ["approvals"] });
      void qc.invalidateQueries({ queryKey: ["runs"] });
      if (approval.run_id) {
        void qc.invalidateQueries({ queryKey: ["run-detail", approval.run_id] });
        void qc.invalidateQueries({ queryKey: ["run-events", approval.run_id] });
      }
      onClose();
    },
    onError: (err: unknown) =>
      toast.error(`Failed to resolve — ${err instanceof Error ? err.message : "try again."}`),
  });

  return (
    <div className="p-4 flex flex-col gap-3 text-[12px]">
      <KV label="Approval ID"><Mono>{approval.approval_id}</Mono><CopyButton text={approval.approval_id} size={10} /></KV>
      {approval.run_id && <KV label="Run"><Mono>{approval.run_id}</Mono><CopyButton text={approval.run_id} size={10} /></KV>}
      {approval.task_id && <KV label="Task"><Mono>{approval.task_id}</Mono></KV>}
      <KV label="Requirement">{approval.requirement}</KV>
      <KV label="Requested">{fmtTime(approval.created_at)}</KV>
      {approval.decision && (
        <KV label="Decision">
          <span className={clsx(
            "text-[11px] font-medium rounded px-1.5 py-0.5",
            approval.decision === "approved"
              ? "text-emerald-400 bg-emerald-950/50 border border-emerald-800/40"
              : "text-red-400 bg-red-950/50 border border-red-800/40",
          )}>
            {approval.decision}
          </span>
        </KV>
      )}
      {approval.decision === null && (
        <div className="pt-2 flex gap-2">
          <button
            onClick={() => resolve.mutate("rejected")}
            disabled={resolve.isPending}
            className="flex-1 px-3 h-8 rounded text-[12px] font-medium bg-red-900/40 text-red-300 hover:bg-red-900/70 border border-red-800/50 transition-colors disabled:opacity-40 inline-flex items-center justify-center gap-1.5"
          >
            <X size={13} /> Reject
          </button>
          <button
            onClick={() => resolve.mutate("approved")}
            disabled={resolve.isPending}
            className="flex-1 px-3 h-8 rounded text-[12px] font-medium bg-emerald-900/50 text-emerald-300 hover:bg-emerald-900 border border-emerald-800/50 transition-colors disabled:opacity-40 inline-flex items-center justify-center gap-1.5"
          >
            {resolve.isPending ? <Loader2 size={13} className="animate-spin" /> : <Check size={13} />}
            Approve
          </button>
        </div>
      )}
    </div>
  );
}

// ── Tool-call drawer ──────────────────────────────────────────────────────────

type ScopeType = "once" | "session";

function ToolCallDrawerBody({
  record,
  onClose,
}: {
  record: ToolCallApprovalRecord;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();

  // The args the operator is approving — seeded from the live record
  // (amended > original). Edits here are staged locally; PATCH is
  // only fired when the user hits "Save amendment".
  const effectiveArgs = useMemo(
    () => JSON.stringify(record.amended_tool_args ?? record.original_tool_args, null, 2),
    [record],
  );
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(effectiveArgs);
  const [scopeType, setScopeType] = useState<ScopeType>("once");
  const [matchOverride, setMatchOverride] =
    useState<ApprovalMatchPolicy | undefined>(undefined);
  const [rejectReason, setRejectReason] = useState("");
  const [rejectOpen, setRejectOpen] = useState(false);

  const onResolved = () => {
    void qc.invalidateQueries({ queryKey: ["tool-call-approvals"] });
    void qc.invalidateQueries({ queryKey: ["approvals"] });
    if (record.run_id) {
      void qc.invalidateQueries({ queryKey: ["run-detail", record.run_id] });
      void qc.invalidateQueries({ queryKey: ["run-events", record.run_id] });
    }
    onClose();
  };

  const amend = useMutation({
    mutationFn: (new_tool_args: unknown) =>
      defaultApi.amendToolCallApproval(record.call_id, { new_tool_args }),
    onSuccess: () => {
      toast.success("Arguments amended.");
      setEditing(false);
      void qc.invalidateQueries({ queryKey: ["tool-call-approvals"] });
    },
    onError: (err: unknown) =>
      toast.error(`Amend failed — ${err instanceof Error ? err.message : "try again."}`),
  });

  const approve = useMutation({
    mutationFn: () =>
      defaultApi.approveToolCallApproval(record.call_id, {
        scope:
          scopeType === "once"
            ? { type: "once" }
            : { type: "session", match_policy: matchOverride },
      }),
    onSuccess: () => {
      toast.success("Tool call approved.");
      onResolved();
    },
    onError: (err: unknown) =>
      toast.error(`Approve failed — ${err instanceof Error ? err.message : "try again."}`),
  });

  const reject = useMutation({
    mutationFn: () =>
      defaultApi.rejectToolCallApproval(record.call_id, {
        reason: rejectReason.trim() ? rejectReason.trim() : undefined,
      }),
    onSuccess: () => {
      toast.success("Tool call rejected.");
      onResolved();
    },
    onError: (err: unknown) =>
      toast.error(`Reject failed — ${err instanceof Error ? err.message : "try again."}`),
  });

  const handleSaveAmend = () => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(draft);
    } catch (e) {
      toast.error(`Invalid JSON: ${e instanceof Error ? e.message : "parse error"}`);
      return;
    }
    amend.mutate(parsed);
  };

  const pending = record.state === "pending";
  const busy = amend.isPending || approve.isPending || reject.isPending;

  return (
    <div className="p-4 flex flex-col gap-3 text-[12px]">
      <KV label="Tool">
        <span className="font-mono text-sky-300">{record.tool_name}</span>
      </KV>
      <KV label="Call ID"><Mono>{record.call_id}</Mono><CopyButton text={record.call_id} size={10} /></KV>
      <KV label="Run"><Mono>{record.run_id}</Mono><CopyButton text={record.run_id} size={10} /></KV>
      <KV label="Session"><Mono>{record.session_id}</Mono></KV>
      <KV label="Proposed">{fmtTime(record.proposed_at_ms)}</KV>
      {record.display_summary && (
        <div className="text-gray-500 dark:text-zinc-400 italic">
          “{record.display_summary}”
        </div>
      )}

      <div>
        <div className="flex items-center justify-between mb-1">
          <span className="text-[11px] font-medium uppercase tracking-wide text-gray-400 dark:text-zinc-500">
            {record.amended_tool_args ? "Amended arguments" : "Arguments"}
          </span>
          {pending && !editing && (
            <button
              onClick={() => { setDraft(effectiveArgs); setEditing(true); }}
              className="inline-flex items-center gap-1 text-[11px] text-indigo-400 hover:text-indigo-300 transition-colors"
            >
              <Pencil size={10} /> Edit args
            </button>
          )}
        </div>
        {editing ? (
          <div className="flex flex-col gap-2">
            <textarea
              value={draft}
              onChange={e => setDraft(e.target.value)}
              rows={10}
              spellCheck={false}
              className="w-full font-mono text-[11px] text-gray-800 dark:text-zinc-200 bg-gray-50 dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded p-2 focus:outline-none focus:border-indigo-500"
            />
            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setEditing(false)}
                disabled={busy}
                className="px-2 h-7 rounded text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-200 transition-colors disabled:opacity-40"
              >
                Cancel
              </button>
              <button
                onClick={handleSaveAmend}
                disabled={busy}
                className="px-3 h-7 rounded text-[11px] font-medium bg-indigo-900/60 text-indigo-200 hover:bg-indigo-900 border border-indigo-800/50 transition-colors disabled:opacity-40 inline-flex items-center gap-1.5"
              >
                {amend.isPending ? <Loader2 size={11} className="animate-spin" /> : <Pencil size={11} />}
                Save amendment
              </button>
            </div>
          </div>
        ) : (
          <pre className="font-mono text-[11px] text-gray-800 dark:text-zinc-200 bg-gray-50 dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded p-2 overflow-x-auto whitespace-pre-wrap">
{effectiveArgs}
          </pre>
        )}
      </div>

      {pending && !editing && (
        <>
          <fieldset className="flex flex-col gap-2 pt-1 border-t border-gray-200 dark:border-zinc-800">
            <legend className="text-[11px] font-medium uppercase tracking-wide text-gray-400 dark:text-zinc-500 pt-2">
              Scope
            </legend>
            <label className="flex items-center gap-2 text-[12px]">
              <input
                type="radio"
                checked={scopeType === "once"}
                onChange={() => setScopeType("once")}
                className="accent-indigo-500"
              />
              <span>Once — this call only</span>
            </label>
            <label className="flex items-center gap-2 text-[12px]">
              <input
                type="radio"
                checked={scopeType === "session"}
                onChange={() => setScopeType("session")}
                className="accent-indigo-500"
              />
              <span>
                Session — widen to matching calls via{" "}
                <span className="font-mono text-[11px] text-gray-500 dark:text-zinc-400">
                  {(matchOverride ?? record.match_policy).kind}
                </span>
              </span>
            </label>
            {scopeType === "session" && (
              <MatchPolicyPicker
                current={matchOverride ?? record.match_policy}
                onChange={setMatchOverride}
              />
            )}
          </fieldset>

          {rejectOpen ? (
            <div className="flex flex-col gap-2 pt-1 border-t border-gray-200 dark:border-zinc-800">
              <label className="text-[11px] font-medium uppercase tracking-wide text-gray-400 dark:text-zinc-500 pt-2">
                Reject reason (optional — surfaced to the agent)
              </label>
              <textarea
                value={rejectReason}
                onChange={e => setRejectReason(e.target.value)}
                rows={2}
                placeholder="e.g. path is outside approved scope"
                className="w-full text-[12px] text-gray-800 dark:text-zinc-200 bg-gray-50 dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded p-2 focus:outline-none focus:border-red-500"
              />
              <div className="flex gap-2">
                <button
                  onClick={() => { setRejectOpen(false); setRejectReason(""); }}
                  disabled={busy}
                  className="px-3 h-8 rounded text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-200 transition-colors disabled:opacity-40"
                >
                  Cancel
                </button>
                <button
                  onClick={() => reject.mutate()}
                  disabled={busy}
                  className="flex-1 px-3 h-8 rounded text-[12px] font-medium bg-red-900/50 text-red-300 hover:bg-red-900 border border-red-800/50 transition-colors disabled:opacity-40 inline-flex items-center justify-center gap-1.5"
                >
                  {reject.isPending ? <Loader2 size={13} className="animate-spin" /> : <X size={13} />}
                  Confirm reject
                </button>
              </div>
            </div>
          ) : (
            <div className="flex gap-2 pt-2">
              <button
                onClick={() => setRejectOpen(true)}
                disabled={busy}
                className="flex-1 px-3 h-8 rounded text-[12px] font-medium bg-red-900/40 text-red-300 hover:bg-red-900/70 border border-red-800/50 transition-colors disabled:opacity-40 inline-flex items-center justify-center gap-1.5"
              >
                <X size={13} /> Reject
              </button>
              <button
                onClick={() => approve.mutate()}
                disabled={busy}
                className="flex-1 px-3 h-8 rounded text-[12px] font-medium bg-emerald-900/50 text-emerald-200 hover:bg-emerald-900 border border-emerald-800/50 transition-colors disabled:opacity-40 inline-flex items-center justify-center gap-1.5"
              >
                {approve.isPending ? <Loader2 size={13} className="animate-spin" /> : <Check size={13} />}
                Approve
              </button>
            </div>
          )}
        </>
      )}

      {!pending && (
        <div className="pt-2 border-t border-gray-200 dark:border-zinc-800 text-[11px] text-gray-500 dark:text-zinc-400">
          Resolved: <span className="font-medium text-gray-700 dark:text-zinc-300">{record.state}</span>
          {record.reason && <> — {record.reason}</>}
          {record.operator_id && <> — by {record.operator_id}</>}
        </div>
      )}
    </div>
  );
}

function MatchPolicyPicker({
  current,
  onChange,
}: {
  current: ApprovalMatchPolicy;
  onChange: (p: ApprovalMatchPolicy | undefined) => void;
}) {
  const kind = current.kind;
  return (
    <div className="ml-5 flex flex-col gap-1.5 text-[11px] text-gray-500 dark:text-zinc-400">
      <label className="flex items-center gap-2">
        <span className="w-20">Policy</span>
        <select
          value={kind}
          onChange={e => {
            const next = e.target.value as ApprovalMatchPolicy["kind"];
            if (next === "exact") onChange({ kind: "exact" });
            else if (next === "exact_path")
              onChange({
                kind: "exact_path",
                path: "path" in current ? current.path : "project_root" in current ? current.project_root : "",
              });
            else if (next === "project_scoped_path")
              onChange({
                kind: "project_scoped_path",
                project_root:
                  "project_root" in current ? current.project_root : "path" in current ? current.path : "",
              });
          }}
          className="flex-1 bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded px-2 h-7 text-[11px] text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500"
        >
          <option value="exact">exact</option>
          <option value="exact_path">exact_path</option>
          <option value="project_scoped_path">project_scoped_path</option>
        </select>
      </label>
      {current.kind === "exact_path" && (
        <label className="flex items-center gap-2">
          <span className="w-20">Path</span>
          <input
            value={current.path}
            onChange={e => onChange({ kind: "exact_path", path: e.target.value })}
            placeholder="/abs/path/to/file"
            className="flex-1 font-mono bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded px-2 h-7 text-[11px] text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500"
          />
        </label>
      )}
      {current.kind === "project_scoped_path" && (
        <label className="flex items-center gap-2">
          <span className="w-20">Root</span>
          <input
            value={current.project_root}
            onChange={e => onChange({ kind: "project_scoped_path", project_root: e.target.value })}
            placeholder="/workspaces/proj"
            className="flex-1 font-mono bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded px-2 h-7 text-[11px] text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500"
          />
        </label>
      )}
    </div>
  );
}

// ── KV helpers ────────────────────────────────────────────────────────────────

function KV({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2 min-h-[20px]">
      <span className="w-24 shrink-0 text-[11px] uppercase tracking-wide text-gray-400 dark:text-zinc-500">
        {label}
      </span>
      <div className="flex items-center gap-1 min-w-0 flex-1">{children}</div>
    </div>
  );
}

const Mono = ({ children }: { children: React.ReactNode }) => (
  <span className="font-mono text-[11px] text-gray-700 dark:text-zinc-300 truncate">{children}</span>
);

// ── Filter tabs ────────────────────────────────────────────────────────────────

type KindFilter = "all" | "tool" | "legacy";
type StateFilter = "all" | "pending" | "resolved";

// ── Page ──────────────────────────────────────────────────────────────────────

export function ApprovalsPage() {
  const { ms: refreshMs, setOption, interval } = useAutoRefresh("approvals", "15s");

  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [stateFilter, setStateFilter] = useState<StateFilter>("all");
  const [search, setSearch] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const legacyQ = useQuery({
    queryKey: ["approvals"],
    queryFn: () => defaultApi.getAllApprovals(),
    refetchInterval: refreshMs,
  });
  const toolQ = useQuery({
    queryKey: ["tool-call-approvals"],
    queryFn: () => defaultApi.listToolCallApprovals(),
    refetchInterval: refreshMs,
  });

  const rows: Row[] = useMemo(() => {
    const merged: Row[] = [
      ...(legacyQ.data ?? []).map(toRow),
      ...(toolQ.data ?? []).map(toRow),
    ];
    // Newest first.
    merged.sort((a, b) => b.createdAt - a.createdAt);
    return merged;
  }, [legacyQ.data, toolQ.data]);

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return rows.filter(r => {
      if (kindFilter === "tool" && r.kind !== "tool") return false;
      if (kindFilter === "legacy" && r.kind !== "legacy") return false;
      if (stateFilter === "pending" && r.resolved) return false;
      if (stateFilter === "resolved" && !r.resolved) return false;
      if (needle) {
        return (
          r.id.toLowerCase().includes(needle) ||
          r.tool.toLowerCase().includes(needle) ||
          (r.runId ?? "").toLowerCase().includes(needle)
        );
      }
      return true;
    });
  }, [rows, kindFilter, stateFilter, search]);

  const selected = useMemo(
    () => filtered.find(r => r.id === selectedId) ?? rows.find(r => r.id === selectedId),
    [filtered, rows, selectedId],
  );

  const pending24 = rows.filter(r => !r.resolved).length;
  const approved24 = useMemo(() => {
    const since = Date.now() - 86_400_000;
    return rows.filter(r => r.resolved && r.decision === "approved" && r.createdAt >= since).length;
  }, [rows]);
  const rejected24 = useMemo(() => {
    const since = Date.now() - 86_400_000;
    return rows.filter(r => r.resolved && r.decision === "rejected" && r.createdAt >= since).length;
  }, [rows]);

  const isLoading = legacyQ.isLoading || toolQ.isLoading;
  const isFetching = legacyQ.isFetching || toolQ.isFetching;
  const error = legacyQ.error ?? toolQ.error;

  if (legacyQ.isError && toolQ.isError) {
    return (
      <ErrorFallback
        error={error}
        resource="approvals"
        onRetry={() => {
          void legacyQ.refetch();
          void toolQ.refetch();
        }}
      />
    );
  }

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Stat strip */}
      {!isLoading && (
        <div className="grid grid-cols-3 gap-x-6 gap-y-3 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 shrink-0">
          <StatCard compact
            label="Pending"
            value={pending24}
            description={pending24 > 0 ? "requires action" : "inbox clear"}
            variant={pending24 > 0 ? "warning" : "success"}
          />
          <StatCard compact label="Approved (24h)" value={approved24} variant="success" />
          <StatCard compact label="Rejected (24h)" value={rejected24} variant="danger" />
        </div>
      )}

      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <div className="flex items-center gap-0">
          {(["all", "tool", "legacy"] as KindFilter[]).map(k => (
            <button
              key={k}
              onClick={() => setKindFilter(k)}
              className={clsx(
                "px-2 h-10 text-[11px] font-medium transition-colors border-b-2",
                kindFilter === k
                  ? "text-gray-900 dark:text-zinc-100 border-indigo-500"
                  : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
              )}
            >
              {k === "all" ? "All" : k === "tool" ? "Tool" : "Plan / Pause"}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-0 ml-2">
          {(["all", "pending", "resolved"] as StateFilter[]).map(s => (
            <button
              key={s}
              onClick={() => setStateFilter(s)}
              className={clsx(
                "px-2 h-10 text-[11px] font-medium transition-colors border-b-2",
                stateFilter === s
                  ? "text-gray-900 dark:text-zinc-100 border-indigo-500"
                  : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
              )}
            >
              {s[0].toUpperCase() + s.slice(1)}
            </button>
          ))}
        </div>

        <div className="relative ml-auto">
          <Search size={11} className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
          <input
            value={search}
            onChange={e => setSearch(e.target.value)}
            placeholder="Search tool, id, run…"
            className="h-7 pl-6 pr-2 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500 transition-colors w-56"
          />
        </div>

        <div className="flex items-center gap-1">
          <div className="relative">
            <select
              value={interval.option}
              onChange={e => setOption(e.target.value as import("../hooks/useAutoRefresh").RefreshOption)}
              className="appearance-none rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] font-mono pl-5 pr-2 h-7 text-gray-500 dark:text-zinc-400 focus:outline-none focus:border-indigo-500 transition-colors"
              title="Auto-refresh interval"
            >
              {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
            </select>
            <span className="absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none">
              <RefreshCw size={9} className={isFetching ? "animate-spin text-indigo-400" : "text-gray-400 dark:text-zinc-600"} />
            </span>
          </div>
          <button
            onClick={() => { void legacyQ.refetch(); void toolQ.refetch(); }}
            disabled={isFetching}
            className="flex items-center gap-1 h-7 px-2 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600 disabled:opacity-40 transition-colors"
            title="Refresh now"
          >
            <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
            <span className="hidden sm:inline">Refresh</span>
          </button>
        </div>
      </div>

      {/* List */}
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <div className="divide-y divide-gray-200 dark:divide-zinc-800/40">
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} className="flex items-center gap-4 px-4 h-9 animate-pulse">
                <div className="h-3 w-10 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="h-2.5 w-28 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="ml-auto h-2.5 w-12 rounded bg-gray-100 dark:bg-zinc-800" />
                <div className="h-2 w-2 rounded-full bg-gray-100 dark:bg-zinc-800" />
              </div>
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 gap-2 text-center px-6">
            <Inbox size={26} className="text-gray-300 dark:text-zinc-600" />
            <p className="text-[13px] text-gray-400 dark:text-zinc-600 font-medium">Inbox clear</p>
            <p className="text-[11px] text-gray-300 dark:text-zinc-600 max-w-xs">
              No approvals match this filter. Approvals appear here when a run hits a
              human-in-the-loop gate or a tool call needs operator sign-off.
            </p>
            <EmptyScopeHint empty className="max-w-lg" />
          </div>
        ) : (
          <div className="divide-y divide-gray-100 dark:divide-zinc-800/40">
            {filtered.map(r => (
              <RowItem
                key={`${r.kind}:${r.id}`}
                row={r}
                selected={selectedId === r.id}
                onClick={() => setSelectedId(r.id)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Drawer */}
      <Drawer
        open={selected !== undefined}
        onClose={() => setSelectedId(null)}
        title={
          selected?.kind === "tool"
            ? `Tool call · ${selected.tool}`
            : selected
              ? `Approval · ${selected.tool}`
              : undefined
        }
        width="w-[420px]"
      >
        {selected?.kind === "legacy" && (
          <LegacyDrawerBody approval={selected.source} onClose={() => setSelectedId(null)} />
        )}
        {selected?.kind === "tool" && (
          <ToolCallDrawerBody record={selected.source} onClose={() => setSelectedId(null)} />
        )}
      </Drawer>
    </div>
  );
}

export default ApprovalsPage;
