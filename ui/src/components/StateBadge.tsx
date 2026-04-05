import { clsx } from "clsx";
import type { RunState } from "../lib/types";

// ── State colour map ──────────────────────────────────────────────────────────

const STATE_STYLES: Record<RunState, string> = {
  pending:             "bg-zinc-800   text-zinc-400  ring-zinc-700",
  running:             "bg-blue-950   text-blue-300  ring-blue-800",
  paused:              "bg-amber-950  text-amber-300 ring-amber-800",
  waiting_approval:    "bg-violet-950 text-violet-300 ring-violet-800",
  waiting_dependency:  "bg-sky-950    text-sky-300   ring-sky-800",
  completed:           "bg-emerald-950 text-emerald-400 ring-emerald-800",
  failed:              "bg-red-950    text-red-400   ring-red-800",
  canceled:            "bg-zinc-900   text-zinc-500  ring-zinc-700",
};

const STATE_DOT: Record<RunState, string> = {
  pending:             "bg-zinc-400",
  running:             "bg-blue-400 animate-pulse",
  paused:              "bg-amber-400",
  waiting_approval:    "bg-violet-400",
  waiting_dependency:  "bg-sky-400",
  completed:           "bg-emerald-400",
  failed:              "bg-red-400",
  canceled:            "bg-zinc-500",
};

const STATE_LABEL: Record<RunState, string> = {
  pending:             "Pending",
  running:             "Running",
  paused:              "Paused",
  waiting_approval:    "Awaiting Approval",
  waiting_dependency:  "Waiting",
  completed:           "Completed",
  failed:              "Failed",
  canceled:            "Canceled",
};

// ── Component ─────────────────────────────────────────────────────────────────

interface StateBadgeProps {
  state: RunState;
  /** Use a smaller variant in table cells (default: false) */
  compact?: boolean;
}

export function StateBadge({ state, compact = false }: StateBadgeProps) {
  const styles = STATE_STYLES[state] ?? STATE_STYLES.pending;
  const dot    = STATE_DOT[state]    ?? STATE_DOT.pending;
  const label  = STATE_LABEL[state]  ?? state;

  return (
    <span
      className={clsx(
        "inline-flex items-center gap-1.5 rounded-full font-medium ring-1 whitespace-nowrap",
        compact ? "px-2 py-0.5 text-xs" : "px-2.5 py-1 text-xs",
        styles,
      )}
    >
      <span className={clsx("rounded-full shrink-0", compact ? "w-1.5 h-1.5" : "w-2 h-2", dot)} />
      {label}
    </span>
  );
}
