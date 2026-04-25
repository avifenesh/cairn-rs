/**
 * Shared formatters — cost, tokens, duration, relative time.
 *
 * Extracted in F29 CE so Run-telemetry, Stuck-runs, Session-cost and
 * Project-cost cards render numbers identically (and so fixes land in
 * one place instead of five). Existing per-page helpers stay for now;
 * new panels in CE use these.
 */

/** Format micros (1 USD = 1_000_000) as "$x.yyyyyy". Zero renders as "—". */
export function formatUsd(costMicros: number): string {
  if (!Number.isFinite(costMicros) || costMicros === 0) return "—";
  // Six fractional digits covers provider-level per-call precision
  // (observed per-call costs fall in the $0.000002–$0.12 range). We
  // intentionally DO NOT round harshly at small values — operators
  // debugging model-switch regressions need to see the fourth and
  // fifth decimal.
  return `$${(costMicros / 1_000_000).toFixed(6)}`;
}

/** Format a raw duration in ms as "123ms" | "1.4s" | "2m 07s" | "1h 05m". */
export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1_000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(1)}s`;
  if (ms < 3_600_000) {
    const m = Math.floor(ms / 60_000);
    const s = Math.floor((ms % 60_000) / 1_000);
    return `${m}m ${String(s).padStart(2, "0")}s`;
  }
  const h = Math.floor(ms / 3_600_000);
  const m = Math.floor((ms % 3_600_000) / 60_000);
  return `${h}h ${String(m).padStart(2, "0")}m`;
}

/** Format a token count as "12", "3.4k", "1.2M". */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/** Format "minutes ago" style for a past epoch-ms timestamp. */
export function formatRelativePast(ms: number, nowMs: number = Date.now()): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const d = nowMs - ms;
  if (d < 0) return "just now";
  if (d < 60_000) return "just now";
  if (d < 3_600_000) {
    const m = Math.floor(d / 60_000);
    return `${m} min ago`;
  }
  if (d < 86_400_000) {
    const h = Math.floor(d / 3_600_000);
    return `${h}h ago`;
  }
  const days = Math.floor(d / 86_400_000);
  return `${days}d ago`;
}

/** Truncate a string to N chars, adding an ellipsis if shortened. */
export function truncate(s: string, max: number): string {
  if (typeof s !== "string") return "";
  if (s.length <= max) return s;
  return `${s.slice(0, Math.max(0, max - 1))}…`;
}
