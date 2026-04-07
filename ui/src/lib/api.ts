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
import { getStoredScope } from "../hooks/useScope";

// ── Client config ─────────────────────────────────────────────────────────────

export interface ApiClientConfig {
  /** Base URL of the cairn-app server, e.g. "http://localhost:3000". */
  baseUrl: string;
  /** Bearer token for the admin account. */
  token: string;
  /**
   * Current tenant/workspace/project scope.  When set, list and create
   * endpoints automatically inject these values as defaults (explicit call-site
   * params always override).
   */
  scope?: import("../hooks/useScope").ProjectScope;
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
  const put  = <T>(path: string, body?: unknown) =>
    apiFetch<T>(config, path, {
      method: "PUT",
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  const del  = <T>(path: string) => apiFetch<T>(config, path, { method: "DELETE" });

  /**
   * Merge the configured scope (as defaults) with any explicit override params.
   * Explicit params always win; scope fills in undefined values only.
   */
  function withScope<T extends { tenant_id?: string; workspace_id?: string; project_id?: string }>(
    explicit?: T,
  ): { tenant_id?: string; workspace_id?: string; project_id?: string } & Omit<T, 'tenant_id' | 'workspace_id' | 'project_id'> {
    const s = config.scope;
    return {
      tenant_id:    s?.tenant_id,
      workspace_id: s?.workspace_id,
      project_id:   s?.project_id,
      ...explicit,
    } as { tenant_id?: string; workspace_id?: string; project_id?: string } & Omit<T, 'tenant_id' | 'workspace_id' | 'project_id'>;
  }

  return {
    // ── Health (public — no auth needed but token is included anyway) ─────────

    /** GET /health — liveness probe. */
    getHealth: (): Promise<HealthResponse> => get("/health"),

    // ── System status ─────────────────────────────────────────────────────────

    /** GET /v1/status — runtime + store health with uptime. */
    getStatus: (): Promise<SystemStatus> => get("/v1/status"),

    /** GET /v1/health/detailed — per-subsystem health with latency, memory, Ollama info. */
    getDetailedHealth: (): Promise<import("./types").DetailedHealth> => get("/v1/health/detailed"),

    /** GET /v1/system/info — version, build metadata, features, environment. */
    getSystemInfo: (): Promise<import("./types").SystemInfo> => get("/v1/system/info"),

    // ── Overview ─────────────────────────────────────────────────────────────

    /** GET /v1/overview — combined deployment info and health. */
    getOverview: (): Promise<OverviewResponse> => get("/v1/overview"),

    // ── Dashboard ─────────────────────────────────────────────────────────────

    /** GET /v1/dashboard — operator overview: runs, tasks, approvals, cost. */
    getDashboard: (): Promise<DashboardOverview> => get("/v1/dashboard"),

    // ── Sessions ──────────────────────────────────────────────────────────────

    /** GET /v1/sessions — list active sessions, most recent first. */
    getSessions: (params?: { limit?: number; offset?: number; tenant_id?: string; workspace_id?: string; project_id?: string }): Promise<SessionRecord[]> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)                  qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id)               qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)                 qs.set("project_id",   merged.project_id);
      if (params?.limit  !== undefined)      qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined)      qs.set("offset", String(params.offset));
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
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)             qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id)          qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)            qs.set("project_id",   merged.project_id);
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
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

    /** POST /v1/tasks/batch/cancel — cancel multiple tasks at once. */
    batchCancelTasks: (taskIds: string[]): Promise<{ cancelled: number; failed: { id: string; reason: string }[] }> =>
      post('/v1/tasks/batch/cancel', { task_ids: taskIds }),

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

    /** POST /v1/runs/batch — create multiple runs at once. */
    batchCreateRuns: (runs: Array<{
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      session_id?: string;
      run_id?: string;
    }>): Promise<{ results: Array<{ ok: boolean; run?: RunRecord; error?: string }> }> =>
      post('/v1/runs/batch', { runs }),

    // ── Approvals ─────────────────────────────────────────────────────────────

    /** GET /v1/approvals/pending — list pending approvals for operator inbox. */
    getPendingApprovals: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<ApprovalRecord[]> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)    qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id) qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)   qs.set("project_id",   merged.project_id);
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

    // ── API metrics ──────────────────────────────────────────────────────────

    /** GET /v1/metrics — rolling request metrics from the tracing middleware. */
    getMetrics: (): Promise<{
      total_requests:   number;
      requests_by_path: Record<string, number>;
      avg_latency_ms:   number;
      p50_latency_ms:   number;
      p95_latency_ms:   number;
      p99_latency_ms:   number;
      error_rate:       number;
      errors_by_status: Record<string, number>;
    }> => get("/v1/metrics"),

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
    getProviderHealth: (): Promise<import("./types").ProviderHealthEntry[]> => get("/v1/providers/health"),

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

    // ── Provider connections ─────────────────────────────────────────────────

    /** GET /v1/providers/connections — list registered provider connections. */
    listProviderConnections: (tenantId = "default"): Promise<{
      items: import("./types").ProviderConnectionRecord[];
      has_more: boolean;
    }> => get(`/v1/providers/connections?tenant_id=${encodeURIComponent(tenantId)}`),

    /** POST /v1/providers/connections — register a new provider connection. */
    createProviderConnection: (body: {
      tenant_id: string;
      provider_connection_id: string;
      provider_family: string;
      adapter_type: string;
      supported_models?: string[];
    }): Promise<import("./types").ProviderConnectionRecord> =>
      post("/v1/providers/connections", body),

    /** GET /v1/providers/connections/:id/models — list models for a connection. */
    listConnectionModels: (id: string): Promise<{ items: unknown[]; has_more: boolean }> =>
      get(`/v1/providers/connections/${encodeURIComponent(id)}/models`),

    // ── Default settings ─────────────────────────────────────────────────────

    /** PUT /v1/settings/defaults/:scope/:scopeId/:key — persist a tenant-level default. */
    setDefaultSetting: (scope: string, scopeId: string, key: string, value: unknown): Promise<unknown> =>
      put(`/v1/settings/defaults/${encodeURIComponent(scope)}/${encodeURIComponent(scopeId)}/${encodeURIComponent(key)}`, { value }),

    /** GET /v1/settings/defaults/resolve/:key — resolve effective default for a key.
     *  project must be "tenant/workspace/project" format, e.g. "default/default/default". */
    resolveDefaultSetting: (key: string, project = "default/default/default"): Promise<{ key: string; value: unknown } | null> =>
      get(`/v1/settings/defaults/resolve/${encodeURIComponent(key)}?project=${encodeURIComponent(project)}`),

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
    /** GET /v1/changelog — release notes array. Public endpoint. */
    getChangelog: (): Promise<import("./types").ChangelogEntry[]> =>
      get('/v1/changelog'),

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
      const merged = withScope(params);
      const s = config.scope;
      const qs = new URLSearchParams();
      qs.set("query_text",   params.query_text);
      qs.set("tenant_id",    merged.tenant_id    ?? s?.tenant_id    ?? "default_tenant");
      qs.set("workspace_id", merged.workspace_id ?? s?.workspace_id ?? "default_workspace");
      qs.set("project_id",   merged.project_id   ?? s?.project_id   ?? "default_project");
      if (params.limit !== undefined) qs.set("limit", String(params.limit));
      return get(`/v1/memory/search?${qs}`);
    },

    /** GET /v1/sources — list registered signal sources. */
    getSources: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").SourceRecord[]> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)    qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id) qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)   qs.set("project_id",   merged.project_id);
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

    // ── Credentials (RFC 011) ────────────────────────────────────────────────

    /**
     * GET /v1/admin/tenants/:tenantId/credentials
     * Returns credential metadata only — secrets are never returned.
     */
    getCredentials: (
      tenantId: string,
      params?: { limit?: number; offset?: number },
    ): Promise<import("./types").ListResponse<import("./types").CredentialSummary>> => {
      const qs = new URLSearchParams();
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/admin/tenants/${encodeURIComponent(tenantId)}/credentials${q}`);
    },

    /**
     * POST /v1/admin/tenants/:tenantId/credentials
     * Creates a new credential. The plaintext_value is transmitted once and
     * then encrypted at rest; it is never returned again.
     */
    storeCredential: (
      tenantId: string,
      body: import("./types").StoreCredentialRequest,
    ): Promise<import("./types").CredentialSummary> =>
      post(`/v1/admin/tenants/${encodeURIComponent(tenantId)}/credentials`, body),

    /**
     * DELETE /v1/admin/tenants/:tenantId/credentials/:id
     * Revokes (soft-deletes) a credential. Record is retained for audit history.
     */
    revokeCredential: (
      tenantId: string,
      credentialId: string,
    ): Promise<import("./types").CredentialSummary> =>
      del(`/v1/admin/tenants/${encodeURIComponent(tenantId)}/credentials/${encodeURIComponent(credentialId)}`),

    // ── Notification channels (RFC 007/014) ──────────────────────────────────

    /** GET /v1/admin/operators/:operatorId/notifications — fetch preferences for one operator. */
    getNotificationPreferences: (
      operatorId: string,
      tenantId = "default",
    ): Promise<import("./types").NotificationPreference> => {
      const qs = new URLSearchParams({ tenant_id: tenantId });
      return get(`/v1/admin/operators/${encodeURIComponent(operatorId)}/notifications?${qs}`);
    },

    /** POST /v1/admin/operators/:operatorId/notifications — create/replace preferences. */
    setNotificationPreferences: (
      operatorId: string,
      body: {
        tenant_id?: string;
        event_types: string[];
        channels: import("./types").NotificationChannel[];
      },
    ): Promise<{ ok: boolean }> =>
      post(`/v1/admin/operators/${encodeURIComponent(operatorId)}/notifications`, body),

    /** GET /v1/admin/notifications/failed — list failed delivery records. */
    getFailedNotifications: (tenantId = "default"): Promise<import("./types").ListResponse<import("./types").NotificationRecord>> => {
      const qs = new URLSearchParams({ tenant_id: tenantId });
      return get(`/v1/admin/notifications/failed?${qs}`);
    },

    /** POST /v1/admin/notifications/:id/retry — retry a failed delivery. */
    retryNotification: (recordId: string, tenantId = "default"): Promise<import("./types").NotificationRecord> => {
      const qs = new URLSearchParams({ tenant_id: tenantId });
      return post(`/v1/admin/notifications/${encodeURIComponent(recordId)}/retry?${qs}`, {});
    },

    /** POST /v1/notifications/send — dispatch an ad-hoc / test notification. */
    sendTestNotification: (
      tenantId: string,
      body: { event_type: string; message: string; severity?: string; operator_id?: string },
    ): Promise<{ dispatched: number; records: import("./types").NotificationRecord[] }> => {
      const qs = new URLSearchParams({ tenant_id: tenantId });
      return post(`/v1/notifications/send?${qs}`, body);
    },

    // ── Prompts (RFC 006) ────────────────────────────────────────────────────

    /** GET /v1/prompts/assets — list prompt assets. */
    getPromptAssets: (params?: { limit?: number; offset?: number }): Promise<import("./types").ListResponse<import("./types").PromptAssetRecord>> => {
      const qs = new URLSearchParams();
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/prompts/assets${q}`);
    },

    /** POST /v1/prompts/assets — create a new prompt asset. */
    createPromptAsset: (body: {
      prompt_asset_id: string;
      name: string;
      kind: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").PromptAssetRecord> =>
      post("/v1/prompts/assets", body),

    /** GET /v1/prompts/assets/:id/versions — version history. */
    getPromptVersions: (assetId: string, params?: { limit?: number }): Promise<import("./types").ListResponse<import("./types").PromptVersionRecord>> => {
      const qs = new URLSearchParams();
      if (params?.limit !== undefined) qs.set("limit", String(params.limit));
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/prompts/assets/${encodeURIComponent(assetId)}/versions${q}`);
    },

    /** POST /v1/prompts/assets/:id/versions — create a new version. */
    createPromptVersion: (assetId: string, body: {
      prompt_version_id: string;
      content_hash: string;
      content?: string;
      template_vars?: import("./types").PromptTemplateVar[];
    }): Promise<import("./types").PromptVersionRecord> =>
      post(`/v1/prompts/assets/${encodeURIComponent(assetId)}/versions`, body),

    /** GET /v1/prompts/assets/:id/versions/:vid/diff — diff two versions. */
    getVersionDiff: (assetId: string, versionId: string, compareTo: string): Promise<import("./types").PromptVersionDiff> =>
      get(`/v1/prompts/assets/${encodeURIComponent(assetId)}/versions/${encodeURIComponent(versionId)}/diff?compare_to=${encodeURIComponent(compareTo)}`),

    /** GET /v1/prompts/releases — list all releases. */
    getPromptReleases: (params?: { limit?: number; offset?: number }): Promise<import("./types").ListResponse<import("./types").PromptReleaseRecord>> => {
      const qs = new URLSearchParams();
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/prompts/releases${q}`);
    },

    /** POST /v1/prompts/releases — create a release from a version. */
    createPromptRelease: (body: {
      prompt_release_id: string;
      prompt_asset_id: string;
      prompt_version_id: string;
      release_tag?: string;
    }): Promise<import("./types").PromptReleaseRecord> =>
      post("/v1/prompts/releases", body),

    /** POST /v1/prompts/releases/:id/activate — activate a release. */
    activatePromptRelease: (releaseId: string): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/activate`, {}),

    /** POST /v1/prompts/releases/:id/rollout — set rollout percentage. */
    rolloutPromptRelease: (releaseId: string, percent: number): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/rollout`, { percent }),

    /** POST /v1/prompts/releases/:id/request-approval — request approval gate. */
    requestPromptReleaseApproval: (releaseId: string): Promise<unknown> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/request-approval`, {}),

    /** POST /v1/prompts/releases/:id/rollback — roll back to a previous release. */
    rollbackPromptRelease: (releaseId: string, targetReleaseId: string): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/rollback`, { target_release_id: targetReleaseId }),

    /** POST /v1/prompts/releases/:id/transition — generic state transition. */
    transitionPromptRelease: (releaseId: string, toState: string): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/transition`, { to_state: toState }),

    // ── Request logs ─────────────────────────────────────────────────────────

    /**
     * GET /v1/admin/logs — structured request log tail from the in-memory ring buffer.
     * Supports ?limit=N and ?level=info,warn,error filtering.
     */
    getRequestLogs: (params?: {
      limit?: number;
      level?: string;
    }): Promise<import("./types").RequestLogsResponse> => {
      const qs = new URLSearchParams();
      if (params?.limit  !== undefined) qs.set("limit", String(params.limit));
      if (params?.level)               qs.set("level", params.level);
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/admin/logs${q}`);
    },

    // ── Export / Import ───────────────────────────────────────────────────────

    /** GET /v1/runs/:id/export — download run + tasks + events as JSON blob. */
    exportRun: (runId: string): Promise<unknown> =>
      get(`/v1/runs/${encodeURIComponent(runId)}/export`),

    /** GET /v1/sessions/:id/export — download session + runs + tasks + events. */
    exportSession: (sessionId: string): Promise<unknown> =>
      get(`/v1/sessions/${encodeURIComponent(sessionId)}/export`),

    /** POST /v1/sessions/import — re-create a session from an export file. */
    importSession: (exportData: unknown): Promise<import("./types").SessionRecord> =>
      post("/v1/sessions/import", exportData),

    // ── Notifications ─────────────────────────────────────────────────────────

    /** GET /v1/notifications?limit=50 — list recent notifications with unread count. */
    getNotifications: (limit = 50): Promise<import("./types").NotifListResponse> =>
      get(`/v1/notifications?limit=${limit}`),

    /** POST /v1/notifications/:id/read — mark one notification as read. */
    markNotificationRead: (id: string): Promise<void> =>
      post(`/v1/notifications/${encodeURIComponent(id)}/read`, {}),

    /** POST /v1/notifications/read-all — mark all notifications as read. */
    markAllNotificationsRead: (): Promise<void> =>
      post("/v1/notifications/read-all", {}),
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
 * Dynamic default client: reads the token AND scope from localStorage on every
 * call so that post-login requests use the newly saved token without
 * re-importing, and scope changes are reflected immediately.
 */
export const defaultApi = new Proxy({} as ReturnType<typeof createApiClient>, {
  get(_target, prop) {
    const client = createApiClient({
      baseUrl: import.meta.env.VITE_API_URL ?? '',
      token:   getStoredToken(),
      scope:   getStoredScope(),
    });
    return (client as Record<string, unknown>)[prop as string];
  },
});

export type ApiClient = ReturnType<typeof createApiClient>;
