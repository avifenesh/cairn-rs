import { clsx } from "clsx";
import type { RunState } from "../lib/types";

const STATE_STYLES: Record<RunState, string> = {
  pending:            "bg-gray-100/80 dark:bg-zinc-800/80 text-gray-500 dark:text-zinc-400",
  running:            "bg-indigo-500/10 text-indigo-400",
  paused:             "bg-amber-500/10  text-amber-400",
  waiting_approval:   "bg-violet-500/10 text-violet-400",
  waiting_dependency: "bg-sky-500/10    text-sky-400",
  completed:          "bg-emerald-500/10 text-emerald-400",
  failed:             "bg-red-500/10    text-red-400",
  canceled:           "bg-gray-100/60 dark:bg-zinc-800/60   text-gray-400 dark:text-zinc-500",
};

const STATE_DOT: Record<RunState, string> = {
  pending:            "bg-zinc-500",
  running:            "bg-indigo-400 animate-pulse",
  paused:             "bg-amber-400",
  waiting_approval:   "bg-violet-400",
  waiting_dependency: "bg-sky-400",
  completed:          "bg-emerald-500",
  failed:             "bg-red-500",
  canceled:           "bg-zinc-600",
};

const STATE_LABEL: Record<RunState, string> = {
  pending:            "Pending",
  running:            "Running",
  paused:             "Paused",
  waiting_approval:   "Awaiting Approval",
  waiting_dependency: "Waiting",
  completed:          "Completed",
  failed:             "Failed",
  canceled:           "Canceled",
};

interface StateBadgeProps {
  state: RunState;
  compact?: boolean;
}

export function StateBadge({ state, compact = false }: StateBadgeProps) {
  const styles = STATE_STYLES[state] ?? STATE_STYLES.pending;
  const dot    = STATE_DOT[state]    ?? STATE_DOT.pending;
  const label  = STATE_LABEL[state]  ?? state;

  return (
    <span className={clsx(
      "inline-flex items-center gap-1.5 rounded font-medium whitespace-nowrap",
      compact ? "px-1.5 py-0.5 text-[11px]" : "px-2 py-1 text-xs",
      styles,
    )}>
      <span className={clsx("rounded-full shrink-0", compact ? "w-1.5 h-1.5" : "w-1.5 h-1.5", dot)} />
      {label}
    </span>
  );
}
