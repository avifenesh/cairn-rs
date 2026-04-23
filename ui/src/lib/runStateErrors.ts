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

export interface FriendlyRunError {
  /** Operator-facing message. */
  message: string;
  /** If we detected a state-machine error, true; otherwise false. */
  isStateTransition: boolean;
}

const RUN_TRANSITION_RE =
  /invalid\s+(run|sandbox)\s+transition(?:\s+for\s+run\s+\S+)?:\s*([A-Za-z0-9_]+)\s*->\s*([A-Za-z0-9_]+)/i;

export function mapRunActionError(err: unknown, fallback: string): string {
  const friendly = classifyRunError(err, fallback);
  return friendly.message;
}

export function classifyRunError(err: unknown, fallback: string): FriendlyRunError {
  if (err instanceof ApiError) {
    // Explicit 409 code from backend.
    if (err.code === "invalid_state_transition" || err.status === 409) {
      const parsed = parseTransitionMessage(err.message);
      if (parsed) return { message: parsed, isStateTransition: true };
      return {
        message: "This run is not in a state that allows this action.",
        isStateTransition: true,
      };
    }
    // Authentication / rate-limit / server-error — let the raw (but short)
    // message through; it is already structured.
    return { message: err.message || fallback, isStateTransition: false };
  }
  if (err instanceof Error) {
    const parsed = parseTransitionMessage(err.message);
    if (parsed) return { message: parsed, isStateTransition: true };
    return { message: err.message || fallback, isStateTransition: false };
  }
  return { message: fallback, isStateTransition: false };
}

function parseTransitionMessage(raw: string): string | null {
  const m = RUN_TRANSITION_RE.exec(raw);
  if (!m) return null;
  const target = m[3].toLowerCase();
  // Map known targets to verbs.
  if (target === "suspended" || target === "paused") {
    return "Cannot pause: the run is not in a pausable state right now.";
  }
  if (target === "running" || target === "provisioning") {
    return "Cannot orchestrate: this run has already finished — create a new run to continue.";
  }
  if (target === "canceled" || target === "cancelled") {
    return "Cannot cancel: the run is already in a terminal state.";
  }
  // Generic fallback keeps it readable without leaking the raw enum names.
  return "This run cannot move to that state from its current state.";
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
