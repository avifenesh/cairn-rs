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

// ── Generic list response ─────────────────────────────────────────────────────

/** Paginated list wrapper used by some endpoints */
export interface ListResponse<T> {
  items: T[];
  has_more: boolean;
}
