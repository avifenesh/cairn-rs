/**
 * Cairn API client for the React frontend.
 *
 * Usage:
 *   import { createApiClient } from './lib/api';
 *   const api = createApiClient({ baseUrl: 'http://localhost:3000', token: 'dev-admin-token' });
 *   const dashboard = await api.getDashboard();
 */

import type {
  ApprovalRecord,
  CostSummary,
  DashboardOverview,
  DeploymentSettings,
  HealthResponse,
  OverviewResponse,
  RunRecord,
  SessionRecord,
  SystemStatus,
} from "./types";

// ── Client config ─────────────────────────────────────────────────────────────

export interface ApiClientConfig {
  /** Base URL of the cairn-app server, e.g. "http://localhost:3000". */
  baseUrl: string;
  /** Bearer token for the admin account. */
  token: string;
}

// ── Error type ────────────────────────────────────────────────────────────────

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

// ── Base fetch wrapper ────────────────────────────────────────────────────────

/**
 * Core fetch wrapper that:
 *  - Injects `Authorization: Bearer <token>` on every request
 *  - Sets `Content-Type: application/json` for POST/PUT/PATCH bodies
 *  - Throws `ApiError` on non-2xx responses
 *  - Returns the parsed JSON body
 */
async function apiFetch<T>(
  config: ApiClientConfig,
  path: string,
  options: RequestInit = {}
): Promise<T> {
  const url = `${config.baseUrl}${path}`;

  const headers: HeadersInit = {
    Authorization: `Bearer ${config.token}`,
    ...(options.body ? { "Content-Type": "application/json" } : {}),
    ...options.headers,
  };

  const response = await fetch(url, { ...options, headers });

  if (!response.ok) {
    let code = "unknown_error";
    let message = `HTTP ${response.status}`;
    try {
      const err = await response.json();
      code = err.code ?? code;
      message = err.message ?? message;
    } catch {
      // ignore JSON parse failure — use defaults above
    }
    throw new ApiError(response.status, code, message);
  }

  // Handle empty bodies (e.g. 204 No Content)
  const text = await response.text();
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

// ── API client factory ────────────────────────────────────────────────────────

export function createApiClient(config: ApiClientConfig) {
  const get  = <T>(path: string) => apiFetch<T>(config, path, { method: "GET" });
  const post = <T>(path: string, body?: unknown) =>
    apiFetch<T>(config, path, {
      method: "POST",
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  const del  = <T>(path: string) => apiFetch<T>(config, path, { method: "DELETE" });

  return {
    // ── Health (public — no auth needed but token is included anyway) ─────────

    /** GET /health — liveness probe. */
    getHealth: (): Promise<HealthResponse> => get("/health"),

    // ── System status ─────────────────────────────────────────────────────────

    /** GET /v1/status — runtime + store health with uptime. */
    getStatus: (): Promise<SystemStatus> => get("/v1/status"),

    // ── Overview ─────────────────────────────────────────────────────────────

    /** GET /v1/overview — combined deployment info and health. */
    getOverview: (): Promise<OverviewResponse> => get("/v1/overview"),

    // ── Dashboard ─────────────────────────────────────────────────────────────

    /** GET /v1/dashboard — operator overview: runs, tasks, approvals, cost. */
    getDashboard: (): Promise<DashboardOverview> => get("/v1/dashboard"),

    // ── Sessions ──────────────────────────────────────────────────────────────

    /** GET /v1/sessions — list active sessions, most recent first. */
    getSessions: (params?: { limit?: number; offset?: number }): Promise<SessionRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.limit !== undefined) qs.set("limit", String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const query = qs.toString() ? `?${qs}` : "";
      return get(`/v1/sessions${query}`);
    },

    /** POST /v1/sessions — create a new session. */
    createSession: (body: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      session_id?: string;
    }): Promise<SessionRecord> => post("/v1/sessions", body),

    // ── Runs ──────────────────────────────────────────────────────────────────

    /** GET /v1/runs — list runs (filtered by project if params supplied). */
    getRuns: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      limit?: number;
      offset?: number;
    }): Promise<RunRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.tenant_id) qs.set("tenant_id", params.tenant_id);
      if (params?.workspace_id) qs.set("workspace_id", params.workspace_id);
      if (params?.project_id) qs.set("project_id", params.project_id);
      if (params?.limit !== undefined) qs.set("limit", String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const query = qs.toString() ? `?${qs}` : "";
      return get(`/v1/runs${query}`);
    },

    /** GET /v1/runs/:id — fetch a single run by ID. */
    getRun: (runId: string): Promise<RunRecord> => get(`/v1/runs/${runId}`),

    /** GET /v1/runs/:id/events — event timeline for a run. */
    getRunEvents: (runId: string, limit = 100): Promise<import("./types").RunEventSummary[]> =>
      get(`/v1/runs/${runId}/events?limit=${limit}`),

    /** GET /v1/tasks — all tasks across every project (operator view). */
    getAllTasks: (params?: { limit?: number; offset?: number }): Promise<import("./types").TaskRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/tasks${q}`);
    },

    /** POST /v1/tasks/:id/claim — claim a queued task for a worker. */
    claimTask: (taskId: string, workerId: string, leaseDurationMs = 30_000): Promise<import("./types").TaskRecord> =>
      post(`/v1/tasks/${taskId}/claim`, { worker_id: workerId, lease_duration_ms: leaseDurationMs }),

    /** POST /v1/tasks/:id/release-lease — release a leased task back to queued. */
    releaseLease: (taskId: string): Promise<import("./types").TaskRecord> =>
      post(`/v1/tasks/${taskId}/release-lease`),

    /** GET /v1/runs/:id/tasks — tasks belonging to a run. */
    getRunTasks: (runId: string): Promise<import("./types").TaskRecord[]> =>
      get(`/v1/runs/${runId}/tasks`),

    /** GET /v1/runs/:id/cost — accumulated cost for a run. */
    getRunCost: (runId: string): Promise<import("./types").RunCostRecord> =>
      get(`/v1/runs/${runId}/cost`),

    /** POST /v1/runs/:id/pause — pause a running run. */
    pauseRun: (runId: string, detail?: string): Promise<RunRecord> =>
      post(`/v1/runs/${runId}/pause`, { detail }),

    /** POST /v1/runs/:id/resume — resume a paused run. */
    resumeRun: (runId: string): Promise<RunRecord> =>
      post(`/v1/runs/${runId}/resume`, {}),

    /** POST /v1/runs — start a new run in a session. */
    createRun: (body: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      session_id?: string;
      run_id?: string;
      parent_run_id?: string;
    }): Promise<RunRecord> => post("/v1/runs", body),

    // ── Approvals ─────────────────────────────────────────────────────────────

    /** GET /v1/approvals/pending — list pending approvals for operator inbox. */
    getPendingApprovals: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<ApprovalRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.tenant_id) qs.set("tenant_id", params.tenant_id);
      if (params?.workspace_id) qs.set("workspace_id", params.workspace_id);
      if (params?.project_id) qs.set("project_id", params.project_id);
      const query = qs.toString() ? `?${qs}` : "";
      return get(`/v1/approvals/pending${query}`);
    },

    /** POST /v1/approvals/:id/resolve — approve or reject. */
    resolveApproval: (
      approvalId: string,
      decision: "approved" | "rejected"
    ): Promise<ApprovalRecord> =>
      post(`/v1/approvals/${approvalId}/resolve`, { decision }),

    // ── Costs ─────────────────────────────────────────────────────────────────

    /** GET /v1/costs — aggregate token and cost totals. */
    getCosts: (): Promise<CostSummary> => get("/v1/costs"),

    // ── Settings ─────────────────────────────────────────────────────────────

    /** GET /v1/settings — deployment configuration. */
    getSettings: (): Promise<DeploymentSettings> => get("/v1/settings"),

    /** GET /v1/events/recent — most recent N runtime events with sequence IDs. */
    getRecentEvents: (limit = 50): Promise<import("./types").RecentEvent[]> =>
      get(`/v1/events/recent?limit=${limit}`),

    /** GET /v1/stats — real-time system-wide counters. */
    getStats: (): Promise<import("./types").SystemStats> =>
      get("/v1/stats"),

    // ── Providers ────────────────────────────────────────────────────────────

    /** GET /v1/providers/health — list provider health records. */
    getProviderHealth: (): Promise<unknown[]> => get("/v1/providers/health"),

    /** GET /v1/providers/ollama/models — list locally available Ollama models. */
    getOllamaModels: (): Promise<{ host: string; models: string[]; count: number }> =>
      get("/v1/providers/ollama/models"),

    /** GET /v1/providers/ollama/models/:name/info — detailed info for one model. */
    getOllamaModelInfo: (name: string): Promise<{
      name: string;
      family: string;
      format: string;
      parameter_size: string;
      parameter_count: number | null;
      quantization_level: string;
      context_length: number | null;
      embedding_length: number | null;
      size_bytes: number | null;
      size_human: string;
    }> => get(`/v1/providers/ollama/models/${encodeURIComponent(name)}/info`),

    /** POST /v1/providers/ollama/pull — download a model into Ollama. */
    pullOllamaModel: (model: string): Promise<{ status: string; model: string }> =>
      post("/v1/providers/ollama/pull", { model }),

    /** POST /v1/providers/ollama/delete — remove a model from the local registry. */
    deleteOllamaModel: (model: string): Promise<{ status: string; model: string }> =>
      post("/v1/providers/ollama/delete", { model }),

    /** POST /v1/providers/ollama/generate — run a prompt through Ollama. */
    ollamaGenerate: (body: {
      prompt: string;
      model?: string;
    }): Promise<{
      text: string;
      model: string;
      tokens_in: number | null;
      tokens_out: number | null;
      latency_ms: number;
    }> => post("/v1/providers/ollama/generate", body),

    // ── LLM Traces ───────────────────────────────────────────────────────────

    /** GET /v1/traces — all recent LLM call traces (operator view). */
    getTraces: (limit = 500): Promise<import("./types").TracesResponse> =>
      get(`/v1/traces?limit=${limit}`),

    /** GET /v1/sessions/:id/llm-traces — traces for one session. */
    getSessionTraces: (sessionId: string, limit = 200): Promise<import("./types").TracesResponse> =>
      get(`/v1/sessions/${sessionId}/llm-traces?limit=${limit}`),

    // ── Evals ────────────────────────────────────────────────────────────────

    /** GET /v1/evals/runs — list eval runs (operator view). */
    getEvalRuns: (limit = 100): Promise<import("./types").EvalRunsResponse> =>
      get(`/v1/evals/runs?limit=${limit}`),

    // ── Audit Log ────────────────────────────────────────────────────────────

    /** GET /v1/admin/audit-log — list audit log entries (most recent first). */
    getAuditLog: (limit = 100): Promise<import("./types").AuditLogResponse> =>
      get(`/v1/admin/audit-log?limit=${limit}`),

    // ── Memory / Knowledge ───────────────────────────────────────────────────

    /** GET /v1/memory/search — lexical retrieval over the knowledge store. */
    searchMemory: (params: {
      query_text: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      limit?: number;
    }): Promise<import("./types").MemorySearchResponse> => {
      const qs = new URLSearchParams();
      qs.set("query_text",   params.query_text);
      qs.set("tenant_id",    params.tenant_id    ?? "default_tenant");
      qs.set("workspace_id", params.workspace_id ?? "default_workspace");
      qs.set("project_id",   params.project_id   ?? "default_project");
      if (params.limit !== undefined) qs.set("limit", String(params.limit));
      return get(`/v1/memory/search?${qs}`);
    },

    /** GET /v1/sources — list registered signal sources. */
    getSources: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").SourceRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.tenant_id)    qs.set("tenant_id",    params.tenant_id);
      if (params?.workspace_id) qs.set("workspace_id", params.workspace_id);
      if (params?.project_id)   qs.set("project_id",   params.project_id);
      const query = qs.toString() ? `?${qs}` : "";
      return get(`/v1/sources${query}`);
    },

    /** GET /v1/sources/:id/quality — quality metrics for a single source. */
    getSourceQuality: (sourceId: string): Promise<import("./types").SourceQualityRecord> =>
      get(`/v1/sources/${encodeURIComponent(sourceId)}/quality`),

    // ── Plugins ───────────────────────────────────────────────────────────────

    /** GET /v1/plugins — list all registered plugins. */
    getPlugins: (): Promise<import("./types").ListResponse<import("./types").PluginManifest>> =>
      get("/v1/plugins"),

    /** GET /v1/plugins/:id — full plugin detail with lifecycle + metrics. */
    getPlugin: (id: string): Promise<import("./types").PluginDetailResponse> =>
      get(`/v1/plugins/${encodeURIComponent(id)}`),

    /** POST /v1/plugins — register a new plugin from a manifest. */
    registerPlugin: (manifest: Record<string, unknown>): Promise<import("./types").PluginManifest> =>
      post("/v1/plugins", manifest),

    /** DELETE /v1/plugins/:id — unregister a plugin. */
    deletePlugin: (id: string): Promise<{ ok: boolean }> =>
      del(`/v1/plugins/${encodeURIComponent(id)}`),

    /** GET /v1/plugins/:id/logs — recent log entries for a plugin. */
    getPluginLogs: (id: string): Promise<{ entries: import("./types").PluginLogEntry[] }> =>
      get(`/v1/plugins/${encodeURIComponent(id)}/logs`),
  };
}

// ── Token persistence ─────────────────────────────────────────────────────────

export const TOKEN_KEY = 'cairn_token';

export function getStoredToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? import.meta.env.VITE_API_TOKEN ?? '';
}

export function setStoredToken(token: string) {
  localStorage.setItem(TOKEN_KEY, token);
}

export function clearStoredToken() {
  localStorage.removeItem(TOKEN_KEY);
}

/**
 * Dynamic default client: reads the token from localStorage on every call
 * so that post-login requests use the newly saved token without re-importing.
 */
export const defaultApi = new Proxy({} as ReturnType<typeof createApiClient>, {
  get(_target, prop) {
    const client = createApiClient({
      baseUrl: import.meta.env.VITE_API_URL ?? '',
      token: getStoredToken(),
    });
    return (client as Record<string, unknown>)[prop as string];
  },
});

export type ApiClient = ReturnType<typeof createApiClient>;
