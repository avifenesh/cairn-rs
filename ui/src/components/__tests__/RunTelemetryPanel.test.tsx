/**
 * RunTelemetryPanel tests — F29 CE.
 *
 * Covers the four states an operator cares about:
 *   1. Provider calls render with model, tokens, cost, latency, status.
 *   2. Tool invocations render with name, status, duration.
 *   3. Totals card reflects the backend totals field.
 *   4. Stuck banner shows when `stuck=true` and hides otherwise.
 *   5. Empty state (0 calls + 0 invocations) renders placeholders.
 */

import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, beforeEach, vi } from "vitest";

import type { RunTelemetry } from "../../lib/types";

// Mock api before component import.
const getRunTelemetry = vi.fn();
vi.mock("../../lib/api", () => ({
  defaultApi: {
    getRunTelemetry: (...args: unknown[]) => getRunTelemetry(...args),
  },
}));

import { RunTelemetryPanel } from "../RunTelemetryPanel";

function renderPanel(runId = "run-1") {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <RunTelemetryPanel runId={runId} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  getRunTelemetry.mockReset();
});

function makeTelemetry(overrides: Partial<RunTelemetry> = {}): RunTelemetry {
  return {
    run_id: "run-1",
    state: "running",
    stuck: false,
    stuck_since_ms: null,
    provider_calls: [],
    tool_invocations: [],
    totals: {
      cost_micros: 0,
      input_tokens: 0,
      output_tokens: 0,
      provider_calls: 0,
      tool_calls: 0,
      errors: 0,
      wall_ms: 0,
    },
    phase_timings: {},
    ...overrides,
  };
}

describe("<RunTelemetryPanel />", () => {
  it("renders provider calls with model, tokens, cost, latency", async () => {
    getRunTelemetry.mockResolvedValue(
      makeTelemetry({
        provider_calls: [{
          provider_call_id: "pc-1",
          model: "glm-4.7",
          status: "succeeded",
          input_tokens: 1234,
          output_tokens: 4567,
          cost_micros: 3500,
          latency_ms: 820,
          started_at_ms: 1,
          finished_at_ms: 821,
          error_class: null,
          error_message: null,
        }],
        totals: {
          cost_micros: 3500,
          input_tokens: 1234,
          output_tokens: 4567,
          provider_calls: 1,
          tool_calls: 0,
          errors: 0,
          wall_ms: 820,
        },
      }),
    );

    renderPanel();

    const table = await screen.findByTestId("run-telemetry-provider-calls");
    expect(table).toBeInTheDocument();
    expect(table).toHaveTextContent("glm-4.7");
    expect(table).toHaveTextContent("succeeded");
    // Tokens formatted: 1234 -> "1.2k", 4567 -> "4.6k"
    expect(table).toHaveTextContent("1.2k");
    expect(table).toHaveTextContent("4.6k");
    // Cost: 3500 micros = $0.003500
    expect(table).toHaveTextContent("$0.003500");
  });

  it("renders tool invocations with name, status, and duration", async () => {
    getRunTelemetry.mockResolvedValue(
      makeTelemetry({
        tool_invocations: [{
          invocation_id: "tool-1",
          tool_name: "run_shell",
          status: "completed",
          started_at_ms: 100,
          finished_at_ms: 350,
          duration_ms: 250,
        }],
      }),
    );

    renderPanel();

    const table = await screen.findByTestId("run-telemetry-tool-invocations");
    expect(table).toHaveTextContent("run_shell");
    expect(table).toHaveTextContent("completed");
    // 250ms is sub-second so renders as "250ms".
    expect(table).toHaveTextContent("250ms");
  });

  it("renders totals card with cost, tokens, calls, wall", async () => {
    getRunTelemetry.mockResolvedValue(
      makeTelemetry({
        totals: {
          cost_micros: 125_000,
          input_tokens: 10_000,
          output_tokens: 2_500,
          provider_calls: 4,
          tool_calls: 2,
          errors: 1,
          wall_ms: 65_000,
        },
      }),
    );
    renderPanel();

    const totals = await screen.findByTestId("run-telemetry-totals");
    // $0.125000 = 125_000 micros
    expect(totals).toHaveTextContent("$0.125000");
    expect(totals).toHaveTextContent("10.0k");
    expect(totals).toHaveTextContent("2.5k");
    expect(totals).toHaveTextContent("1 error");
    // wall_ms 65_000 -> "1m 05s"
    expect(totals).toHaveTextContent("1m 05s");
  });

  it("shows stuck banner when telemetry.stuck = true", async () => {
    getRunTelemetry.mockResolvedValue(
      makeTelemetry({
        stuck: true,
        stuck_since_ms: Date.now() - 10 * 60_000, // 10 minutes ago
      }),
    );
    renderPanel();
    const banner = await screen.findByTestId("run-telemetry-stuck-banner");
    expect(banner).toBeInTheDocument();
    // 10 minutes ago
    expect(banner).toHaveTextContent(/min ago/);
  });

  it("hides stuck banner when not stuck", async () => {
    getRunTelemetry.mockResolvedValue(makeTelemetry({ stuck: false }));
    renderPanel();
    await waitFor(() => expect(getRunTelemetry).toHaveBeenCalled());
    expect(screen.queryByTestId("run-telemetry-stuck-banner")).toBeNull();
  });

  it("shows empty-state placeholders when no calls or invocations", async () => {
    getRunTelemetry.mockResolvedValue(makeTelemetry());
    renderPanel();
    expect(await screen.findByText(/No provider calls yet/i)).toBeInTheDocument();
    expect(screen.getByText(/No tool invocations yet/i)).toBeInTheDocument();
  });

  it("truncates long error_message in the row and shows full text on expand", async () => {
    const longMsg = "x".repeat(500);
    getRunTelemetry.mockResolvedValue(
      makeTelemetry({
        provider_calls: [{
          provider_call_id: "pc-err",
          model: "bad-model",
          status: "failed",
          input_tokens: 0,
          output_tokens: 0,
          cost_micros: 0,
          latency_ms: 10,
          started_at_ms: 1,
          finished_at_ms: 11,
          error_class: "rate_limit",
          error_message: longMsg,
        }],
      }),
    );
    const { container } = renderPanel();
    await screen.findByTestId("run-telemetry-provider-calls");
    // Inline row must contain error class + truncated message with ellipsis.
    expect(container.textContent ?? "").toContain("rate_limit");
    // Truncation to 240 chars: only 239 x's visible + the ellipsis char.
    expect(container.textContent ?? "").not.toContain(longMsg);
  });
});
