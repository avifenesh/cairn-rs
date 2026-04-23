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
import { DEFAULT_SCOPE } from "./scope";

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
      // Cairn handlers use two body shapes for errors:
      //   1. `{ code, message }`    — used by most handlers.
      //   2. `{ error: string }`    — used by repo_routes, credentials, and
      //      a handful of older handlers that predate the unified envelope.
      // Prefer `message` when present, fall back to `error` so UI toasts
      // surface the real backend reason instead of a generic `HTTP 400`.
      message = err.message ?? err.error ?? message;
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
 * The parser recognises two distinct metric families:
 *
 * 1. **`/v1/metrics/prometheus` (the cairn-app handler — primary path).**
 *    Emits direct gauges computed from the live latency reservoir and
 *    per-path / per-status counters. No histogram buckets.
 *      cairn_http_requests_total                                 42
 *      cairn_http_requests_by_path_total{path="/v1/runs"}        42
 *      cairn_http_latency_ms{quantile="0.50"}                    12
 *      cairn_http_latency_ms{quantile="0.95"}                    85
 *      cairn_http_latency_ms{quantile="0.99"}                   140
 *      cairn_http_latency_ms{quantile="avg"}                     18
 *      cairn_http_error_rate                                      0.004
 *      cairn_http_errors_by_status{status="500"}                  2
 *
 * 2. **Standard Prometheus histogram + generic counters (defensive
 *    fallback).** Kept so the parser still works against any upstream
 *    `text/plain` scrape that emits classical histogram buckets — e.g. a
 *    future hardening of the endpoint or a proxy that re-exports buckets.
 *      http_requests_total{method="GET",path="/v1/runs",status="200"} 42
 *      http_request_duration_ms_sum{method="GET",path="/v1/runs"}   1234
 *      http_request_duration_ms_count{method="GET",path="/v1/runs"}   42
 *      http_request_duration_ms_bucket{method="GET",path="/v1/runs",le="100"} 40
 *      active_runs_total                                              3
 *      active_tasks_total                                             7
 *
 * Both forms accept an optional `cairn_` prefix (see #131). When both a
 * direct quantile gauge and a bucket series are present, the direct
 * gauge wins — the reservoir is authoritative.
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

  // Direct quantile/gauge values emitted by `/v1/metrics/prometheus`
  // (cairn_http_latency_ms{quantile="0.50|0.95|0.99|avg"}). These take
  // precedence over histogram-bucket-derived percentiles when present —
  // the Rust handler computes them from the live reservoir and doesn't
  // emit `*_bucket` series.
  let quantileP50: number | null = null;
  let quantileP95: number | null = null;
  let quantileP99: number | null = null;
  let quantileAvg: number | null = null;
  let directErrorRate: number | null = null;

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

    // cairn_http_latency_ms{quantile="0.50|0.95|0.99|avg"} — direct gauges
    // from the `/v1/metrics/prometheus` handler (no histogram buckets).
    if (metricName === 'cairn_http_latency_ms' || metricName === 'http_latency_ms') {
      if (labels.quantile === '0.50') quantileP50 = value;
      else if (labels.quantile === '0.95') quantileP95 = value;
      else if (labels.quantile === '0.99') quantileP99 = value;
      else if (labels.quantile === 'avg')  quantileAvg = value;
    }

    // cairn_http_requests_by_path_total{path="…"} — per-path request counts
    // (the handler keeps this separate from the unlabelled `*_requests_total`).
    if (metricName === 'cairn_http_requests_by_path_total' ||
        metricName === 'http_requests_by_path_total') {
      if (labels.path) {
        requestsByPath[labels.path] = (requestsByPath[labels.path] ?? 0) + value;
      }
    }

    // cairn_http_error_rate — gauge, fraction in [0,1].
    if (metricName === 'cairn_http_error_rate' || metricName === 'http_error_rate') {
      directErrorRate = value;
    }

    // cairn_http_errors_by_status{status="…"} — counters.
    if (metricName === 'cairn_http_errors_by_status' ||
        metricName === 'http_errors_by_status') {
      if (labels.status) {
        errorsByStatus[labels.status] =
          (errorsByStatus[labels.status] ?? 0) + value;
        totalErrors += value;
      }
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

  // Prefer directly-emitted quantile gauges (from `/v1/metrics/prometheus`)
  // over bucket-derived approximations when both are available.
  const p50 = quantileP50 ?? percentileFromBuckets(0.5);
  const p95 = quantileP95 ?? percentileFromBuckets(0.95);
  const p99 = quantileP99 ?? percentileFromBuckets(0.99);
  const avgLatencyFinal = quantileAvg ?? avgLatency;

  const errorRate = directErrorRate ??
    (totalRequests > 0 ? totalErrors / totalRequests : 0);

  return {
    total_requests: totalRequests,
    requests_by_path: requestsByPath,
    avg_latency_ms: avgLatencyFinal,
    p50_latency_ms: p50,
    p95_latency_ms: p95,
    p99_latency_ms: p99,
    error_rate: errorRate,
    errors_by_status: errorsByStatus,
  };
}

/** Reduce a list of `SessionCostRecord` into the aggregate stat-card shape.
 *
 *  Pre-fix (issue #158) the CostsPage assumed `/v1/costs` returned a flat
 *  `CostSummary`; it actually returns `{items, has_more}`. This helper does
 *  the fold client-side so every stat card is accurate regardless of how
 *  many sessions are present. */
export function summariseCostItems(
  items: readonly import("./types").SessionCostRecord[],
): CostSummary {
  let total_cost_micros = 0;
  let total_tokens_in   = 0;
  let total_tokens_out  = 0;
  let total_provider_calls = 0;
  for (const it of items) {
    total_cost_micros    += it.total_cost_micros ?? 0;
    total_tokens_in      += it.total_tokens_in   ?? it.token_in  ?? 0;
    total_tokens_out     += it.total_tokens_out  ?? it.token_out ?? 0;
    total_provider_calls += it.provider_calls    ?? 0;
  }
  return { total_cost_micros, total_tokens_in, total_tokens_out, total_provider_calls };
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

    /** GET /v1/sessions/:id/runs — list runs for one session (server-side filter). */
    getSessionRuns: (
      sessionId: string,
      params?: { limit?: number; offset?: number },
    ): Promise<RunRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const query = qs.toString() ? `?${qs}` : "";
      return getList(`/v1/sessions/${encodeURIComponent(sessionId)}/runs${query}`);
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

    /**
     * DELETE /v1/admin/tenants/:tenant_id/workspaces/:workspace_id — soft-delete
     * a workspace (issue #218). The record is preserved on the backend with
     * `archived_at` set so audit trails remain intact; by default the workspace
     * drops out of `getWorkspaces`. Pass `include_archived: true` on a GET to
     * surface archived workspaces again.
     */
    deleteWorkspace: (tenantId: string, workspaceId: string): Promise<void> =>
      del<void>(
        `/v1/admin/tenants/${encodeURIComponent(tenantId)}/workspaces/${encodeURIComponent(workspaceId)}`,
      ),

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

    // ── Workers / Fleet (GAP-005) ─────────────────────────────────────────────

    /**
     * GET /v1/workers — list registered external workers for the active tenant.
     * Scope is derived from the tenant-scoped auth token (admin or tenant-bound).
     */
    listWorkers: (params?: { limit?: number; offset?: number }): Promise<import("./types").WorkerRecord[]> => {
      const qs = new URLSearchParams();
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const q = qs.toString() ? `?${qs}` : "";
      return getList(`/v1/workers${q}`);
    },

    /** GET /v1/workers/:id — single worker detail. */
    getWorker: (id: string): Promise<import("./types").WorkerRecord> =>
      get(`/v1/workers/${encodeURIComponent(id)}`),

    /** GET /v1/fleet — fleet report: per-tenant worker list + aggregate counts. */
    getFleet: (): Promise<import("./types").FleetReport> => get("/v1/fleet"),

    /**
     * POST /v1/workers/:id/suspend — mark the worker suspended so it stops
     * accepting claims. `reason` is required by the handler.
     */
    suspendWorker: (id: string, reason: string): Promise<import("./types").WorkerRecord> =>
      post(`/v1/workers/${encodeURIComponent(id)}/suspend`, { reason }),

    /** POST /v1/workers/:id/reactivate — clear suspension, worker can claim again. */
    reactivateWorker: (id: string): Promise<import("./types").WorkerRecord> =>
      post(`/v1/workers/${encodeURIComponent(id)}/reactivate`),

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
    pauseRun: async (runId: string, body?: import("./types").PauseRunRequest): Promise<RunRecord> =>
      unwrapRun(await post<RunRecord | { run: RunRecord }>(`/v1/runs/${encodeURIComponent(runId)}/pause`, body ?? {})),

    /** POST /v1/runs/:id/resume — resume a paused run. */
    resumeRun: async (runId: string, body?: import("./types").ResumeRunRequest): Promise<RunRecord> =>
      unwrapRun(await post<RunRecord | { run: RunRecord }>(`/v1/runs/${encodeURIComponent(runId)}/resume`, body ?? {})),

    /**
     * POST /v1/runs/:id/recover — legacy no-op kept for back-compat.
     * Recovery is driven by FlowFabric scanners; this endpoint simply
     * confirms the run exists and returns 202. The UI surfaces it so
     * operators have a "nudge" affordance that matches Go-sdk parity.
     */
    recoverRun: (runId: string): Promise<import("./types").RecoverRunResponse> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/recover`),

    /**
     * Replay a run.
     *
     *  - `GET /v1/runs/:id/replay` — summary replay (optionally windowed
     *    by `from_position` / `to_position`).
     *  - `POST /v1/runs/:id/replay-to-checkpoint?checkpoint_id=…` —
     *    replay up to a specific checkpoint.
     */
    replayRun: (
      runId: string,
      query?: { from_position?: number; to_position?: number; checkpoint_id?: string },
    ): Promise<import("./types").ReplayResult> => {
      if (query?.checkpoint_id) {
        const qs = new URLSearchParams({ checkpoint_id: query.checkpoint_id }).toString();
        return post(`/v1/runs/${encodeURIComponent(runId)}/replay-to-checkpoint?${qs}`);
      }
      const params = new URLSearchParams();
      if (query?.from_position !== undefined) params.set("from_position", String(query.from_position));
      if (query?.to_position !== undefined) params.set("to_position", String(query.to_position));
      const qs = params.toString();
      return get(`/v1/runs/${encodeURIComponent(runId)}/replay${qs ? `?${qs}` : ""}`);
    },

    /** POST /v1/runs/:id/claim — operator claim (inspection lock). */
    claimRun: async (runId: string): Promise<RunRecord> =>
      unwrapRun(await post<RunRecord | { run: RunRecord }>(`/v1/runs/${encodeURIComponent(runId)}/claim`)),

    /** POST /v1/runs/:id/spawn — spawn a subagent run. */
    spawnSubagentRun: (
      runId: string,
      body: import("./types").SpawnSubagentRequest,
    ): Promise<import("./types").SpawnSubagentResponse> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/spawn`, body),

    /** GET /v1/runs/:id/children — list subagent runs spawned by this run. */
    listChildRuns: (runId: string, limit = 50): Promise<RunRecord[]> =>
      getList(`/v1/runs/${encodeURIComponent(runId)}/children?limit=${limit}`),

    /** POST /v1/runs/:id/orchestrate — drive the GATHER/DECIDE/EXECUTE loop. */
    orchestrateRun: (
      runId: string,
      body?: { goal?: string; model_id?: string; max_iterations?: number; timeout_ms?: number },
    ): Promise<import("./types").OrchestrateResult> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/orchestrate`, body ?? {}),

    /** POST /v1/runs/:id/diagnose — return a diagnosis report for a stuck run. */
    diagnoseRun: (runId: string): Promise<import("./types").DiagnoseResult> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/diagnose`),

    /** POST /v1/runs/:id/intervene — operator intervention. */
    interveneRun: (
      runId: string,
      body: import("./types").InterveneRequest,
    ): Promise<import("./types").InterveneResponse> =>
      post(`/v1/runs/${encodeURIComponent(runId)}/intervene`, body),

    /** GET /v1/runs/:id/interventions — list interventions on this run. */
    listRunInterventions: (runId: string, limit = 50): Promise<import("./types").InterventionRecord[]> =>
      getList(`/v1/runs/${encodeURIComponent(runId)}/interventions?limit=${limit}`),

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

    /** GET /v1/costs — list of per-session cost records for the active tenant.
     *  Returns `{ items, hasMore }` (backend `ListResponse<T>` uses
     *  camelCase on the wire). Callers typically pass this through
     *  `summariseCostItems` to get a `CostSummary` for stat-card rendering. */
    getCosts: (): Promise<import("./types").CostListResponse> => get("/v1/costs"),

    // ── API metrics ──────────────────────────────────────────────────────────

    /** GET /v1/metrics/prometheus — rolling request metrics in Prometheus
     *  text exposition format. This endpoint emits direct quantile gauges
     *  (`cairn_http_latency_ms{quantile="0.50|0.95|0.99|avg"}`) plus per-path
     *  and per-status counters, so the UI can render p50/p95/p99 latency —
     *  which the JSON `/v1/metrics` endpoint does not provide. The response
     *  is always `text/plain`, but the JSON fallback below is kept as
     *  defensive compatibility.
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
      const url = `${config.baseUrl}/v1/metrics/prometheus`;
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

    /** GET /v1/providers/connections/:id/discover-models — query the upstream provider for its model catalog.
     *
     *  Response shape: `{ provider, endpoint, models: DiscoveredModel[] }`.
     *  Callers that only want the model IDs can pass through `discoverModelIds`. */
    discoverModels: (id: string): Promise<{
      provider: string;
      endpoint: string;
      models: Array<{
        model_id: string;
        name: string;
        parameter_size?: string;
        quantization?: string;
        capabilities: string[];
        context_window_tokens?: number;
      }>;
    }> => get(`/v1/providers/connections/${encodeURIComponent(id)}/discover-models`),

    /** Convenience wrapper: return only the model IDs from `discoverModels`. */
    discoverModelIds: async (id: string): Promise<string[]> => {
      const r = await get<{ models: Array<{ model_id: string }> }>(
        `/v1/providers/connections/${encodeURIComponent(id)}/discover-models`,
      );
      return r.models.map((m) => m.model_id);
    },

    // ── Default settings ─────────────────────────────────────────────────────

    /** PUT /v1/settings/defaults/:scope/:scopeId/:key — persist a tenant-level default. */
    setDefaultSetting: (scope: string, scopeId: string, key: string, value: unknown): Promise<unknown> =>
      put(`/v1/settings/defaults/${encodeURIComponent(scope)}/${encodeURIComponent(scopeId)}/${encodeURIComponent(key)}`, { value }),

    /** GET /v1/settings/defaults/resolve/:key — resolve effective default for a key.
     *  `project` must be "tenant/workspace/project" format. When omitted, the
     *  canonical DEFAULT_SCOPE (`default_tenant/default_workspace/default_project`)
     *  is used — these must match the Rust `DEFAULT_*` constants.
     *  Returns null on 404 (setting not configured) to avoid console error noise. */
    resolveDefaultSetting: async (
      key: string,
      project = `${DEFAULT_SCOPE.tenant_id}/${DEFAULT_SCOPE.workspace_id}/${DEFAULT_SCOPE.project_id}`,
    ): Promise<{ key: string; value: unknown } | null> => {
      try {
        return await get<{ key: string; value: unknown }>(`/v1/settings/defaults/resolve/${encodeURIComponent(key)}?project=${encodeURIComponent(project)}`);
      } catch (e) {
        if (e instanceof ApiError && (e.status === 404 || e.status === 501)) return null;
        throw e;
      }
    },

    // ── LLM Traces ───────────────────────────────────────────────────────────

    /** GET /v1/traces — all recent LLM call traces (operator view).
     *
     * Scope params are folded in via `withScope()` so the request URL
     * carries the current tenant/workspace/project for
     * forward-compatible backend filtering. Callers that want their
     * React-Query cache to invalidate on scope change must also
     * include scope in their `queryKey` (see `TracesPage`).
     */
    getTraces: (
      params?: {
        limit?: number;
        tenant_id?: string;
        workspace_id?: string;
        project_id?: string;
      },
    ): Promise<import("./types").TracesResponse> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      qs.set("limit", String(merged.limit ?? 500));
      if (merged.tenant_id)    qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id) qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)   qs.set("project_id",   merged.project_id);
      return get(`/v1/traces?${qs}`);
    },

    /** GET /v1/sessions/:id/llm-traces — traces for one session. */
    getSessionTraces: (sessionId: string, limit = 200): Promise<import("./types").TracesResponse> =>
      get(`/v1/sessions/${encodeURIComponent(sessionId)}/llm-traces?limit=${limit}`),

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

    /** POST /v1/evals/runs — create a new eval run.
     *  Real eval contract: dataset_id / rubric_id / baseline_id are validated
     *  against tenant state at create time (404 if dangling). prompt_release_id
     *  ties the run to the subject under test when subject_kind is
     *  `prompt_release`. */
    createEvalRun: (body: {
      eval_run_id: string;
      subject_kind: string;
      evaluator_type: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      dataset_id?: string;
      rubric_id?: string;
      baseline_id?: string;
      prompt_release_id?: string;
      prompt_asset_id?: string;
      prompt_version_id?: string;
      created_by?: string;
    }): Promise<import("./types").EvalRunRecord> => post("/v1/evals/runs", withScope(body)),

    /** GET /v1/evals/datasets — list datasets scoped to the active tenant. */
    listEvalDatasets: async (): Promise<import("./types").EvalDatasetRecord[]> => {
      const merged = withScope();
      const qs = new URLSearchParams();
      if (merged.tenant_id) qs.set("tenant_id", merged.tenant_id);
      return getList<import("./types").EvalDatasetRecord>(`/v1/evals/datasets${qs.toString() ? `?${qs}` : ""}`);
    },

    /** GET /v1/evals/rubrics — list rubrics scoped to the active tenant. */
    listEvalRubrics: async (): Promise<import("./types").EvalRubricRecord[]> => {
      const merged = withScope();
      const qs = new URLSearchParams();
      if (merged.tenant_id) qs.set("tenant_id", merged.tenant_id);
      return getList<import("./types").EvalRubricRecord>(`/v1/evals/rubrics${qs.toString() ? `?${qs}` : ""}`);
    },

    /** GET /v1/evals/baselines — list baselines scoped to the active tenant. */
    listEvalBaselines: async (): Promise<import("./types").EvalBaselineRecord[]> => {
      const merged = withScope();
      const qs = new URLSearchParams();
      if (merged.tenant_id) qs.set("tenant_id", merged.tenant_id);
      return getList<import("./types").EvalBaselineRecord>(`/v1/evals/baselines${qs.toString() ? `?${qs}` : ""}`);
    },

    /** GET /v1/evals/compare?run_ids=a,b — side-by-side metric comparison. */
    getEvalComparison: (runIds: string[]): Promise<import("./types").EvalCompareResponse> => {
      const qs = new URLSearchParams();
      qs.set("run_ids", runIds.join(","));
      return get(`/v1/evals/compare?${qs}`);
    },

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

    listSkills: async (params?: { tag?: string }): Promise<import("./types").SkillsResponse> => {
      const qs = params?.tag ? `?tag=${encodeURIComponent(params.tag)}` : "";
      const raw = await get<{
        items?: import("./types").SkillRecord[];
        summary?: import("./types").SkillsSummary;
        currentlyActive?: string[];
        currently_active?: string[];
      }>(`/v1/skills${qs}`);
      return {
        items: raw.items ?? [],
        summary: raw.summary ?? { total: 0, enabled: 0, disabled: 0 },
        currently_active: raw.currently_active ?? raw.currentlyActive ?? [],
      };
    },

    getSkill: (skillId: string): Promise<import("./types").SkillDetail> =>
      get(`/v1/skills/${encodeURIComponent(skillId)}`),

    getChangelog: (): Promise<import("./types").ChangelogEntry[]> =>
      get('/v1/changelog'),

    getAuditLog: (params?: {
      limit?: number;
      /** Inclusive lower bound on `occurred_at_ms`. */
      since_ms?: number;
      /** Exclusive upper bound on `occurred_at_ms` — cursor for older pages. */
      before_ms?: number;
    }): Promise<import("./types").AuditLogResponse> => {
      const qs = new URLSearchParams();
      qs.set("limit", String(params?.limit ?? 100));
      if (params?.since_ms  !== undefined) qs.set("since_ms",  String(params.since_ms));
      if (params?.before_ms !== undefined) qs.set("before_ms", String(params.before_ms));
      return get(`/v1/admin/audit-log?${qs}`);
    },

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
      const qs = new URLSearchParams();
      qs.set("query_text",   params.query_text);
      qs.set("tenant_id",    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id);
      qs.set("workspace_id", merged.workspace_id ?? DEFAULT_SCOPE.workspace_id);
      qs.set("project_id",   merged.project_id   ?? DEFAULT_SCOPE.project_id);
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

    /** GET /v1/graph/execution-trace/:run_id — execution subgraph rooted at a run. */
    getGraphExecutionTrace: (params: {
      run_id: string;
      max_depth?: number;
    }): Promise<import("./types").GraphTraceResponse> => {
      const qs = new URLSearchParams();
      if (params.max_depth !== undefined) qs.set("max_depth", String(params.max_depth));
      return get(`/v1/graph/execution-trace/${encodeURIComponent(params.run_id)}?${qs}`);
    },

    /**
     * GET /v1/graph/dependency-path/:run_id — downstream dependency path.
     *
     * The backend path-param is named `:run_id` but the handler treats it
     * as a generic `node_id` (it is fed straight into
     * `GraphQuery::DependencyPath { node_id }`), so any graph node works.
     * Direction is fixed to `downstream` on the server today; there is no
     * upstream toggle exposed by this route.
     */
    getGraphDependencyPath: (params: {
      node_id: string;
      max_depth?: number;
    }): Promise<import("./types").GraphTraceResponse> => {
      const qs = new URLSearchParams();
      if (params.max_depth !== undefined) qs.set("max_depth", String(params.max_depth));
      return get(`/v1/graph/dependency-path/${encodeURIComponent(params.node_id)}?${qs}`);
    },

    /** GET /v1/graph/retrieval-provenance/:run_id — answer → chunk → document → source lineage. */
    getGraphRetrievalProvenance: (params: {
      run_id: string;
    }): Promise<import("./types").GraphTraceResponse> => {
      return get(`/v1/graph/retrieval-provenance/${encodeURIComponent(params.run_id)}`);
    },

    /** GET /v1/graph/prompt-provenance/:release_id — prompt release lineage. */
    getGraphPromptProvenance: (params: {
      release_id: string;
    }): Promise<import("./types").GraphTraceResponse> => {
      return get(`/v1/graph/prompt-provenance/${encodeURIComponent(params.release_id)}`);
    },

    /** GET /v1/graph/multi-hop/:node_id — generic BFS traversal. */
    getGraphMultiHop: (params: {
      node_id: string;
      max_hops?: number;
      min_confidence?: number;
      direction?: "upstream" | "downstream";
    }): Promise<import("./types").GraphTraceResponse> => {
      const qs = new URLSearchParams();
      if (params.max_hops !== undefined) qs.set("max_hops", String(params.max_hops));
      if (params.min_confidence !== undefined) qs.set("min_confidence", String(params.min_confidence));
      if (params.direction) qs.set("direction", params.direction);
      return get(`/v1/graph/multi-hop/${encodeURIComponent(params.node_id)}?${qs}`);
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

    /** POST /v1/memory/ingest — ingest a single document into a source. */
    ingestMemory: (body: {
      source_id: string;
      document_id: string;
      content: string;
      source_type?: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").MemoryIngestResponse> => {
      const merged = withScope(body);
      return post("/v1/memory/ingest", {
        tenant_id:    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id,
        workspace_id: merged.workspace_id ?? DEFAULT_SCOPE.workspace_id,
        project_id:   merged.project_id   ?? DEFAULT_SCOPE.project_id,
        source_id:    body.source_id,
        document_id:  body.document_id,
        content:      body.content,
        ...(body.source_type ? { source_type: body.source_type } : {}),
      });
    },

    /** POST /v1/sources — register a new source. */
    createSource: (body: {
      source_id: string;
      name?: string;
      description?: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").SourceRecord> => {
      const merged = withScope(body);
      return post("/v1/sources", {
        tenant_id:    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id,
        workspace_id: merged.workspace_id ?? DEFAULT_SCOPE.workspace_id,
        project_id:   merged.project_id   ?? DEFAULT_SCOPE.project_id,
        source_id:    body.source_id,
        ...(body.name        ? { name: body.name }               : {}),
        ...(body.description ? { description: body.description } : {}),
      });
    },

    /** GET /v1/sources/:id — detailed source view. */
    getSource: (sourceId: string, params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").SourceDetailResponse> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      qs.set("tenant_id",    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id);
      qs.set("workspace_id", merged.workspace_id ?? DEFAULT_SCOPE.workspace_id);
      qs.set("project_id",   merged.project_id   ?? DEFAULT_SCOPE.project_id);
      return get(`/v1/sources/${encodeURIComponent(sourceId)}?${qs}`);
    },

    /** PUT /v1/sources/:id — update source metadata (name/description). */
    updateSource: (sourceId: string, body: {
      // Widened to `string | null` to match the wire contract — the backend
      // accepts null to clear these fields, and this client actively sends
      // null when the caller passes undefined. Matches UpdateSourceRequest
      // in types.ts.
      name?: string | null;
      description?: string | null;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").SourceDetailResponse> => {
      const merged = withScope(body);
      return put(`/v1/sources/${encodeURIComponent(sourceId)}`, {
        tenant_id:    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id,
        workspace_id: merged.workspace_id ?? DEFAULT_SCOPE.workspace_id,
        project_id:   merged.project_id   ?? DEFAULT_SCOPE.project_id,
        name:         body.name        ?? null,
        description:  body.description ?? null,
      });
    },

    /** DELETE /v1/sources/:id — deactivate a source. */
    deleteSource: (sourceId: string, params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<{ ok: boolean }> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      qs.set("tenant_id",    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id);
      qs.set("workspace_id", merged.workspace_id ?? DEFAULT_SCOPE.workspace_id);
      qs.set("project_id",   merged.project_id   ?? DEFAULT_SCOPE.project_id);
      return del(`/v1/sources/${encodeURIComponent(sourceId)}?${qs}`);
    },

    /** GET /v1/sources/:id/chunks — paginated chunk list for a source. */
    getSourceChunks: (sourceId: string, params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      limit?: number;
      offset?: number;
    }): Promise<import("./types").ListResponse<import("./types").SourceChunkView>> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      qs.set("tenant_id",    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id);
      qs.set("workspace_id", merged.workspace_id ?? DEFAULT_SCOPE.workspace_id);
      qs.set("project_id",   merged.project_id   ?? DEFAULT_SCOPE.project_id);
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      return get(`/v1/sources/${encodeURIComponent(sourceId)}/chunks?${qs}`);
    },

    /** GET /v1/sources/:id/refresh-schedule — current schedule, if any. */
    getSourceRefreshSchedule: (sourceId: string, params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").RefreshScheduleResponse> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      qs.set("tenant_id",    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id);
      qs.set("workspace_id", merged.workspace_id ?? DEFAULT_SCOPE.workspace_id);
      qs.set("project_id",   merged.project_id   ?? DEFAULT_SCOPE.project_id);
      return get(`/v1/sources/${encodeURIComponent(sourceId)}/refresh-schedule?${qs}`);
    },

    /** POST /v1/sources/:id/refresh-schedule — create or update schedule. */
    setSourceRefreshSchedule: (sourceId: string, body: {
      interval_ms: number;
      refresh_url?: string | null;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").RefreshScheduleResponse> => {
      const merged = withScope(body);
      const qs = new URLSearchParams();
      qs.set("tenant_id",    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id);
      qs.set("workspace_id", merged.workspace_id ?? DEFAULT_SCOPE.workspace_id);
      qs.set("project_id",   merged.project_id   ?? DEFAULT_SCOPE.project_id);
      return post(`/v1/sources/${encodeURIComponent(sourceId)}/refresh-schedule?${qs}`, {
        interval_ms:  body.interval_ms,
        ...(body.refresh_url !== undefined ? { refresh_url: body.refresh_url } : {}),
      });
    },

    /** POST /v1/sources/process-refresh — trigger due refresh schedules. */
    processSourceRefresh: (): Promise<import("./types").ProcessRefreshResponse> =>
      post("/v1/sources/process-refresh", {}),

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

    /**
     * POST /v1/projects/:project/plugins/:id — enable plugin for project.
     *
     * The backend route (`marketplace_routes.rs`) registers this as
     * `POST /v1/projects/:proj/plugins/:id` with NO `/enable` suffix, and
     * `:proj` is parsed as "tenant_id/workspace_id/project_id". Sending a
     * 1-segment id silently falls back to `default_tenant/default_workspace/<id>`
     * — the same cross-tenant leak that PR #132 closed for TriggersPage.
     * Axum 0.7 captures a single segment so `/` characters MUST be
     * percent-encoded on the wire. Callers pass the active scope; the
     * config-scope fallback mirrors `attachProjectRepo`.
     */
    enablePluginForProject: (
      pluginId: string,
      scope?: import("./scope").ProjectScope,
      body?: unknown,
    ): Promise<unknown> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return post(`/v1/projects/${path}/plugins/${encodeURIComponent(pluginId)}`, body);
    },

    /**
     * DELETE /v1/projects/:project/plugins/:id — disable plugin for project.
     *
     * Same path contract as enable above; method is DELETE (not POST) and
     * the URL has no `/disable` suffix. The old code 405'd in the browser
     * because it POSTed to a path that only accepts DELETE.
     */
    disablePluginForProject: (
      pluginId: string,
      scope?: import("./scope").ProjectScope,
    ): Promise<unknown> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return del(`/v1/projects/${path}/plugins/${encodeURIComponent(pluginId)}`);
    },

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

    // ── Runtime message channels (/v1/channels) ──────────────────────────────

    /** GET /v1/channels — list runtime channels in the current project. */
    listChannels: (params?: {
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
      limit?: number;
      offset?: number;
    }): Promise<import("./types").ListResponse<import("./types").Channel>> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      qs.set("tenant_id",    merged.tenant_id    ?? DEFAULT_SCOPE.tenant_id);
      qs.set("workspace_id", merged.workspace_id ?? DEFAULT_SCOPE.workspace_id);
      qs.set("project_id",   merged.project_id   ?? DEFAULT_SCOPE.project_id);
      if (params?.limit !== undefined)  qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      return get(`/v1/channels?${qs}`);
    },

    /** POST /v1/channels — create a new runtime channel. */
    createChannel: (
      name: string,
      capacity: number,
      scope?: import("./scope").ProjectScope,
    ): Promise<import("./types").Channel> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      return post("/v1/channels", {
        tenant_id:    s.tenant_id,
        workspace_id: s.workspace_id,
        project_id:   s.project_id,
        name,
        capacity,
      });
    },

    /** POST /v1/channels/:id/send — publish a message to a channel. */
    sendToChannel: (
      channelId: string,
      senderId: string,
      body: string,
    ): Promise<import("./types").SendChannelMessageResponse> =>
      post(`/v1/channels/${encodeURIComponent(channelId)}/send`, {
        sender_id: senderId,
        body,
      }),

    /** GET /v1/channels/:id/messages — list messages on a channel. */
    getChannelMessages: (
      channelId: string,
      limit?: number,
    ): Promise<import("./types").ChannelMessage[]> => {
      const qs = new URLSearchParams();
      if (limit !== undefined) qs.set("limit", String(limit));
      const suffix = qs.toString().length > 0 ? `?${qs}` : "";
      return get(`/v1/channels/${encodeURIComponent(channelId)}/messages${suffix}`);
    },

    /** POST /v1/channels/:id/consume — consume next message for consumer_id. */
    consumeChannelMessage: (
      channelId: string,
      consumerId: string,
    ): Promise<import("./types").ChannelMessage | null> =>
      post(`/v1/channels/${encodeURIComponent(channelId)}/consume`, {
        consumer_id: consumerId,
      }),

    // ── Notification channels (RFC 007/014) ──────────────────────────────────

    /** GET /v1/admin/operators/:operatorId/notifications — fetch preferences for one operator. */
    getNotificationPreferences: (
      operatorId: string,
      tenantId = DEFAULT_SCOPE.tenant_id,
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
    getFailedNotifications: (tenantId = DEFAULT_SCOPE.tenant_id): Promise<import("./types").ListResponse<import("./types").NotificationRecord>> => {
      const qs = new URLSearchParams({ tenant_id: tenantId });
      return get(`/v1/admin/notifications/failed?${qs}`);
    },

    /** POST /v1/admin/notifications/:id/retry — retry a failed delivery. */
    retryNotification: (recordId: string, tenantId = DEFAULT_SCOPE.tenant_id): Promise<import("./types").NotificationRecord> => {
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

    /** GET /v1/prompts/assets — list prompt assets (RFC 006 project-scoped). */
    getPromptAssets: (params?: {
      limit?: number;
      offset?: number;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").ListResponse<import("./types").PromptAssetRecord>> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.tenant_id)    qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id) qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id)   qs.set("project_id",   merged.project_id);
      if (params?.limit  !== undefined) qs.set("limit",  String(params.limit));
      if (params?.offset !== undefined) qs.set("offset", String(params.offset));
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/prompts/assets${q}`);
    },

    /** POST /v1/prompts/assets — create a new prompt asset (RFC 006 project-scoped). */
    createPromptAsset: (body: {
      prompt_asset_id: string;
      name: string;
      kind: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").PromptAssetRecord> =>
      post("/v1/prompts/assets", withScope(body)),

    /** GET /v1/prompts/assets/:id/versions — version history. */
    getPromptVersions: (assetId: string, params?: { limit?: number }): Promise<import("./types").ListResponse<import("./types").PromptVersionRecord>> => {
      const qs = new URLSearchParams();
      if (params?.limit !== undefined) qs.set("limit", String(params.limit));
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/prompts/assets/${encodeURIComponent(assetId)}/versions${q}`);
    },

    /** POST /v1/prompts/assets/:id/versions — create a new version. Server mints `pv_<uuid>` when `prompt_version_id` is omitted. */
    createPromptVersion: (assetId: string, body: {
      prompt_version_id?: string;
      content_hash: string;
      content?: string;
      template_vars?: import("./types").PromptTemplateVar[];
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").PromptVersionRecord> =>
      post(`/v1/prompts/assets/${encodeURIComponent(assetId)}/versions`, withScope(body)),

    /** GET /v1/prompts/assets/:id/versions/:vid/diff — diff two versions. */
    getVersionDiff: (assetId: string, versionId: string, compareTo: string): Promise<import("./types").PromptVersionDiff> =>
      get(`/v1/prompts/assets/${encodeURIComponent(assetId)}/versions/${encodeURIComponent(versionId)}/diff?compare_to=${encodeURIComponent(compareTo)}`),

    /** GET /v1/prompts/releases — list releases scoped to the active project. */
    getPromptReleases: (params?: {
      limit?: number;
      offset?: number;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").ListResponse<import("./types").PromptReleaseRecord>> => {
      const merged = withScope(params);
      const qs = new URLSearchParams();
      if (merged.limit        !== undefined) qs.set("limit",        String(merged.limit));
      if (merged.offset       !== undefined) qs.set("offset",       String(merged.offset));
      if (merged.tenant_id    !== undefined) qs.set("tenant_id",    merged.tenant_id);
      if (merged.workspace_id !== undefined) qs.set("workspace_id", merged.workspace_id);
      if (merged.project_id   !== undefined) qs.set("project_id",   merged.project_id);
      const q = qs.toString() ? `?${qs}` : "";
      return get(`/v1/prompts/releases${q}`);
    },

    /** POST /v1/prompts/releases — create a release from a version. Server mints `rel_<uuid>` when `prompt_release_id` is omitted. */
    createPromptRelease: (body: {
      prompt_release_id?: string;
      prompt_asset_id: string;
      prompt_version_id: string;
      release_tag?: string;
      tenant_id?: string;
      workspace_id?: string;
      project_id?: string;
    }): Promise<import("./types").PromptReleaseRecord> =>
      post("/v1/prompts/releases", withScope(body)),

    /** POST /v1/prompts/releases/:id/activate — activate a release. */
    activatePromptRelease: (releaseId: string): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/activate`, {}),

    /** POST /v1/prompts/releases/:id/rollout — set rollout percentage. */
    rolloutPromptRelease: (releaseId: string, percent: number): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/rollout`, { percent }),

    /** POST /v1/prompts/releases/:id/request-approval — request approval gate. */
    requestPromptReleaseApproval: (releaseId: string): Promise<unknown> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/request-approval`, withScope()),

    /** POST /v1/prompts/releases/:id/rollback — roll back to a previous release. */
    rollbackPromptRelease: (releaseId: string, targetReleaseId: string): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/rollback`, { target_release_id: targetReleaseId }),

    /** POST /v1/prompts/releases/:id/transition — generic state transition. */
    transitionPromptRelease: (
      releaseId: string,
      toState: import("./types").PromptReleaseState,
    ): Promise<import("./types").PromptReleaseRecord> =>
      post(`/v1/prompts/releases/${encodeURIComponent(releaseId)}/transition`, { to_state: toState }),

    // ── Request logs ─────────────────────────────────────────────────────────

    /**
     * GET /v1/admin/logs — structured request log tail from the in-memory ring buffer.
     * Supports ?limit=N and ?level=info,warn,error filtering.
     */
    getRequestLogs: (params?: {
      limit?: number;
      level?: string;
      /** Lower bound on entry timestamp in Unix-ms (for "last hour" filter). */
      since_ms?: number;
    }): Promise<import("./types").RequestLogsResponse> => {
      const qs = new URLSearchParams();
      if (params?.limit    !== undefined) qs.set("limit",    String(params.limit));
      if (params?.level)                  qs.set("level",    params.level);
      if (params?.since_ms !== undefined) qs.set("since_ms", String(params.since_ms));
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

    /** PUT /v1/webhooks/github/queue/concurrency — set max concurrent processing (1..=20).
     *  Returns the applied value (server clamps to 1..=20) and the previous value. */
    setGitHubQueueConcurrency: (maxConcurrent: number): Promise<{ max_concurrent: number; previous: number }> =>
      put("/v1/webhooks/github/queue/concurrency", { max_concurrent: maxConcurrent }),

    // ── Project repos (RFC 016) ─────────────────────────────────────────────
    //
    // The backend (`crates/cairn-app/src/repo_routes.rs`) parses `:project`
    // as `tenant_id/workspace_id/project_id` and silently falls back to the
    // DEFAULT_* constants when it cannot split on `/`. Sending a plain
    // `project_id` therefore cross-leaks — always send the full slash path.
    // Axum 0.7's `:project` captures a single segment, so the `/` chars must
    // be percent-encoded via `encodeURIComponent`. See FE audit 2026-04-22
    // and PR #132 (TriggersPage) for the same pattern.

    /** GET /v1/projects/:project/repos — list repos attached to a project. */
    listProjectRepos: async (
      scope?: import("./scope").ProjectScope,
    ): Promise<import("./types").ProjectRepoEntry[]> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      const raw = await get<{ project: string; repos: import("./types").ProjectRepoEntry[] }>(
        `/v1/projects/${path}/repos`,
      );
      return raw.repos ?? [];
    },

    /** POST /v1/projects/:project/repos — attach a repo to a project. */
    attachProjectRepo: (
      body: { repo_id: string },
      scope?: import("./scope").ProjectScope,
    ): Promise<import("./types").ProjectRepoMutation> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return post(`/v1/projects/${path}/repos`, body);
    },

    /** GET /v1/projects/:project/repos/:owner/:repo — repo detail. */
    getProjectRepo: (
      owner: string,
      repo: string,
      scope?: import("./scope").ProjectScope,
    ): Promise<import("./types").ProjectRepoDetail> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return get(`/v1/projects/${path}/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`);
    },

    /** DELETE /v1/projects/:project/repos/:owner/:repo — detach a repo. */
    detachProjectRepo: (
      owner: string,
      repo: string,
      scope?: import("./scope").ProjectScope,
    ): Promise<void> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return del(`/v1/projects/${path}/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`);
    },

    // ── Triggers & run templates (RFC 022) ────────────────────────────────────
    //
    // Same slash-path scope contract as project repos / plugins above: the
    // backend parses `:project` as "tenant_id/workspace_id/project_id" and
    // silently falls back to DEFAULT_* when it cannot split on `/`. Always
    // send the full scope, percent-encoded. See PR #132 (TriggersPage) and
    // issue #154 for the raw-fetch regression this closes.

    /** GET /v1/projects/:project/triggers — list triggers for a project.
     *  Callers provide their own row type; shapes live in TriggersPage. */
    listTriggers: <T = unknown>(
      scope?: import("./scope").ProjectScope,
    ): Promise<T[]> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return getList<T>(`/v1/projects/${path}/triggers`);
    },

    /** GET /v1/projects/:project/run-templates — list run templates for a project. */
    listRunTemplates: <T = unknown>(
      scope?: import("./scope").ProjectScope,
    ): Promise<T[]> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return getList<T>(`/v1/projects/${path}/run-templates`);
    },

    /** POST /v1/projects/:project/triggers/:id/enable — enable a trigger. */
    enableTrigger: (
      triggerId: string,
      scope?: import("./scope").ProjectScope,
    ): Promise<unknown> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return post(`/v1/projects/${path}/triggers/${encodeURIComponent(triggerId)}/enable`);
    },

    /** POST /v1/projects/:project/triggers/:id/disable — disable a trigger. */
    disableTrigger: (
      triggerId: string,
      scope?: import("./scope").ProjectScope,
    ): Promise<unknown> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return post(`/v1/projects/${path}/triggers/${encodeURIComponent(triggerId)}/disable`);
    },

    /** DELETE /v1/projects/:project/triggers/:id — delete a trigger. */
    deleteTrigger: (
      triggerId: string,
      scope?: import("./scope").ProjectScope,
    ): Promise<unknown> => {
      const s = scope ?? config.scope ?? DEFAULT_SCOPE;
      const path = encodeURIComponent(`${s.tenant_id}/${s.workspace_id}/${s.project_id}`);
      return del(`/v1/projects/${path}/triggers/${encodeURIComponent(triggerId)}`);
    },
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
