/**
 * Human-readable mapping for 409 `invalid_state_transition` errors coming
 * back from the run/sandbox state machine.
 *
 * Backend surfaces errors like:
 *   - `invalid run transition: partial_fence_triple -> suspended`
 *   - `invalid sandbox transition for run run_xyz: Failed -> Provisioning`
 *   - `{"code":"invalid_state_transition","message":"..."}`
 *
 * The raw strings leak internal state-machine vocabulary
 * (e.g. `partial_fence_triple`) that operators cannot act on. This helper
 * produces a friendlier, action-oriented message while keeping the raw
 * detail available for debugging if needed.
 */

import { ApiError } from "./api";

export type RunAction =
  | "pause"
  | "resume"
  | "orchestrate"
  | "intervene"
  | "recover"
  | "claim"
  | "diagnose";

export interface FriendlyRunError {
  /** Operator-facing message. */
  message: string;
  /** If we detected a state-machine error, true; otherwise false. */
  isStateTransition: boolean;
}

const RUN_TRANSITION_RE =
  /invalid\s+(run|sandbox)\s+transition(?:\s+for\s+run\s+\S+)?:\s*([A-Za-z0-9_]+)\s*->\s*([A-Za-z0-9_]+)/i;

/**
 * Convenience wrapper that produces just the string to pass to `toast.error`.
 * The optional `action` argument lets the classifier pick a verb that matches
 * what the operator just clicked (e.g. "Cannot resume..." for resume, rather
 * than always leaning on the target state).
 */
export function mapRunActionError(
  err: unknown,
  fallback: string,
  action?: RunAction,
): string {
  return classifyRunError(err, fallback, action).message;
}

export function classifyRunError(
  err: unknown,
  fallback: string,
  action?: RunAction,
): FriendlyRunError {
  if (err instanceof ApiError) {
    // Only treat this as a state-machine error when the backend says so
    // explicitly (code) or the message matches the canonical shape.
    // Other 409s (e.g. `run_terminal` on inject_message) already carry
    // actionable copy — don't collapse those.
    const isTransition =
      err.code === "invalid_state_transition" ||
      RUN_TRANSITION_RE.test(err.message);
    if (isTransition) {
      const friendly = friendlyTransitionMessage(err.message, action);
      return { message: friendly, isStateTransition: true };
    }
    return { message: err.message || fallback, isStateTransition: false };
  }
  if (err instanceof Error) {
    if (RUN_TRANSITION_RE.test(err.message)) {
      const friendly = friendlyTransitionMessage(err.message, action);
      return { message: friendly, isStateTransition: true };
    }
    return { message: err.message || fallback, isStateTransition: false };
  }
  return { message: fallback, isStateTransition: false };
}

/**
 * Build the user-facing string for a state-transition error. Prefers the
 * action the operator just took (unambiguous verb) over inferring from the
 * target state, which can be wrong (e.g. a 409 on Resume also targets
 * `running` but should say "Cannot resume", not "Cannot orchestrate").
 */
function friendlyTransitionMessage(raw: string, action?: RunAction): string {
  const m = RUN_TRANSITION_RE.exec(raw);
  const target = m ? m[3].toLowerCase() : "";

  switch (action) {
    case "pause":
      return "Cannot pause: the run is not in a pausable state right now.";
    case "resume":
      return "Cannot resume: the run is not paused.";
    case "orchestrate":
      return "Cannot orchestrate: this run has already finished — create a new run to continue.";
    case "intervene":
      return "Cannot intervene: the run is in a terminal state.";
    case "recover":
    case "claim":
    case "diagnose":
      return "Cannot complete this action in the run's current state.";
    default:
      // No action hint — infer from target state. Covers cases where the
      // error bubbles up from somewhere that didn't pass an action (e.g.
      // a generic Error from a non-mutation path).
      if (target === "suspended" || target === "paused") {
        return "Cannot pause: the run is not in a pausable state right now.";
      }
      if (target === "provisioning") {
        return "Cannot orchestrate: this run has already finished — create a new run to continue.";
      }
      if (target === "canceled" || target === "cancelled") {
        return "Cannot cancel: the run is already in a terminal state.";
      }
      return "This run cannot move to that state from its current state.";
  }
}

// ── Shared run-state sets ─────────────────────────────────────────────────────
//
// Centralized so OrchestrationPage, RunDetailPage, and TestHarnessPage all
// agree on which states allow which operator action, avoiding drift as the
// backend state machine evolves.

/**
 * States from which backend pause (`ff_suspend_execution`) can succeed.
 *
 * `pending` is intentionally excluded: a pending run has no lease yet,
 * so pause is rejected with `fence_required` / `partial_fence_triple`
 * → HTTP 409 `invalid run transition: partial_fence_triple -> suspended`.
 * See `crates/cairn-app/src/fabric_adapter.rs::is_suspend_state_conflict`
 * for the canonical list of reject codes.
 */
export const PAUSABLE_RUN_STATES: ReadonlySet<string> = new Set([
  "running",
  "waiting_approval",
  "waiting_dependency",
]);

/**
 * Terminal run states per `cairn-domain::RunState::is_terminal()`.
 * `dead_lettered` belongs to `TaskState`, not `RunState`, so it is NOT
 * included here.
 */
export const TERMINAL_RUN_STATES: ReadonlySet<string> = new Set([
  "completed",
  "failed",
  "canceled",
]);

/** Returns true if a pause call from `state` can plausibly succeed. */
export function isPausableState(state: string | undefined): boolean {
  return state !== undefined && PAUSABLE_RUN_STATES.has(state);
}

/** Returns true if `state` is terminal (no further transitions possible). */
export function isTerminalState(state: string | undefined): boolean {
  return state !== undefined && TERMINAL_RUN_STATES.has(state);
}

/**
 * Hover-tooltip string for a state-gated button.
 * Use when the button is disabled because of run state.
 */
export function stateGateTooltip(
  action: "pause" | "resume" | "orchestrate" | "intervene",
  currentState: string | undefined,
): string {
  const s = currentState ?? "unknown";
  switch (action) {
    case "pause":
      return `Cannot pause — run is ${s}.`;
    case "resume":
      return `Cannot resume — run is ${s} (only paused runs can resume).`;
    case "orchestrate":
      return `Run is ${s}; create a new run to continue.`;
    case "intervene":
      return `Cannot intervene — run is ${s}.`;
  }
}
