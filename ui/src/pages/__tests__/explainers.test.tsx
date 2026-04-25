/**
 * F32 page-explainer integration tests.
 *
 * For a representative set of entity pages, render the page against a
 * minimally-mocked data layer and assert that the entity explainer text
 * from `entityExplainers.ts` is present in the DOM. This prevents a
 * refactor from silently deleting the inline explainer.
 *
 * We do not mock every single page (the test matrix would be enormous);
 * instead we cover the five surfaces where confusing-with-a-sibling-entity
 * is most likely:
 *
 *   - SkillsPage (trivial surface — establishes the pattern)
 *   - RunsPage   (Run vs Task)
 *   - TasksPage  (Task vs Run)
 *   - ApprovalsPage (Approval vs Decision)
 *   - TriggersPage  (Trigger domain)
 *
 * All mocks use minimal shapes just sufficient to get the component past
 * its top-level loading/error branches so the explainer renders.
 */

import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, beforeEach, vi } from "vitest";

import { ENTITY_EXPLAINERS } from "../../lib/entityExplainers";
import { ToastProvider } from "../../components/Toast";

// ── Shared API mock ──────────────────────────────────────────────────────────
//
// Any method not explicitly listed resolves to an empty array or object so
// secondary queries fired by a page never blow up the test.

const mockApi = {
  listSkills:             vi.fn(),
  listRuns:               vi.fn(),
  getRuns:                vi.fn(),
  getAllTasks:            vi.fn(),
  getRunsStalled:         vi.fn(),
  getAllApprovals:        vi.fn(),
  listToolCallApprovals:  vi.fn(),
  listTriggers:           vi.fn(),
  listRunTemplates:       vi.fn(),
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

// Auto-refresh — page-level setIntervals confuse jsdom. Pin to a no-op.
vi.mock("../../hooks/useAutoRefresh", () => ({
  useAutoRefresh: () => ({
    ms: false,
    setOption: () => {},
    interval: { option: "off", label: "off", ms: false },
  }),
  REFRESH_OPTIONS: [{ option: "off", label: "off", ms: false }],
}));

// EventStream — SSE not wired in jsdom.
vi.mock("../../hooks/useEventStream", () => ({
  useEventStream: () => ({ events: [] }),
}));

// Scope hook — return a stable default without touching localStorage.
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

// ── Helpers ──────────────────────────────────────────────────────────────────

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
});

// ── Tests ────────────────────────────────────────────────────────────────────

describe("F32 inline entity explainers", () => {
  it("SkillsPage renders the skill explainer", async () => {
    mockApi.listSkills.mockResolvedValue({
      items: [],
      summary: { total: 0, enabled: 0, disabled: 0 },
      currently_active: [],
    });
    const { SkillsPage } = await import("../SkillsPage");
    renderPage(<SkillsPage />);
    await waitFor(() => {
      expect(screen.getByText(ENTITY_EXPLAINERS.skill)).toBeInTheDocument();
    });
  });

  it("RunsPage renders the runsList explainer", async () => {
    mockApi.getRuns.mockResolvedValue([]);
    mockApi.getRunsStalled.mockResolvedValue([]);
    const { RunsPage } = await import("../RunsPage");
    renderPage(<RunsPage />);
    await waitFor(() => {
      expect(screen.getByText(ENTITY_EXPLAINERS.runsList)).toBeInTheDocument();
    });
  });

  it("TasksPage renders the task explainer", async () => {
    mockApi.getAllTasks.mockResolvedValue([]);
    const { TasksPage } = await import("../TasksPage");
    renderPage(<TasksPage />);
    await waitFor(() => {
      expect(screen.getByText(ENTITY_EXPLAINERS.task)).toBeInTheDocument();
    });
  });

  it("ApprovalsPage renders the approval explainer (distinguishes from decisions)", async () => {
    mockApi.getAllApprovals.mockResolvedValue([]);
    mockApi.listToolCallApprovals.mockResolvedValue([]);
    const { ApprovalsPage } = await import("../ApprovalsPage");
    renderPage(<ApprovalsPage />);
    await waitFor(() => {
      expect(screen.getByText(ENTITY_EXPLAINERS.approval)).toBeInTheDocument();
    });
  });

  it("TriggersPage renders the trigger explainer", async () => {
    mockApi.listTriggers.mockResolvedValue([]);
    mockApi.listRunTemplates.mockResolvedValue([]);
    const { TriggersPage } = await import("../TriggersPage");
    renderPage(<TriggersPage />);
    await waitFor(() => {
      expect(screen.getByText(ENTITY_EXPLAINERS.trigger)).toBeInTheDocument();
    });
  });
});
