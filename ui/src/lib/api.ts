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

type RunModeRequest =
  | { type: "direct" }
  | { type: "plan" }
  | { type: "execute"; plan_run_id: string };

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

// ── List response unwrapper ───────────────────────────────────────────────────

/**
 * The server may return list endpoints as either a plain `T[]` array or wrapped
 * in `{ items: T[] }`.  This helper normalises both shapes into a plain array.
 */
function unwrapList<T>(data: unknown): T[] {
  if (Array.isArray(data)) return data as T[];
  if (data && typeof data === 'object' && 'items' in data && Array.isArray((data as { items: unknown }).items)) {
    return (data as { items: T[] }).items;
  }
  return [];
}

/**
 * The server may return a run endpoint as either a bare `RunRecord` or an
 * envelope such as `{ run, tasks }`. Normalize both into the canonical run.
 */
function unwrapRun(data: unknown): RunRecord {
  if (data && typeof data === "object" && "run" in data) {
    return (data as { run: RunRecord }).run;
  }
  return data as RunRecord;
}

// ── Prometheus text parser ────────────────────────────────────────────────────

/**
 * Parse Prometheus exposition format text into a MetricsSnapshot object.
 *
 * The cairn backend emits lines like:
 *   http_requests_total{method="GET",path="/v1/runs",status="200"} 42
 *   http_request_duration_ms_sum{method="GET",path="/v1/runs"} 1234
 *   http_request_duration_ms_count{method="GET",path="/v1/runs"} 42
 *   http_request_duration_ms_bucket{method="GET",path="/v1/runs",le="100"} 40
 *   active_runs_total 3
 *   active_tasks_total 7
 */
function parsePrometheusMetrics(text: string): {
  total_requests: number;
  requests_by_path: Record<string, number>;
  avg_latency_ms: number;
  p50_latency_ms: number;
  p95_latency_ms: number;
  p99_latency_ms: number;
  error_rate: number;
  errors_by_status: Record<string, number>;
} {
  let totalRequests = 0;
  const requestsByPath: Record<string, number> = {};
  let totalErrors = 0;
  const errorsByStatus: Record<string, number> = {};

  // For latency: accumulate sum and count across all paths to compute avg.
  // For percentile approximation from histogram buckets, track per-path data.
  let globalDurationSum = 0;
  let globalDurationCount = 0;

  // Histogram buckets keyed by path: { le_value => cumulative_count }
  const bucketsByPath: Record<string, { le: number; count: number }[]> = {};

  function parseLabels(labelsStr: string | undefined): Record<string, string> {
    const labels: Record<string, string> = {};
    if (!labelsStr) return labels;
    for (const pair of labelsStr.split(',')) {
      const eqIdx = pair.indexOf('=');
      if (eqIdx > 0) {
        const k = pair.slice(0, eqIdx).trim();
        let v = pair.slice(eqIdx + 1).trim();
        if (v.startsWith('"') && v.endsWith('"')) v = v.slice(1, -1);
        labels[k] = v;
      }
    }
    return labels;
  }

  for (const line of text.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;

    const match = trimmed.match(/^([a-zA-Z_:][a-zA-Z0-9_:]*)(?:\{([^}]*)\})?\s+(.+)$/);
    if (!match) continue;

    const [, metricName, labelsStr, valueStr] = match;
    const value = parseFloat(valueStr);
    if (Number.isNaN(value)) continue;

    const labels = parseLabels(labelsStr);

    // http_requests_total{method,path,status} — aggregate by path; track errors.
    if (metricName === 'http_requests_total' || metricName === 'cairn_http_requests_total') {
      totalRequests += value;
      if (labels.path) {
        requestsByPath[labels.path] = (requestsByPath[labels.path] ?? 0) + value;
      }
      const status = Number(labels.status);
      if (status >= 400) {
        totalErrors += value;
        const statusKey = String(status);
        errorsByStatus[statusKey] = (errorsByStatus[statusKey] ?? 0) + value;
      }
    }

    // http_request_duration_ms_sum{method,path}
    // Also accept the `cairn_`-prefixed form so percentiles keep working if
    // the backend uniformly namespaces metrics (parallels the dual-match on
    // `http_requests_total` / `cairn_http_requests_total` above).
    if (metricName === 'http_request_duration_ms_sum' || metricName === 'cairn_http_request_duration_ms_sum') {
      globalDurationSum += value;
    }

    // http_request_duration_ms_count{method,path}
    if (metricName === 'http_request_duration_ms_count' || metricName === 'cairn_http_request_duration_ms_count') {
      globalDurationCount += value;
    }

    // http_request_duration_ms_bucket{method,path,le}
    if ((metricName === 'http_request_duration_ms_bucket' || metricName === 'cairn_http_request_duration_ms_bucket') && labels.le) {
      const pathKey = labels.path ?? '_all';
      if (!bucketsByPath[pathKey]) bucketsByPath[pathKey] = [];
      const le = labels.le === '+Inf' ? Infinity : parseFloat(labels.le);
      bucketsByPath[pathKey].push({ le, count: value });
    }

    // active_runs_total / active_tasks_total — gauges (no labels).
    if (metricName === 'active_runs_total' || metricName === 'cairn_active_runs_total') {
      requestsByPath['active_runs (gauge)'] = value;
    }
    if (metricName === 'active_tasks_total' || metricName === 'cairn_active_tasks_total') {
      requestsByPath['active_tasks (gauge)'] = value;
    }
  }

  // Compute avg latency from sum/count.
  const avgLatency = globalDurationCount > 0
    ? Math.round(globalDurationSum / globalDurationCount)
    : 0;

  // Approximate percentiles from merged histogram buckets.
  // Merge all per-path buckets into one global histogram.
  const globalBuckets: Record<number, number> = {};
  for (const pathBuckets of Object.values(bucketsByPath)) {
    for (const { le, count } of pathBuckets) {
      globalBuckets[le] = (globalBuckets[le] ?? 0) + count;
    }
  }
  const sortedLe = Object.keys(globalBuckets)
    .map(Number)
    .filter(n => Number.isFinite(n))
    .sort((a, b) => a - b);

  function percentileFromBuckets(target: number): number {
    if (sortedLe.length === 0 || globalDurationCount === 0) return 0;
    const threshold = target * globalDurationCount;
    for (const le of sortedLe) {
      if (globalBuckets[le] >= threshold) return le;
    }
    return sortedLe[sortedLe.length - 1] ?? 0;
  }

  const p50 = percentileFromBuckets(0.5);
  const p95 = percentileFromBuckets(0.95);
  const p99 = percentileFromBuckets(0.99);

  const errorRate = totalRequests > 0 ? totalErrors / totalRequests : 0;

  return {
    total_requests: totalRequests,
    requests_by_path: requestsByPath,
    avg_latency_ms: avgLatency,
    p50_latency_ms: p50,
    p95_latency_ms: p95,
    p99_latency_ms: p99,
    error_rate: errorRate,
    errors_by_status: errorsByStatus,
  };
}

// ── API client factory ────────────────────────────────────────────────────────

export function createApiClient(config: ApiClientConfig) {
  const get  = <T>(path: string) => apiFetch<T>(config, path, { method: "GET" });
  /** GET that unwraps list responses into a plain array. */
  const getList = <T>(path: string) => apiFetch<unknown>(config, path, { method: "GET" }).then(unwrapList<T>);
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

    /** GET /v1/health/detailed — per-subsystem health with latency and memory. */
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
    getSessions: (params?: {
      limit?: number;
      offset?: number;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      inherit_scope?: boolean;
    }): Promise<SessionRecord[]> => {
      const merged = params?.inherit_scope === false ? (params ?? {}) : withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)                  qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id)               qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)                 qs.set("project_id",   merged.project_id);
      if (params?.limit  !== undefined)      qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined)      qs.set("offset", String(params.offset));
      const query = qs.toString() ? `?${qs}` : "";
      return getList(`/v1/sessions${query}`);
    },

    /** POST /v1/sessions — create a new session. */
    createSession: (body: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      session_id?: string;
    }): Promise<SessionRecord> => post("/v1/sessions", withScope(body)),

    // ── Runs ──────────────────────────────────────────────────────────────────

    /** GET /v1/runs — list runs (filtered by project if params supplied). */
    getRuns: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      limit?: number;
      offset?: number;
      inherit_scope?: boolean;
    }): Promise<RunRecord[]> => {
      const merged = params?.inherit_scope === false ? (params ?? {}) : withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)             qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id)          qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)            qs.set("project_id",   merged.project_id);
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const query = qs.toString() ? `?${qs}` : "";
      return getList(`/v1/runs${query}`);
    },

    /** GET /v1/runs/:id — fetch a single run by ID. */
    getRun: async (runId: string): Promise<RunRecord> => {
      const raw = await get<RunRecord | { run: RunRecord; tasks?: import("./types").TaskRecord[] }>(
        `/v1/runs/${encodeURIComponent(runId)}`,
      );
      return unwrapRun(raw);
    },

    // ── Workspaces ────────────────────────────────────────────────────────────

    /** GET /v1/admin/tenants/:tenant_id/workspaces — list persisted workspaces for one tenant. */
    getWorkspaces: (
      tenantId: string,
      params?: { limit?: number; offset?: number },
    ): Promise<import("./types").WorkspaceRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.limit !== undefined) qs.set("limit", String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const query = qs.toString() ? `?${qs}` : "";
      return getList(`/v1/admin/tenants/${encodeURIComponent(tenantId)}/workspaces${query}`);
    },

    /** POST /v1/admin/tenants/:tenant_id/workspaces — create a persisted workspace. */
    createWorkspace: (
      tenantId: string,
      body: { workspace_id: string; name: string },
    ): Promise<import("./types").WorkspaceRecord> =>
      post(`/v1/admin/tenants/${encodeURIComponent(tenantId)}/workspaces`, body),

    /** GET /v1/runs/:id/events — event timeline for a run. */
    getRunEvents: async (runId: string, limit = 100): Promise<import("./types").RunEventSummary[]> => {
      let raw: unknown;
      try {
        raw = await get<unknown>(`/v1/runs/${encodeURIComponent(runId)}/events?limit=${limit}`);
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return [];
        throw e;
      }
      // The backend returns { events: [...], next_cursor, has_more } (EventsPage)
      // unless the legacy `from` param is used.  Normalise both shapes.
      let arr: Record<string, unknown>[];
      if (Array.isArray(raw)) {
        arr = raw;
      } else if (raw && typeof raw === 'object' && 'events' in raw && Array.isArray((raw as { events: unknown }).events)) {
        arr = (raw as { events: Record<string, unknown>[] }).events;
      } else {
        return [];
      }
      // Backend sends `occurred_at_ms`; UI expects `stored_at`.  Normalise.
      return arr.map(e => ({
        ...e,
        stored_at: (e.stored_at ?? e.occurred_at_ms ?? 0) as number,
      })) as import("./types").RunEventSummary[];
    },

    /** GET /v1/tasks — all tasks across every project (operator view). */
    getAllTasks: (params?: { limit?: number; offset?: number; tenant_id?: string; workspace_id?: string; project_id?: string }): Promise<import("./types").TaskRecord[]> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)             qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id)          qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)            qs.set("project_id",   merged.project_id);
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const q = qs.toString() ? `?${qs}` : "";
      return getList(`/v1/tasks${q}`);
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
      getList(`/v1/runs/${encodeURIComponent(runId)}/tasks`),

    /** GET /v1/runs/:id/cost — accumulated cost for a run.  Returns null when no cost data exists (404). */
    getRunCost: async (runId: string): Promise<import("./types").RunCostRecord | null> => {
      try {
        return await get<import("./types").RunCostRecord>(`/v1/runs/${encodeURIComponent(runId)}/cost`);
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }
    },

    /** POST /v1/runs/:id/cancel — cancel a run. */
    cancelRun: async (runId: string): Promise<RunRecord> =>
      unwrapRun(await post<RunRecord | { run: RunRecord }>(`/v1/runs/${encodeURIComponent(runId)}/cancel`, {})),

    /** POST /v1/runs/:id/pause — pause a running run. */
    pauseRun: async (runId: string, detail?: string): Promise<RunRecord> =>
      unwrapRun(await post<RunRecord | { run: RunRecord }>(`/v1/runs/${encodeURIComponent(runId)}/pause`, { detail })),

    /** POST /v1/runs/:id/resume — resume a paused run. */
    resumeRun: async (runId: string): Promise<RunRecord> =>
      unwrapRun(await post<RunRecord | { run: RunRecord }>(`/v1/runs/${encodeURIComponent(runId)}/resume`, {})),

    /** POST /v1/runs — start a new run in a session. */
    createRun: (body: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      session_id?: string;
      run_id?: string;
      parent_run_id?: string;
      mode?: RunModeRequest;
    }): Promise<RunRecord> => post("/v1/runs", withScope(body)),

    /** POST /v1/runs/batch — create multiple runs at once. */
    batchCreateRuns: (runs: Array<{
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      session_id?: string;
      run_id?: string;
      mode?: RunModeRequest;
    }>): Promise<{ results: Array<{ ok: boolean; run?: RunRecord; error?: string }> }> =>
      post('/v1/runs/batch', { runs: runs.map((run) => withScope(run)) }),

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
      return getList(`/v1/approvals/pending${query}`);
    },

    /** GET /v1/approvals — list all approvals (pending + resolved). */
    getAllApprovals: (params?: {
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
      return getList(`/v1/approvals${query}`);
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

    /** GET /v1/metrics — rolling request metrics from the tracing middleware.
     *  The backend may return JSON or Prometheus text/plain format.
     *  This method handles both and normalises into a MetricsSnapshot object.
     */
    getMetrics: async (): Promise<{
      total_requests:   number;
      requests_by_path: Record<string, number>;
      avg_latency_ms:   number;
      p50_latency_ms:   number;
      p95_latency_ms:   number;
      p99_latency_ms:   number;
      error_rate:       number;
      errors_by_status: Record<string, number>;
    }> => {
      const url = `${config.baseUrl}/v1/metrics`;
      const response = await fetch(url, {
        method: "GET",
        headers: { Authorization: `Bearer ${config.token}` },
      });
      if (!response.ok) {
        throw new ApiError(response.status, "metrics_error", `HTTP ${response.status}`);
      }
      const contentType = response.headers.get("content-type") ?? "";
      const text = await response.text();
      if (!text) {
        throw new ApiError(500, "empty_response", "empty metrics response");
      }

      // If the response is JSON, parse directly.
      if (contentType.includes("application/json")) {
        return JSON.parse(text);
      }

      // Otherwise parse Prometheus text exposition format into MetricsSnapshot.
      return parsePrometheusMetrics(text);
    },

    // ── Settings ─────────────────────────────────────────────────────────────

    /** GET /v1/settings — deployment configuration. */
    getSettings: (): Promise<DeploymentSettings> => get("/v1/settings"),

    /** GET /v1/events/recent — most recent N runtime events with sequence IDs. */
    getRecentEvents: (limit = 50): Promise<import("./types").RecentEvent[]> =>
      getList(`/v1/events/recent?limit=${limit}`),

    /** GET /v1/stats — real-time system-wide counters. */
    getStats: (): Promise<import("./types").SystemStats> =>
      get("/v1/stats"),

    // ── Providers ────────────────────────────────────────────────────────────

    /** GET /v1/providers/health — list provider health records. */
    getProviderHealth: (): Promise<import("./types").ProviderHealthEntry[]> => {
      const s = withScope();
      const qs = new URLSearchParams();
      if (s.tenant_id)    qs.set("tenant_id",    s.tenant_id);
      if (s.workspace_id) qs.set("workspace_id", s.workspace_id);
      if (s.project_id)   qs.set("project_id",   s.project_id);
      return getList(`/v1/providers/health?${qs}`);
    },

    /** GET /v1/providers/registry — live provider registry state plus static catalog. */
    getProviderRegistry: async (): Promise<import("./types").ProviderRegistryEntry[]> => {
      const response = await get<
        | import("./types").ProviderRegistryEntry[]
        | { catalog?: import("./types").ProviderRegistryEntry[] }
      >("/v1/providers/registry");
      return Array.isArray(response) ? response : (response.catalog ?? []);
    },

    // ── Provider connections ─────────────────────────────────────────────────

    /** GET /v1/providers/connections — list registered provider connections. */
    listProviderConnections: (): Promise<{
      items: import("./types").ProviderConnectionRecord[];
      has_more: boolean;
    }> => {
      const s = withScope();
      const qs = new URLSearchParams();
      if (s.tenant_id)    qs.set("tenant_id",    s.tenant_id);
      if (s.workspace_id) qs.set("workspace_id", s.workspace_id);
      if (s.project_id)   qs.set("project_id",   s.project_id);
      return get(`/v1/providers/connections?${qs}`);
    },

    /** POST /v1/providers/connections — register a new provider connection. */
    createProviderConnection: (body: {
      tenant_id: string;
      provider_connection_id: string;
      provider_family: string;
      adapter_type: string;
      supported_models?: string[];
      credential_id?: string;
      endpoint_url?: string;
    }): Promise<import("./types").ProviderConnectionRecord> =>
      post("/v1/providers/connections", body),

    /** PUT /v1/providers/connections/:id — update a provider connection. */
    updateProviderConnection: (id: string, body: Record<string, unknown>): Promise<unknown> =>
      put(`/v1/providers/connections/${encodeURIComponent(id)}`, body),

    /** DELETE /v1/providers/connections/:id — disable/remove a provider connection. */
    deleteProviderConnection: (id: string): Promise<{ deleted: boolean; connection_id: string }> =>
      del(`/v1/providers/connections/${encodeURIComponent(id)}`),

    /** GET /v1/providers/connections/:id/models — list models for a connection. */
    listConnectionModels: (id: string): Promise<{ items: unknown[]; has_more: boolean }> =>
      get(`/v1/providers/connections/${encodeURIComponent(id)}/models`),

    /** GET /v1/providers/connections/:id/test — probe the provider and return reachability + latency. */
    testConnection: (id: string): Promise<{ ok: boolean; latency_ms: number; provider: string; status: number; detail: string }> =>
      get(`/v1/providers/connections/${encodeURIComponent(id)}/test`),

    // ── Default settings ─────────────────────────────────────────────────────

    /** PUT /v1/settings/defaults/:scope/:scopeId/:key — persist a tenant-level default. */
    setDefaultSetting: (scope: string, scopeId: string, key: string, value: unknown): Promise<unknown> =>
      put(`/v1/settings/defaults/${encodeURIComponent(scope)}/${encodeURIComponent(scopeId)}/${encodeURIComponent(key)}`, { value }),

    /** GET /v1/settings/defaults/resolve/:key — resolve effective default for a key.
     *  project must be "tenant/workspace/project" format, e.g. "default/default/default".
     *  Returns null on 404 (setting not configured) to avoid console error noise. */
    resolveDefaultSetting: async (key: string, project = "default/default/default"): Promise<{ key: string; value: unknown } | null> => {
      try {
        return await get<{ key: string; value: unknown }>(`/v1/settings/defaults/resolve/${encodeURIComponent(key)}?project=${encodeURIComponent(project)}`);
      } catch (e) {
        if (e instanceof ApiError && (e.status === 404 || e.status === 501)) return null;
        throw e;
      }
    },

    // ── LLM Traces ───────────────────────────────────────────────────────────

    /** GET /v1/traces — all recent LLM call traces (operator view). */
    getTraces: (limit = 500): Promise<import("./types").TracesResponse> =>
      get(`/v1/traces?limit=${limit}`),

    /** GET /v1/sessions/:id/llm-traces — traces for one session. */
    getSessionTraces: (sessionId: string, limit = 200): Promise<import("./types").TracesResponse> =>
      get(`/v1/sessions/${sessionId}/llm-traces?limit=${limit}`),

    // ── Evals ────────────────────────────────────────────────────────────────

    /** GET /v1/evals/runs — list eval runs (operator view). */
    getEvalRuns: async (limit = 100): Promise<import("./types").EvalRunsResponse> => {
      const merged = withScope();
      const qs = new URLSearchParams();
      if (merged.tenant_id)    qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id) qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)   qs.set("project_id",   merged.project_id);
      qs.set("limit", String(limit));
      const raw = await get<{ items: Record<string, unknown>[]; hasMore?: boolean; has_more?: boolean }>(`/v1/evals/runs?${qs}`);
      // Backend sends created_at; UI expects started_at.  Normalise.
      const items = (raw.items ?? []).map(r => ({
        ...r,
        started_at: (r.started_at ?? r.created_at ?? 0) as number,
      })) as import("./types").EvalRunRecord[];
      return { items, has_more: raw.has_more ?? raw.hasMore ?? false };
    },

    /** POST /v1/evals/runs — create a new eval run. */
    createEvalRun: (body: {
      eval_run_id: string;
      subject_kind: string;
      evaluator_type: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").EvalRunRecord> => post("/v1/evals/runs", withScope(body)),

    // ── Audit Log ────────────────────────────────────────────────────────────

    /** GET /v1/admin/audit-log — list audit log entries (most recent first). */
    /** GET /v1/changelog — release notes array. Public endpoint. */
    // ── Agent templates ──────────────────────────────────────────────────────
    listAgentTemplates: (): Promise<import("./types").AgentTemplate[]> =>
      get("/v1/agent-templates"),

    instantiateAgentTemplate: (templateId: string, body: {
      goal: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<{
      template_id: string; template_name: string;
      session_id: string; run_id: string;
      goal: string; default_tools: string[];
      agent_role: string; approval_policy: string;
    }> => post(`/v1/agent-templates/${encodeURIComponent(templateId)}/instantiate`, withScope(body)),

    listSkills: async (): Promise<import("./types").SkillsResponse> => {
      const raw = await get<{
        items?: import("./types").SkillRecord[];
        summary?: import("./types").SkillsSummary;
        currentlyActive?: string[];
        currently_active?: string[];
      }>("/v1/skills");
      return {
        items: raw.items ?? [],
        summary: raw.summary ?? { total: 0, enabled: 0, disabled: 0 },
        currently_active: raw.currently_active ?? raw.currentlyActive ?? [],
      };
    },

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
      qs.set("tenant_id",    merged.tenant_id    ?? s?.tenant_id    ?? "default");
      qs.set("workspace_id", merged.workspace_id ?? s?.workspace_id ?? "default");
      qs.set("project_id",   merged.project_id   ?? s?.project_id   ?? "default");
      if (params.limit !== undefined) qs.set("limit", String(params.limit));
      return get(`/v1/memory/search?${qs}`);
    },

    /** GET /v1/graph/trace — live graph snapshot for the current project scope. */
    getGraphTrace: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      limit?: number;
    }): Promise<import("./types").GraphTraceResponse> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id) qs.set("tenant_id", merged.tenant_id);
      if (merged.workspace_id) qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id) qs.set("project_id", merged.project_id);
      if (params?.limit !== undefined) qs.set("limit", String(params.limit));
      return get(`/v1/graph/trace?${qs}`);
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

    // ── Marketplace (RFC 015) ─────────────────────────────────────────────────

    /** GET /v1/plugins/catalog — list marketplace catalog entries. */
    getPluginCatalog: (): Promise<{ plugins: import("./types").CatalogEntry[] }> =>
      get("/v1/plugins/catalog"),

    /** POST /v1/plugins/:id/install — install a marketplace plugin. */
    installPlugin: (pluginId: string): Promise<unknown> =>
      post(`/v1/plugins/${encodeURIComponent(pluginId)}/install`),

    /** POST /v1/plugins/:id/credentials — provide credentials for an installed plugin. */
    providePluginCredentials: (pluginId: string, credentials: Record<string, string>): Promise<unknown> =>
      post(`/v1/plugins/${encodeURIComponent(pluginId)}/credentials`, { credentials }),

    /** POST /v1/plugins/:id/verify — verify credentials are working. */
    verifyPlugin: (pluginId: string): Promise<unknown> =>
      post(`/v1/plugins/${encodeURIComponent(pluginId)}/verify`),

    /** POST /v1/plugins/:id/uninstall — uninstall a marketplace plugin. */
    uninstallPlugin: (pluginId: string): Promise<unknown> =>
      del(`/v1/plugins/${encodeURIComponent(pluginId)}/uninstall`),

    /** POST /v1/projects/:project/plugins/:id/enable — enable plugin for project. */
    enablePluginForProject: (project: string, pluginId: string, body?: unknown): Promise<unknown> =>
      post(`/v1/projects/${encodeURIComponent(project)}/plugins/${encodeURIComponent(pluginId)}/enable`, body),

    /** DELETE /v1/projects/:project/plugins/:id/disable — disable plugin for project. */
    disablePluginForProject: (project: string, pluginId: string): Promise<unknown> =>
      post(`/v1/projects/${encodeURIComponent(project)}/plugins/${encodeURIComponent(pluginId)}/disable`),

    // ── Plan Review (RFC 018) ──────────────────────────────────────────────────

    /** POST /v1/runs/:id/approve — approve a plan-mode run. */
    approvePlan: (runId: string, body: { approved_by: string; comments?: string }): Promise<unknown> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/approve`, body),

    /** POST /v1/runs/:id/reject — reject a plan-mode run. */
    rejectPlan: (runId: string, body: { rejected_by: string; reason: string }): Promise<unknown> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/reject`, body),

    /** POST /v1/runs/:id/revise — request revision of a plan-mode run. */
    revisePlan: (runId: string, body: { reviewer_comments: string }): Promise<unknown> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/revise`, body),

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

    // ── Integrations (GitHub) ───────────────────────────────────────────────

    /** GET /v1/webhooks/github/installations — list GitHub App installations. */
    getGitHubInstallations: (): Promise<{ installations: { id: number; account: string; repository_selection: string | null }[]; configured: boolean }> =>
      get("/v1/webhooks/github/installations"),

    /** GET /v1/webhooks/github/actions — list event→action mappings. */
    getGitHubActions: (): Promise<{ actions: GitHubEventAction[]; github_configured: boolean }> =>
      get("/v1/webhooks/github/actions"),

    /** PUT /v1/webhooks/github/actions — replace event→action mappings. */
    setGitHubActions: (actions: GitHubEventAction[]): Promise<{ status: string; actions_count: number }> =>
      put("/v1/webhooks/github/actions", { actions }),

    /** POST /v1/webhooks/github/scan — scan repo for open issues. */
    scanGitHubIssues: (repo: string, opts?: { installation_id?: number; labels?: string; limit?: number }): Promise<GitHubScanResult> =>
      post("/v1/webhooks/github/scan", { repo, ...opts }),

    /** GET /v1/webhooks/github/queue — get issue processing queue. */
    getGitHubQueue: (): Promise<{
      queue: GitHubQueueEntry[];
      total: number;
      max_concurrent: number;
      dispatcher_running: boolean;
    }> =>
      get("/v1/webhooks/github/queue"),

    /** POST /v1/webhooks/github/queue/pause — pause processing. */
    pauseGitHubQueue: (): Promise<{ status: string }> =>
      post("/v1/webhooks/github/queue/pause", {}),

    /** POST /v1/webhooks/github/queue/resume — resume processing. */
    resumeGitHubQueue: (): Promise<{ status: string }> =>
      post("/v1/webhooks/github/queue/resume", {}),

    /** POST /v1/webhooks/github/queue/:issue/skip — skip an issue. */
    skipGitHubIssue: (issue: number): Promise<{ status: string }> =>
      post(`/v1/webhooks/github/queue/${issue}/skip`, {}),

    /** POST /v1/webhooks/github/queue/:issue/retry — retry a failed issue. */
    retryGitHubIssue: (issue: number): Promise<{ status: string }> =>
      post(`/v1/webhooks/github/queue/${issue}/retry`, {}),
  };
}

// ── GitHub integration types ─────────────────────────────────────────────────

export interface GitHubEventAction {
  event_pattern: string;
  label_filter?: string;
  repo_filter?: string;
  action: "create_and_orchestrate" | "acknowledge" | "ignore";
}

export interface GitHubScanResult {
  status: string;
  repo: string;
  total_issues: number;
  queued: number;
  issues: { issue_number: number; title: string; session_id: string; run_id: string }[];
}

export interface GitHubQueueEntry {
  repo: string;
  issue_number: number;
  title: string;
  session_id: string;
  run_id: string;
  status: string;
}

// ── Token persistence ─────────────────────────────────────────────────────────

export const TOKEN_KEY = 'cairn_token';

/**
 * Custom-event name dispatched on `window` by the global 401 interceptor
 * (see `main.tsx`) when a query or mutation fails with status 401. The App
 * shell listens for this and transitions auth state back to `unauthenticated`
 * so the operator is routed to the LoginPage instead of staring at a red
 * error badge on every page.
 */
export const AUTH_EXPIRED_EVENT = 'cairn:auth-expired';

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
