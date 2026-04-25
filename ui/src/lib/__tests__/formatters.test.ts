/**
 * Formatter unit tests — F29 CE.
 *
 * Deliberately exhaustive for `formatUsd` because Bugbot caught a
 * zero-vs-missing conflation here that would have made free-model
 * costs invisible in production.
 */

import { describe, it, expect } from "vitest";
import {
  formatUsd,
  formatDurationMs,
  formatTokens,
  formatRelativePast,
  truncate,
} from "../formatters";

describe("formatUsd", () => {
  it("renders zero as an explicit $0, not a missing-data em-dash", () => {
    // Free-model call is a LEGITIMATE cost of zero, not missing data.
    expect(formatUsd(0)).toBe("$0.000000");
  });

  it("renders positive micros with six fractional digits", () => {
    expect(formatUsd(3_500)).toBe("$0.003500");
    expect(formatUsd(1_250_000)).toBe("$1.250000");
  });

  it("renders negative values literally (should never happen, but do not crash)", () => {
    expect(formatUsd(-1)).toBe("$-0.000001");
  });

  it("falls back to em-dash for non-finite input", () => {
    expect(formatUsd(Number.NaN)).toBe("—");
    expect(formatUsd(Number.POSITIVE_INFINITY)).toBe("—");
  });
});

describe("formatDurationMs", () => {
  it("sub-second as ms", () => {
    expect(formatDurationMs(0)).toBe("0ms");
    expect(formatDurationMs(500)).toBe("500ms");
  });

  it("seconds with 1 decimal", () => {
    expect(formatDurationMs(2_500)).toBe("2.5s");
  });

  it("minutes + zero-padded seconds", () => {
    expect(formatDurationMs(65_000)).toBe("1m 05s");
    expect(formatDurationMs(125_000)).toBe("2m 05s");
  });

  it("hours + zero-padded minutes", () => {
    expect(formatDurationMs(3_600_000)).toBe("1h 00m");
    expect(formatDurationMs(3_900_000)).toBe("1h 05m");
  });

  it("rejects negatives and non-finite", () => {
    expect(formatDurationMs(-1)).toBe("—");
    expect(formatDurationMs(Number.NaN)).toBe("—");
  });
});

describe("formatTokens", () => {
  it("small ints pass through", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
  });
  it("thousands with k suffix", () => {
    expect(formatTokens(1_234)).toBe("1.2k");
  });
  it("millions with M suffix", () => {
    expect(formatTokens(1_500_000)).toBe("1.5M");
  });
});

describe("formatRelativePast", () => {
  const NOW = 1_700_000_000_000;
  it("just-now for < 60s", () => {
    expect(formatRelativePast(NOW - 30_000, NOW)).toBe("just now");
  });
  it("minutes", () => {
    expect(formatRelativePast(NOW - 5 * 60_000, NOW)).toBe("5 min ago");
  });
  it("hours", () => {
    expect(formatRelativePast(NOW - 3 * 3_600_000, NOW)).toBe("3h ago");
  });
  it("days", () => {
    expect(formatRelativePast(NOW - 2 * 86_400_000, NOW)).toBe("2d ago");
  });
});

describe("truncate", () => {
  it("short strings pass through", () => {
    expect(truncate("hello", 20)).toBe("hello");
  });
  it("long strings get a horizontal-ellipsis suffix", () => {
    const out = truncate("x".repeat(500), 10);
    expect(out.length).toBe(10);
    expect(out.endsWith("…")).toBe(true);
  });
});
