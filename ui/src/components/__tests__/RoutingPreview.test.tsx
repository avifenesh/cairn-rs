/**
 * RoutingPreview tests — F29 CE.
 *
 * This is the highest-priority CE feature (it replaces the curl-debug
 * flow that blocked dogfood) — cover the three states:
 *
 *   - fully wired (connection advertises the model)
 *   - unwired (default set but no connection advertises it — warning)
 *   - unset (no default persisted — placeholder)
 */

import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, beforeEach, vi } from "vitest";

import type { ProviderConnectionRecord, SettingDefault } from "../../lib/types";

const listProviderConnections = vi.fn();
const getSettingsDefault = vi.fn();
vi.mock("../../lib/api", () => ({
  defaultApi: {
    listProviderConnections: (...args: unknown[]) => listProviderConnections(...args),
    getSettingsDefault:      (...args: unknown[]) => getSettingsDefault(...args),
  },
}));

import { RoutingPreview } from "../RoutingPreview";

function renderPreview() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <RoutingPreview />
    </QueryClientProvider>,
  );
}

function setting(key: string, value: unknown): SettingDefault {
  return { scope: "system", scope_id: "system", key, value, source: "system" };
}

function connection(id: string, models: string[], status: "active" | "disabled" = "active"): ProviderConnectionRecord {
  return {
    provider_connection_id: id,
    tenant_id:       "default_tenant",
    provider_family: "custom",
    adapter_type:    "openai-compatible",
    supported_models: models,
    status,
    created_at:      0,
  };
}

beforeEach(() => {
  listProviderConnections.mockReset();
  getSettingsDefault.mockReset();
});

describe("<RoutingPreview />", () => {
  it("shows the connection and model when a default maps to an active connection", async () => {
    listProviderConnections.mockResolvedValue({
      items: [
        connection("zai_coding", ["glm-4.7"]),
        connection("openrouter", ["minimax-m2.5:free"]),
      ],
      has_more: false,
    });
    getSettingsDefault.mockImplementation(async (_scope: string, _scopeId: string, key: string) => {
      if (key === "brain_model")    return setting("brain_model",    "glm-4.7");
      if (key === "generate_model") return setting("generate_model", "minimax-m2.5:free");
      return null;
    });

    renderPreview();

    await waitFor(() => {
      const brain = screen.getByTestId("routing-row-brain_model");
      expect(brain.dataset.status).toBe("ok");
    });
    const brain = screen.getByTestId("routing-row-brain_model");
    expect(brain).toHaveTextContent("zai_coding");
    expect(brain).toHaveTextContent("glm-4.7");

    await waitFor(() => {
      const gen = screen.getByTestId("routing-row-generate_model");
      expect(gen.dataset.status).toBe("ok");
    });
    const gen = screen.getByTestId("routing-row-generate_model");
    expect(gen).toHaveTextContent("openrouter");
    expect(gen).toHaveTextContent("minimax-m2.5:free");
  });

  it("warns when a default model is set but no active connection advertises it", async () => {
    listProviderConnections.mockResolvedValue({
      items: [connection("openrouter", ["different-model"])],
      has_more: false,
    });
    getSettingsDefault.mockImplementation(async (_scope: string, _scopeId: string, key: string) => {
      if (key === "brain_model") return setting("brain_model", "glm-4.7");
      return null;
    });

    renderPreview();

    await waitFor(() => {
      const brain = screen.getByTestId("routing-row-brain_model");
      expect(brain.dataset.status).toBe("unwired");
    });
    const brain = screen.getByTestId("routing-row-brain_model");
    expect(screen.getByTestId("routing-warn-brain_model")).toHaveTextContent(
      "no active provider connection",
    );
    expect(brain).toHaveTextContent("glm-4.7");
  });

  it("ignores disabled connections (still warns when only disabled advertise the model)", async () => {
    listProviderConnections.mockResolvedValue({
      items: [connection("openai_old", ["glm-4.7"], "disabled")],
      has_more: false,
    });
    getSettingsDefault.mockImplementation(async (_scope: string, _scopeId: string, key: string) =>
      key === "brain_model" ? setting("brain_model", "glm-4.7") : null,
    );

    renderPreview();

    const brain = await screen.findByTestId("routing-row-brain_model");
    await waitFor(() => expect(brain.dataset.status).toBe("unwired"));
  });

  it("surfaces error status (not 'not configured') when settings fetch fails", async () => {
    // Bugbot-flagged regression: a non-404 settings error used to hide
    // behind the "unset" placeholder, misleading operators whose
    // default was in fact persisted but temporarily unreadable.
    listProviderConnections.mockResolvedValue({ items: [], has_more: false });
    getSettingsDefault.mockImplementation(async (_scope: string, _scopeId: string, key: string) => {
      if (key === "brain_model") throw new Error("500 internal error");
      return null;
    });

    renderPreview();

    await waitFor(() => {
      const brain = screen.getByTestId("routing-row-brain_model");
      expect(brain.dataset.status).toBe("error");
    });
    expect(screen.getByTestId("routing-error-brain_model")).toHaveTextContent(
      "failed to read default",
    );
  });

  it("shows placeholder when no default is set", async () => {
    listProviderConnections.mockResolvedValue({ items: [], has_more: false });
    getSettingsDefault.mockResolvedValue(null);

    renderPreview();

    const brain = await screen.findByTestId("routing-row-brain_model");
    expect(brain.dataset.status).toBe("unset");
    expect(screen.getByTestId("routing-unset-brain_model")).toHaveTextContent("not configured");
  });
});
