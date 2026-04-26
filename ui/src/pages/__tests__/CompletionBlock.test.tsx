/**
 * F47 PR3: "What actually happened" block tests.
 *
 * Renders RunDetailPage against a minimally-mocked data layer and asserts
 * the completion block behaves across three cases:
 *   1. completion: null            → block absent
 *   2. clean run (scanned > 0,
 *      all arrays empty)           → positive affirmation
 *   3. warnings + errors + commands populated → all three sections render
 */

import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, beforeEach, vi } from "vitest";

import type { RunCompletion, RunRecord } from "../../lib/types";
import { ToastProvider } from "../../components/Toast";

const baseRun: RunRecord = {
  run_id: "run-test",
  session_id: "sess-test",
  state: "completed",
  project: {
    tenant_id: "default_tenant",
    workspace_id: "default_workspace",
    project_id: "default_project",
  },
  created_at: 1_700_000_000_000,
  updated_at: 1_700_000_010_000,
} as unknown as RunRecord;

const mockApi = {
  getRun: vi.fn(),
  getRunDetail: vi.fn(),
  getRunTasks: vi.fn(),
  getRunEvents: vi.fn(),
  getRunCost: vi.fn(),
  getRunTelemetry: vi.fn(),
  exportRun: vi.fn(),
  cancelRun: vi.fn(),
};

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    defaultApi: new Proxy({} as Record<string, unknown>, {
      get: (_t, prop: string) => {
        if (prop in mockApi) return (mockApi as Record<string, unknown>)[prop];
        return (..._args: unknown[]) => Promise.resolve([]);
      },
    }),
  };
});

vi.mock("../../hooks/useEventStream", () => ({
  useEventStream: () => ({ events: [] }),
}));

vi.mock("../../hooks/useScope", async () => {
  const actual = await vi.importActual<typeof import("../../hooks/useScope")>(
    "../../hooks/useScope",
  );
  const scope = {
    tenant_id: "default_tenant",
    workspace_id: "default_workspace",
    project_id: "default_project",
  };
  return {
    ...actual,
    useScope: () => [scope, () => {}, () => {}] as const,
    getStoredScope: () => scope,
    isDefaultScope: () => true,
  };
});

function renderPage(node: React.ReactNode) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <ToastProvider>{node}</ToastProvider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  for (const fn of Object.values(mockApi)) fn.mockReset();
  mockApi.getRunTasks.mockResolvedValue([]);
  mockApi.getRunEvents.mockResolvedValue([]);
  mockApi.getRunCost.mockResolvedValue(null);
  // Telemetry panel requires a shaped object; make it fail fast so the panel
  // renders its error branch instead of crashing on undefined totals.
  mockApi.getRunTelemetry.mockRejectedValue(Object.assign(new Error("not found"), { status: 404 }));
});

describe("F47 PR3 CompletionBlock", () => {
  it("renders nothing when completion is null", async () => {
    mockApi.getRunDetail.mockResolvedValue({
      run: baseRun,
      tasks: [],
      completion: null,
    });
    const { RunDetailPage } = await import("../RunDetailPage");
    renderPage(<RunDetailPage runId="run-test" />);
    await waitFor(() => {
      expect(screen.getByText(/Back to Runs/i)).toBeInTheDocument();
    });
    expect(screen.queryByText(/What actually happened/i)).not.toBeInTheDocument();
  });

  it("renders the clean-run affirmation when scanned > 0 and all arrays empty", async () => {
    const completion: RunCompletion = {
      summary: "Upgraded dependencies.",
      verification: {
        warnings: [],
        errors: [],
        commands: [],
        tool_results_scanned: 3,
        extractor_version: 1,
      },
      completed_at: 1_700_000_010_000,
    };
    mockApi.getRunDetail.mockResolvedValue({
      run: baseRun,
      tasks: [],
      completion,
    });
    const { RunDetailPage } = await import("../RunDetailPage");
    renderPage(<RunDetailPage runId="run-test" />);
    await waitFor(() => {
      expect(screen.getByText(/What actually happened/i)).toBeInTheDocument();
    });
    expect(
      screen.getByText(/No warnings or errors captured from 3 tool results/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/Upgraded dependencies\./)).toBeInTheDocument();
  });

  it("renders all three subsections with counts + entries when populated", async () => {
    const completion: RunCompletion = {
      summary: "Build succeeded.",
      verification: {
        warnings: ["warning: unused variable `x`"],
        errors: ["error[E0308]: mismatched types"],
        commands: [
          { tool_name: "bash", cmd: "cargo build --workspace", exit_code: 0 },
          { tool_name: "bash", cmd: "cargo test", exit_code: null },
        ],
        tool_results_scanned: 2,
        extractor_version: 1,
      },
      completed_at: 1_700_000_010_000,
    };
    mockApi.getRunDetail.mockResolvedValue({
      run: baseRun,
      tasks: [],
      completion,
    });
    const { RunDetailPage } = await import("../RunDetailPage");
    renderPage(<RunDetailPage runId="run-test" />);
    await waitFor(() => {
      expect(screen.getByText("Warnings")).toBeInTheDocument();
    });
    expect(screen.getByText("Errors")).toBeInTheDocument();
    expect(screen.getByText("Commands")).toBeInTheDocument();
    expect(screen.getByText(/warning: unused variable/)).toBeInTheDocument();
    expect(screen.getByText(/error\[E0308\]/)).toBeInTheDocument();
    expect(screen.getByText(/cargo build --workspace/)).toBeInTheDocument();
    // `null` exit_code renders as em-dash cell (there are other em-dashes on the
    // page for missing cost/tasks, so just confirm at least one is present).
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
    // Footer
    expect(screen.getByText(/Scanned 2 tool results · extractor v1/)).toBeInTheDocument();
  });
});
