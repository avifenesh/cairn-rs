/**
 * StuckRunsWidget tests — F29 CE.
 *
 * 1. Renders the count and a top-5 list when runs are stalled.
 * 2. Renders nothing (hidden) when the stalled list is empty.
 * 3. Row click navigates to #run/:id.
 */

import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, beforeEach, vi } from "vitest";

import type { StuckRunReport } from "../../lib/types";

const getStalledRuns = vi.fn();
vi.mock("../../lib/api", () => ({
  defaultApi: {
    getStalledRuns: (...args: unknown[]) => getStalledRuns(...args),
  },
}));

import { StuckRunsWidget } from "../StuckRunsWidget";

function renderWidget() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <StuckRunsWidget />
    </QueryClientProvider>,
  );
}

function report(run_id: string, overrides: Partial<StuckRunReport> = {}): StuckRunReport {
  return {
    run_id,
    state: "running",
    duration_ms: 45 * 60_000,
    active_tasks: [],
    stalled_tasks: [],
    last_event_type: "task_started",
    last_event_ms: Date.now() - 30 * 60_000,
    suggested_action: "observe",
    ...overrides,
  };
}

beforeEach(() => {
  getStalledRuns.mockReset();
  // Clean hash between tests so row-click assertions don't leak.
  window.location.hash = "";
});

describe("<StuckRunsWidget />", () => {
  it("renders count + top 5 stalled runs", async () => {
    const reports = Array.from({ length: 8 }, (_, i) =>
      report(`run-${i}`, { last_event_ms: Date.now() - (i + 5) * 60_000 }),
    );
    getStalledRuns.mockResolvedValue(reports);

    renderWidget();

    const widget = await screen.findByTestId("stuck-runs-widget");
    expect(widget).toBeInTheDocument();
    // Count shows 8.
    expect(await screen.findByTestId("stuck-runs-count")).toHaveTextContent("8");
    // Only top 5 rows render.
    const rows = await screen.findAllByTestId("stuck-runs-row");
    expect(rows).toHaveLength(5);
    // "+ 3 more" footer shows.
    expect(widget).toHaveTextContent("+ 3 more");
  });

  it("hides entirely when count is zero", async () => {
    getStalledRuns.mockResolvedValue([]);
    renderWidget();
    await waitFor(() => expect(getStalledRuns).toHaveBeenCalled());
    expect(screen.queryByTestId("stuck-runs-widget")).toBeNull();
  });

  it("navigates to RunDetailPage on row click", async () => {
    getStalledRuns.mockResolvedValue([report("abc123")]);
    renderWidget();
    const row = await screen.findByTestId("stuck-runs-row");
    fireEvent.click(row);
    expect(window.location.hash).toBe("#run/abc123");
  });

  it("hides the '+N more' footer when there are 5 or fewer runs", async () => {
    getStalledRuns.mockResolvedValue([report("a"), report("b")]);
    renderWidget();
    const widget = await screen.findByTestId("stuck-runs-widget");
    expect(widget).not.toHaveTextContent("+ ");
  });
});
