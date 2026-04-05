/**
 * TypeScript interfaces matching cairn-rs backend JSON shapes.
 *
 * Field names match the serde output exactly (snake_case for Rust structs,
 * camelCase only where the Rust side uses #[serde(rename_all = "camelCase")]).
 */

// ── Health ────────────────────────────────────────────────────────────────────

export interface HealthResponse {
  ok: boolean;
}

// ── System status ─────────────────────────────────────────────────────────────

/** GET /v1/status */
export interface SystemStatus {
  runtime_ok: boolean;
  store_ok: boolean;
  uptime_secs: number;
}

// ── Dashboard ─────────────────────────────────────────────────────────────────

/** GET /v1/dashboard */
export interface DashboardOverview {
  active_runs: number;
  active_tasks: number;
  pending_approvals: number;
  failed_runs_24h: number;
  system_healthy: boolean;
  latency_p50_ms: number | null;
  latency_p95_ms: number | null;
  error_rate_24h: number;
  degraded_components: string[];
  recent_critical_events: string[];
  active_providers: number;
  active_plugins: number;
  memory_doc_count: number;
  eval_runs_today: number;
}

// ── Project key ───────────────────────────────────────────────────────────────

export interface ProjectKey {
  tenant_id: string;
  workspace_id: string;
  project_id: string;
}

// ── Sessions ──────────────────────────────────────────────────────────────────

/** Session lifecycle state — mirrors cairn_domain::SessionState */
export type SessionState = "open" | "completed" | "failed" | "archived";

/** GET /v1/sessions — array of SessionRecord */
export interface SessionRecord {
  session_id: string;
  project: ProjectKey;
  state: SessionState;
  version: number;
  created_at: number; // unix ms
  updated_at: number; // unix ms
}

// ── Runs ──────────────────────────────────────────────────────────────────────

/** Run lifecycle state — mirrors cairn_domain::RunState */
export type RunState =
  | "pending"
  | "running"
  | "paused"
  | "waiting_approval"
  | "waiting_dependency"
  | "completed"
  | "failed"
  | "canceled";

/** Failure classification */
export type FailureClass =
  | "provider_failure"
  | "policy_denied"
  | "timeout"
  | "internal_error"
  | "approval_rejected";

/** GET /v1/runs — array of RunRecord */
export interface RunRecord {
  run_id: string;
  session_id: string;
  parent_run_id: string | null;
  project: ProjectKey;
  state: RunState;
  prompt_release_id: string | null;
  agent_role_id: string | null;
  failure_class: FailureClass | null;
  pause_reason: string | null;
  resume_trigger: string | null;
  version: number;
  created_at: number; // unix ms
  updated_at: number; // unix ms
}

// ── Run sub-resources ─────────────────────────────────────────────────────────

/** One entry from GET /v1/runs/:id/events */
export interface RunEventSummary {
  position: number;
  stored_at: number;
  event_type: string;
}

/** GET /v1/runs/:id/cost */
export interface RunCostRecord {
  run_id: string;
  total_cost_micros: number;
  total_tokens_in: number;
  total_tokens_out: number;
  provider_calls: number;
}

/** Task state mirrors cairn_domain::TaskState */
export type TaskState =
  | 'queued' | 'leased' | 'running' | 'completed'
  | 'failed' | 'canceled' | 'paused'
  | 'waiting_dependency' | 'retryable_failed' | 'dead_lettered';

/** One record from GET /v1/runs/:id/tasks or GET /v1/tasks */
export interface TaskRecord {
  task_id: string;
  project: { tenant_id: string; workspace_id: string; project_id: string };
  parent_run_id: string | null;
  parent_task_id: string | null;
  state: TaskState;
  failure_class: string | null;
  lease_owner: string | null;
  lease_expires_at: number | null;
  version: number;
  created_at: number;
  updated_at: number;
}

// ── Settings ──────────────────────────────────────────────────────────────────

/** System health aggregate (RFC 014) */
export interface SystemHealthSettings {
  provider_health_count: number;
  plugin_health_count: number;
  degraded_count: number;
  credential_count: number;
}

/** Encryption key management status (RFC 014) */
export interface KeyManagementStatus {
  encryption_key_configured: boolean;
  key_version: number | null;
  last_rotation_at: number | null; // unix ms
}

/** GET /v1/settings — full deployment settings */
export interface DeploymentSettings {
  deployment_mode: "local" | "self_hosted_team";
  store_backend: "memory" | "sqlite" | "postgres";
  plugin_count: number;
  system_health: SystemHealthSettings;
  key_management: KeyManagementStatus;
}

// ── Overview ──────────────────────────────────────────────────────────────────

/** GET /v1/overview — combined status + deployment info */
export interface OverviewResponse {
  deployment_mode: string;
  store_backend: string;
  uptime_secs: number;
  runtime_ok: boolean;
  store_ok: boolean;
}

// ── Costs ─────────────────────────────────────────────────────────────────────

/** GET /v1/costs */
export interface CostSummary {
  total_provider_calls: number;
  total_tokens_in: number;
  total_tokens_out: number;
  total_cost_micros: number;
}

// ── Approvals ─────────────────────────────────────────────────────────────────

export type ApprovalDecision = "approved" | "rejected";
export type ApprovalRequirement = "required" | "advisory";

export interface ApprovalRecord {
  approval_id: string;
  project: ProjectKey;
  run_id: string | null;
  task_id: string | null;
  requirement: ApprovalRequirement;
  decision: ApprovalDecision | null;
  created_at: number; // unix ms
}

// ── Memory / Knowledge ───────────────────────────────────────────────────────

/** One chunk returned by /v1/memory/search */
export interface MemoryChunkResult {
  score: number;
  chunk: {
    chunk_id: string;
    document_id: string;
    source_id: string;
    source_type: string;
    text: string;
    position: number;
    created_at: number;
    content_hash: string | null;
    credibility_score: number | null;
  };
  breakdown: {
    lexical_relevance: number;
    freshness: number;
    source_credibility: number;
  };
}

/** GET /v1/memory/search response */
export interface MemorySearchResponse {
  results: MemoryChunkResult[];
  diagnostics?: {
    mode_used: string;
    results_returned: number;
    candidates_generated: number;
    latency_ms: number;
  };
}

/** One entry from GET /v1/sources */
export interface SourceRecord {
  source_id: string;
  document_count: number;
  avg_quality_score: number;
  last_ingested_at_ms: number | null;
}

/** GET /v1/sources/:id/quality */
export interface SourceQualityRecord {
  source_id: string;
  credibility_score: number;
  total_retrievals: number;
  avg_rating: number | null;
  chunk_count: number;
}

// ── Generic list response ─────────────────────────────────────────────────────

/** Paginated list wrapper used by some endpoints */
export interface ListResponse<T> {
  items: T[];
  has_more: boolean;
}

// ── LLM Traces ────────────────────────────────────────────────────────────────

/** GET /v1/traces or GET /v1/sessions/:id/llm-traces — one call per trace */
export interface LlmCallTrace {
  trace_id: string;
  model_id: string;
  prompt_tokens: number;
  completion_tokens: number;
  latency_ms: number;
  cost_micros: number;
  session_id: string | null;
  run_id: string | null;
  created_at_ms: number;
  is_error: boolean;
}

export interface TracesResponse {
  traces: LlmCallTrace[];
}
