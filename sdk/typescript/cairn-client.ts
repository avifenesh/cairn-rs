/**
 * cairn-client.ts — TypeScript SDK for cairn-rs
 *
 * Single-file client for the cairn-rs operator control plane API.
 * Zero runtime dependencies — uses the standard `fetch` API only.
 *
 * Compatible with Node.js ≥18, Deno, Bun, and modern browsers.
 *
 * @example
 * ```typescript
 * const cairn = new CairnClient("http://localhost:3000", "my-token");
 * const session = await cairn.createSession({ tenant_id: "t1", workspace_id: "w1", project_id: "p1" });
 * const run     = await cairn.createRun({ session_id: session.session_id, project_id: "p1" });
 * ```
 */

// ── Domain types ──────────────────────────────────────────────────────────────

export interface ProjectKey {
  tenant_id:    string;
  workspace_id: string;
  project_id:   string;
}

export type SessionState = "open" | "completed" | "failed" | "archived";

export interface Session {
  session_id: string;
  project:    ProjectKey;
  state:      SessionState;
  version:    number;
  /** Unix milliseconds */
  created_at: number;
  /** Unix milliseconds */
  updated_at: number;
}

export type RunState =
  | "pending"
  | "running"
  | "paused"
  | "waiting_approval"
  | "waiting_dependency"
  | "completed"
  | "failed"
  | "canceled";

export interface Run {
  run_id:           string;
  session_id:       string;
  parent_run_id:    string | null;
  project:          ProjectKey;
  state:            RunState;
  prompt_release_id: string | null;
  agent_role_id:    string | null;
  failure_class:    string | null;
  pause_reason:     string | null;
  resume_trigger:   string | null;
  version:          number;
  /** Unix milliseconds */
  created_at: number;
  /** Unix milliseconds */
  updated_at: number;
}

export type TaskState =
  | "queued"
  | "leased"
  | "running"
  | "completed"
  | "failed"
  | "canceled"
  | "paused"
  | "waiting_dependency"
  | "retryable_failed"
  | "dead_lettered";

export interface Task {
  task_id:        string;
  project:        ProjectKey;
  parent_run_id:  string | null;
  parent_task_id: string | null;
  state:          TaskState;
  failure_class:  string | null;
  lease_owner:    string | null;
  /** Unix milliseconds — null when not leased */
  lease_expires_at: number | null;
  version:    number;
  created_at: number;
  updated_at: number;
}

export type ApprovalDecision  = "approved" | "rejected";
export type ApprovalRequirement = "required" | "advisory";

export interface Approval {
  approval_id: string;
  project:     ProjectKey;
  run_id:      string | null;
  task_id:     string | null;
  requirement: ApprovalRequirement;
  decision:    ApprovalDecision | null;
  /** Unix milliseconds */
  created_at: number;
}

export interface GenerateResponse {
  /** The model's text reply */
  response:   string;
  model:      string;
  latency_ms: number;
  /** Prompt tokens consumed */
  prompt_tokens:     number | null;
  /** Completion tokens generated */
  completion_tokens: number | null;
}

export interface SystemStatus {
  runtime_ok:  boolean;
  store_ok:    boolean;
  uptime_secs: number;
}

export interface SystemStats {
  total_events:      number;
  total_sessions:    number;
  total_runs:        number;
  total_tasks:       number;
  active_runs:       number;
  pending_approvals: number;
  uptime_seconds:    number;
}

export interface DashboardOverview {
  active_runs:            number;
  active_tasks:           number;
  pending_approvals:      number;
  failed_runs_24h:        number;
  system_healthy:         boolean;
  latency_p50_ms:         number | null;
  latency_p95_ms:         number | null;
  error_rate_24h:         number;
  degraded_components:    string[];
  recent_critical_events: string[];
  active_providers:       number;
  active_plugins:         number;
  memory_doc_count:       number;
  eval_runs_today:        number;
}

export interface RunCost {
  run_id:            string;
  total_cost_micros: number;
  total_tokens_in:   number;
  total_tokens_out:  number;
  provider_calls:    number;
}

export interface LlmTrace {
  trace_id:          string;
  model_id:          string;
  prompt_tokens:     number;
  completion_tokens: number;
  latency_ms:        number;
  cost_micros:       number;
  session_id:        string | null;
  run_id:            string | null;
  created_at_ms:     number;
  is_error:          boolean;
}

export interface AuditEntry {
  entry_id:      string;
  tenant_id:     string;
  actor_id:      string;
  action:        string;
  resource_type: string;
  resource_id:   string;
  outcome:       "success" | "failure";
  occurred_at_ms: number;
  metadata:      Record<string, unknown>;
}

export interface StoreSnapshot {
  version:        number;
  created_at_ms:  number;
  event_count:    number;
  events:         unknown[];
}

export interface WebhookTestResult {
  success:     boolean;
  status_code: number;
  latency_ms:  number;
}

/** Pagination metadata extracted from response headers. */
export interface PaginationMeta {
  totalCount: number | null;
  page:       number | null;
  perPage:    number | null;
  /** Raw `Link` header value */
  link:       string | null;
}

/** A paginated response wraps items with header-extracted metadata. */
export interface PagedResult<T> {
  items:      T[];
  pagination: PaginationMeta;
}

// ── Error type ────────────────────────────────────────────────────────────────

/** Thrown when the server returns a 4xx or 5xx response. */
export class CairnApiError extends Error {
  constructor(
    public readonly status:  number,
    public readonly code:    string,
    message: string,
  ) {
    super(message);
    this.name = "CairnApiError";
  }
}

// ── Client ────────────────────────────────────────────────────────────────────

/** Default lease duration for task claims: 30 seconds. */
const DEFAULT_LEASE_MS = 30_000;

/** Default Ollama model when none is specified. */
const DEFAULT_MODEL = "llama3";

/**
 * CairnClient — typed HTTP client for the cairn-rs control plane API.
 *
 * All methods are async and throw {@link CairnApiError} on HTTP 4xx/5xx.
 *
 * Pagination is surfaced through {@link PagedResult} on list methods.
 * Raw arrays are returned when the endpoint does not paginate.
 */
export class CairnClient {
  /**
   * @param baseUrl - Root URL of the cairn server, e.g. `http://localhost:3000`
   * @param token   - Bearer token (set via `CAIRN_ADMIN_TOKEN` on the server)
   */
  constructor(
    private readonly baseUrl: string,
    private readonly token:   string,
  ) {}

  // ── Low-level helpers ───────────────────────────────────────────────────────

  private url(path: string, query?: Record<string, string | number | undefined>): string {
    const u = new URL(path, this.baseUrl.endsWith("/") ? this.baseUrl : this.baseUrl + "/");
    if (query) {
      for (const [k, v] of Object.entries(query)) {
        if (v !== undefined) u.searchParams.set(k, String(v));
      }
    }
    return u.toString();
  }

  private headers(extra?: Record<string, string>): HeadersInit {
    return {
      "Authorization": `Bearer ${this.token}`,
      "Content-Type":  "application/json",
      ...extra,
    };
  }

  /**
   * Execute a fetch and deserialize the JSON body.
   * Throws {@link CairnApiError} on HTTP 4xx/5xx.
   */
  private async fetch<T>(
    method:  string,
    path:    string,
    body?:   unknown,
    query?:  Record<string, string | number | undefined>,
  ): Promise<{ data: T; response: Response }> {
    const resp = await fetch(this.url(path, query), {
      method,
      headers: this.headers(),
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });

    if (!resp.ok) {
      let code = "api_error";
      let message = `HTTP ${resp.status}`;
      try {
        const err = await resp.json() as { code?: string; message?: string };
        code    = err.code    ?? code;
        message = err.message ?? message;
      } catch { /* body was not JSON */ }
      throw new CairnApiError(resp.status, code, message);
    }

    const data = await resp.json() as T;
    return { data, response: resp };
  }

  /** Parse X-Total-Count / X-Page / X-Per-Page / Link headers into metadata. */
  private static paginationMeta(resp: Response): PaginationMeta {
    const hdr = (n: string) => resp.headers.get(n);
    return {
      totalCount: hdr("x-total-count") ? Number(hdr("x-total-count")) : null,
      page:       hdr("x-page")        ? Number(hdr("x-page"))        : null,
      perPage:    hdr("x-per-page")    ? Number(hdr("x-per-page"))    : null,
      link:       hdr("link"),
    };
  }

  private async paged<T>(
    method:  string,
    path:    string,
    body?:   unknown,
    query?:  Record<string, string | number | undefined>,
  ): Promise<PagedResult<T>> {
    const { data, response } = await this.fetch<T[]>(method, path, body, query);
    return { items: data, pagination: CairnClient.paginationMeta(response) };
  }

  // ── Health ──────────────────────────────────────────────────────────────────

  /**
   * `GET /health` — unauthenticated liveness probe.
   * Safe to call before setting up a token.
   */
  async health(): Promise<{ ok: boolean }> {
    const resp = await fetch(this.url("/health"));
    return resp.json() as Promise<{ ok: boolean }>;
  }

  /**
   * `GET /v1/status` — authenticated health check including store and runtime
   * liveness plus server uptime.
   */
  async status(): Promise<SystemStatus> {
    const { data } = await this.fetch<SystemStatus>("GET", "/v1/status");
    return data;
  }

  /**
   * `GET /v1/stats` — system-wide counters: total events, sessions, runs,
   * tasks, active runs, pending approvals, and uptime.
   */
  async stats(): Promise<SystemStats> {
    const { data } = await this.fetch<SystemStats>("GET", "/v1/stats");
    return data;
  }

  /**
   * `GET /v1/dashboard` — aggregated real-time dashboard metrics including
   * active run/task counts, error rates, and latency percentiles.
   */
  async dashboard(): Promise<DashboardOverview> {
    const { data } = await this.fetch<DashboardOverview>("GET", "/v1/dashboard");
    return data;
  }

  // ── Sessions ────────────────────────────────────────────────────────────────

  /**
   * `POST /v1/sessions` — open a new session.
   *
   * @param params.tenant_id    - Tenant identifier
   * @param params.workspace_id - Workspace identifier
   * @param params.project_id   - Project identifier
   * @param params.session_id   - Optional explicit session ID (server generates one if omitted)
   */
  async createSession(params: {
    tenant_id:    string;
    workspace_id: string;
    project_id:   string;
    session_id?:  string;
  }): Promise<Session> {
    const { data } = await this.fetch<Session>("POST", "/v1/sessions", params);
    return data;
  }

  /**
   * `GET /v1/sessions` — list active sessions, most-recent first.
   * Returns a {@link PagedResult} with pagination header metadata.
   *
   * @param limit  - Maximum sessions to return (default 50)
   * @param offset - Skip this many results (default 0)
   */
  async listSessions(limit = 50, offset = 0): Promise<PagedResult<Session>> {
    return this.paged<Session>("GET", "/v1/sessions", undefined, { limit, offset });
  }

  /**
   * `GET /v1/sessions/:id/runs` — list all runs that belong to a session.
   *
   * @param sessionId - Session ID
   * @param limit     - Maximum runs to return (default 100)
   * @param offset    - Skip this many results (default 0)
   */
  async listSessionRuns(sessionId: string, limit = 100, offset = 0): Promise<PagedResult<Run>> {
    return this.paged<Run>("GET", `/v1/sessions/${sessionId}/runs`, undefined, { limit, offset });
  }

  /**
   * `GET /v1/sessions/:id/llm-traces` — LLM call traces for a session,
   * including per-call token counts, latency, and cost.
   *
   * @param sessionId - Session ID
   * @param limit     - Maximum traces to return (default 100)
   */
  async listSessionTraces(sessionId: string, limit = 100): Promise<LlmTrace[]> {
    const { data } = await this.fetch<{ traces: LlmTrace[] }>(
      "GET", `/v1/sessions/${sessionId}/llm-traces`, undefined, { limit },
    );
    return data.traces;
  }

  // ── Runs ────────────────────────────────────────────────────────────────────

  /**
   * `POST /v1/runs` — start a new run within an existing session.
   *
   * @param params.session_id    - Parent session ID
   * @param params.tenant_id     - Tenant (defaults to `"default"`)
   * @param params.workspace_id  - Workspace (defaults to `"default"`)
   * @param params.project_id    - Project (defaults to `"default"`)
   * @param params.run_id        - Optional explicit run ID
   * @param params.parent_run_id - Optional parent run ID for subagent runs
   */
  async createRun(params: {
    session_id:     string;
    tenant_id?:     string;
    workspace_id?:  string;
    project_id?:    string;
    run_id?:        string;
    parent_run_id?: string;
  }): Promise<Run> {
    const { data } = await this.fetch<Run>("POST", "/v1/runs", {
      tenant_id:    params.tenant_id    ?? "default",
      workspace_id: params.workspace_id ?? "default",
      project_id:   params.project_id   ?? "default",
      session_id:   params.session_id,
      run_id:       params.run_id,
      parent_run_id: params.parent_run_id,
    });
    return data;
  }

  /**
   * `GET /v1/runs/:id` — fetch a single run by ID.
   *
   * @param runId - Run ID
   */
  async getRun(runId: string): Promise<Run> {
    const { data } = await this.fetch<Run>("GET", `/v1/runs/${runId}`);
    return data;
  }

  /**
   * `GET /v1/runs` — list all runs, most-recent first.
   * Returns a {@link PagedResult} with pagination header metadata.
   *
   * @param limit  - Maximum runs to return (default 50)
   * @param offset - Skip this many results (default 0)
   */
  async listRuns(limit = 50, offset = 0): Promise<PagedResult<Run>> {
    return this.paged<Run>("GET", "/v1/runs", undefined, { limit, offset });
  }

  /**
   * `GET /v1/runs/:id/tasks` — list tasks belonging to a run.
   *
   * @param runId - Run ID
   */
  async listRunTasks(runId: string): Promise<Task[]> {
    const { data } = await this.fetch<Task[]>("GET", `/v1/runs/${runId}/tasks`);
    return data;
  }

  /**
   * `GET /v1/runs/:id/approvals` — list approvals associated with a run
   * (all states: pending and resolved).
   *
   * @param runId - Run ID
   */
  async listRunApprovals(runId: string): Promise<Approval[]> {
    const { data } = await this.fetch<Approval[]>("GET", `/v1/runs/${runId}/approvals`);
    return data;
  }

  /**
   * `GET /v1/runs/:id/cost` — accumulated cost for a run including total
   * tokens consumed and micro-dollar cost.
   *
   * @param runId - Run ID
   */
  async getRunCost(runId: string): Promise<RunCost> {
    const { data } = await this.fetch<RunCost>("GET", `/v1/runs/${runId}/cost`);
    return data;
  }

  /**
   * `POST /v1/runs/:id/pause` — pause a running run.
   *
   * @param runId  - Run ID
   * @param reason - Optional human-readable pause reason
   */
  async pauseRun(runId: string, reason?: string): Promise<Run> {
    const { data } = await this.fetch<Run>("POST", `/v1/runs/${runId}/pause`, {
      kind:   "operator_pause",
      detail: reason ?? null,
    });
    return data;
  }

  /**
   * `POST /v1/runs/:id/resume` — resume a paused run.
   *
   * @param runId - Run ID
   */
  async resumeRun(runId: string): Promise<Run> {
    const { data } = await this.fetch<Run>("POST", `/v1/runs/${runId}/resume`, {
      trigger: "operator",
    });
    return data;
  }

  // ── Tasks ───────────────────────────────────────────────────────────────────

  /**
   * `GET /v1/tasks` — list all tasks across all runs, most-recent first.
   * Returns a {@link PagedResult} with pagination header metadata.
   *
   * @param limit  - Maximum tasks to return (default 50)
   * @param offset - Skip this many results (default 0)
   */
  async listTasks(limit = 50, offset = 0): Promise<PagedResult<Task>> {
    return this.paged<Task>("GET", "/v1/tasks", undefined, { limit, offset });
  }

  /**
   * `POST /v1/tasks/:id/claim` — claim a queued task for a worker, acquiring
   * an exclusive lease.
   *
   * @param taskId          - Task ID
   * @param workerId        - Unique identifier for the claiming worker
   * @param leaseDurationMs - Lease TTL in milliseconds (default 30 000)
   */
  async claimTask(
    taskId:          string,
    workerId:        string,
    leaseDurationMs: number = DEFAULT_LEASE_MS,
  ): Promise<Task> {
    const { data } = await this.fetch<Task>("POST", `/v1/tasks/${taskId}/claim`, {
      worker_id:          workerId,
      lease_duration_ms:  leaseDurationMs,
    });
    return data;
  }

  /**
   * `POST /v1/tasks/:id/release-lease` — release a leased task back to the
   * queued state so another worker can claim it.
   *
   * @param taskId - Task ID
   */
  async releaseTask(taskId: string): Promise<Task> {
    const { data } = await this.fetch<Task>("POST", `/v1/tasks/${taskId}/release-lease`, {});
    return data;
  }

  /**
   * `POST /v1/tasks/batch/cancel` — cancel multiple tasks atomically.
   * Returns a summary of successes and failures.
   *
   * @param taskIds - Array of task IDs to cancel
   */
  async cancelTasks(taskIds: string[]): Promise<{ cancelled: string[]; failed: string[] }> {
    const { data } = await this.fetch<{ cancelled: string[]; failed: string[] }>(
      "POST", "/v1/tasks/batch/cancel", { task_ids: taskIds },
    );
    return data;
  }

  // ── Approvals ───────────────────────────────────────────────────────────────

  /**
   * `GET /v1/approvals/pending` — fetch all pending (undecided) approvals.
   * Returns a {@link PagedResult} with pagination header metadata.
   *
   * @param limit  - Maximum approvals to return (default 50)
   * @param offset - Skip this many results (default 0)
   */
  async listPendingApprovals(limit = 50, offset = 0): Promise<PagedResult<Approval>> {
    return this.paged<Approval>("GET", "/v1/approvals/pending", undefined, { limit, offset });
  }

  /**
   * `POST /v1/approvals/:id/resolve` — approve a pending approval request.
   *
   * @param approvalId - Approval ID
   * @param reason     - Optional explanation logged with the decision
   */
  async approve(approvalId: string, reason?: string): Promise<Approval> {
    const { data } = await this.fetch<Approval>(
      "POST", `/v1/approvals/${approvalId}/resolve`,
      { decision: "approved", reason: reason ?? null },
    );
    return data;
  }

  /**
   * `POST /v1/approvals/:id/resolve` — reject a pending approval request.
   *
   * @param approvalId - Approval ID
   * @param reason     - Optional explanation logged with the decision
   */
  async reject(approvalId: string, reason?: string): Promise<Approval> {
    const { data } = await this.fetch<Approval>(
      "POST", `/v1/approvals/${approvalId}/resolve`,
      { decision: "rejected", reason: reason ?? null },
    );
    return data;
  }

  // ── LLM ────────────────────────────────────────────────────────────────────

  /**
   * `POST /v1/providers/ollama/generate` — send a single-turn prompt to an
   * Ollama-hosted model and receive a complete (non-streaming) response.
   *
   * Requires the server to be started with `OLLAMA_HOST` set.
   *
   * @param prompt - The user prompt
   * @param model  - Model name (default `"llama3"`)
   */
  async generate(prompt: string, model: string = DEFAULT_MODEL): Promise<GenerateResponse> {
    const { data } = await this.fetch<GenerateResponse>(
      "POST", "/v1/providers/ollama/generate", { model, prompt },
    );
    return data;
  }

  /**
   * `POST /v1/providers/ollama/stream` — stream a response from Ollama
   * token-by-token via Server-Sent Events.
   *
   * Returns an `AsyncGenerator<string>` that yields each text fragment as
   * it arrives. The generator completes when the model finishes.
   *
   * @param prompt - The user prompt
   * @param model  - Model name (default `"llama3"`)
   *
   * @example
   * ```typescript
   * for await (const token of cairn.streamGenerate("Tell me a story.")) {
   *   process.stdout.write(token);
   * }
   * ```
   */
  async *streamGenerate(prompt: string, model: string = DEFAULT_MODEL): AsyncGenerator<string> {
    const resp = await fetch(this.url("/v1/providers/ollama/stream"), {
      method:  "POST",
      headers: this.headers(),
      body:    JSON.stringify({ model, prompt }),
    });

    if (!resp.ok) {
      throw new CairnApiError(resp.status, "stream_error", `HTTP ${resp.status}`);
    }

    if (!resp.body) return;

    const reader  = resp.body.getReader();
    const decoder = new TextDecoder();
    let buffer    = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        if (line.startsWith("data: ")) {
          const raw = line.slice(6).trim();
          if (raw === "[DONE]") return;
          try {
            const chunk = JSON.parse(raw) as { token?: string; text?: string; content?: string };
            const text = chunk.token ?? chunk.text ?? chunk.content;
            if (text) yield text;
          } catch { /* skip malformed frames */ }
        }
      }
    }
  }

  /**
   * `GET /v1/providers/ollama/models` — list models available in the local
   * Ollama registry.
   *
   * @returns Array of model name strings, or empty array if Ollama is not configured.
   */
  async listModels(): Promise<string[]> {
    const { data } = await this.fetch<{ models: string[]; count: number; host: string }>(
      "GET", "/v1/providers/ollama/models",
    );
    return data.models;
  }

  // ── Events ──────────────────────────────────────────────────────────────────

  /**
   * `POST /v1/events/append` — append raw `RuntimeEvent` envelopes to the
   * event log. Used when driving the store directly from an agent or test harness.
   *
   * Returns positions assigned to each appended event.
   *
   * @param envelopes - Array of event envelope objects matching the `EventEnvelope<RuntimeEvent>` schema.
   */
  async appendEvents(
    envelopes: Array<{
      event_id:       string;
      source:         { source_type: string };
      ownership:      { scope: string; tenant_id: string; workspace_id: string; project_id: string };
      causation_id:   string | null;
      correlation_id: string | null;
      payload:        Record<string, unknown>;
    }>,
  ): Promise<Array<{ event_id: string; position: number; appended: boolean }>> {
    const { data } = await this.fetch<Array<{ event_id: string; position: number; appended: boolean }>>(
      "POST", "/v1/events/append", envelopes,
    );
    return data;
  }

  /**
   * `GET /v1/events/recent` — the most recent events from the live event log.
   *
   * @param limit - Maximum events to return (default 50)
   */
  async recentEvents(limit = 50): Promise<Array<{ seq: number; event_type: string; data: unknown; timestamp: string }>> {
    const { data } = await this.fetch<Array<{ seq: number; event_type: string; data: unknown; timestamp: string }>>(
      "GET", "/v1/events/recent", undefined, { limit },
    );
    return data;
  }

  // ── LLM Traces ──────────────────────────────────────────────────────────────

  /**
   * `GET /v1/traces` — all recent LLM call traces (operator view).
   * Returns a {@link PagedResult} with pagination header metadata.
   *
   * @param limit  - Maximum traces to return, capped at 500 (default 100)
   * @param offset - Skip this many results (default 0)
   */
  async listTraces(limit = 100, offset = 0): Promise<PagedResult<LlmTrace>> {
    const { data, response } = await this.fetch<{ traces: LlmTrace[] }>(
      "GET", "/v1/traces", undefined, { limit, offset },
    );
    return { items: data.traces, pagination: CairnClient.paginationMeta(response) };
  }

  // ── Audit log ───────────────────────────────────────────────────────────────

  /**
   * `GET /v1/admin/audit-log` — structured audit log for the operator's tenant.
   *
   * @param limit  - Maximum entries to return (default 100)
   * @param offset - Skip this many results (default 0)
   */
  async listAuditLog(limit = 100, offset = 0): Promise<AuditEntry[]> {
    const { data } = await this.fetch<{ entries: AuditEntry[] }>(
      "GET", "/v1/admin/audit-log", undefined, { limit, offset },
    );
    return data.entries ?? (data as unknown as AuditEntry[]);
  }

  // ── Admin: snapshot / restore ───────────────────────────────────────────────

  /**
   * `POST /v1/admin/snapshot` — export the full in-memory event log as a
   * downloadable JSON snapshot.
   *
   * The returned object can be stored and later passed to {@link restore}.
   */
  async snapshot(): Promise<StoreSnapshot> {
    const { data } = await this.fetch<StoreSnapshot>("POST", "/v1/admin/snapshot", {});
    return data;
  }

  /**
   * `POST /v1/admin/restore` — clear all state and replay events from a
   * previously taken snapshot. **Irreversible** — take a snapshot first.
   *
   * @param snap - Snapshot returned by {@link snapshot}
   */
  async restore(snap: StoreSnapshot): Promise<{ ok: boolean; replayed: number }> {
    const { data } = await this.fetch<{ ok: boolean; replayed: number }>(
      "POST", "/v1/admin/restore", snap,
    );
    return data;
  }

  // ── Webhook test ────────────────────────────────────────────────────────────

  /**
   * `POST /v1/test/webhook` — send a test payload to an operator-supplied
   * webhook URL to verify channel configuration.
   *
   * @param url        - Target webhook URL (Slack, Discord, etc.)
   * @param eventType  - Event type string included in the test payload
   */
  async testWebhook(url: string, eventType: string): Promise<WebhookTestResult> {
    const { data } = await this.fetch<WebhookTestResult>(
      "POST", "/v1/test/webhook", { url, event_type: eventType },
    );
    return data;
  }

  // ── SSE subscription ────────────────────────────────────────────────────────

  /**
   * `GET /v1/stream` — subscribe to the real-time event stream via
   * Server-Sent Events.
   *
   * The token is passed as a query parameter because browsers cannot send
   * custom headers during an `EventSource` upgrade.
   *
   * @param onEvent - Callback invoked for every received event
   * @returns Unsubscribe function — call it to close the SSE connection
   *
   * @example
   * ```typescript
   * const unsub = cairn.subscribeEvents((event) => {
   *   console.log("event:", event.type, event.data);
   * });
   * // Later:
   * unsub();
   * ```
   */
  subscribeEvents(onEvent: (event: MessageEvent) => void): () => void {
    const url = this.url("/v1/stream", { token: this.token });

    // EventSource is available in browsers and Node.js ≥18 (with --experimental-fetch).
    // For environments without a global EventSource, polyfill before calling this method.
    const es = new EventSource(url);

    es.onmessage = onEvent;
    es.onerror   = (err) => {
      console.error("[cairn-client] SSE error", err);
    };

    return () => es.close();
  }

  /**
   * `GET /v1/ws` — WebSocket-based alternative to SSE for environments that
   * require bidirectional communication or event-type filtering.
   *
   * Returns a native `WebSocket`. The token is passed as `?token=…` because
   * browser WebSocket upgrades cannot carry `Authorization` headers.
   *
   * @param onMessage - Callback invoked for every received message
   * @param onClose   - Optional callback invoked when the socket closes
   * @returns The raw WebSocket instance
   */
  connectWebSocket(
    onMessage: (data: unknown) => void,
    onClose?:  () => void,
  ): WebSocket {
    const url = this.url("/v1/ws", { token: this.token }).replace(/^http/, "ws");
    const ws  = new WebSocket(url);

    ws.onmessage = (e) => {
      try { onMessage(JSON.parse(e.data as string)); }
      catch { onMessage(e.data); }
    };

    if (onClose) ws.onclose = onClose;

    return ws;
  }
}
