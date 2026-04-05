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

// ── Detailed health (GET /v1/health/detailed) ─────────────────────────────────

export interface HealthCheckEntry {
  status: 'healthy' | 'degraded' | 'unhealthy' | 'unconfigured';
  latency_ms?: number;
  models?: number;
  size?: number;
  capacity?: number;
  rss_mb?: number;
  heap_mb?: number;
}

export interface DetailedHealthChecks {
  store: HealthCheckEntry;
  ollama: HealthCheckEntry;
  event_buffer: HealthCheckEntry;
  memory: HealthCheckEntry;
}

export interface DetailedHealth {
  status: 'healthy' | 'degraded' | 'unhealthy';
  checks: DetailedHealthChecks;
  uptime_seconds: number;
  version: string;
  started_at: string;
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

// ── Recent events ─────────────────────────────────────────────────────────────

/** One entry from GET /v1/events/recent — includes SSE sequence ID. */
export interface RecentEvent {
  seq: number;
  event_type: string;
  data: unknown;
  timestamp: string;
}

// ── System stats ──────────────────────────────────────────────────────────────

/** GET /v1/stats — real-time system-wide counters. */
export interface SystemStats {
  total_events: number;
  total_sessions: number;
  total_runs: number;
  total_tasks: number;
  active_runs: number;
  pending_approvals: number;
  uptime_seconds: number;
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

// ── Prompts (RFC 006) ─────────────────────────────────────────────────────────

/** GET /v1/prompts/assets */
export interface PromptAssetRecord {
  prompt_asset_id: string;
  project: ProjectKey;
  name: string;
  kind: string;
  scope?: string;
  status?: string;
  created_at: number;
  updated_at?: number;
}

/** One template variable definition */
export interface PromptTemplateVar {
  name: string;
  description?: string;
  required?: boolean;
  default_value?: string;
}

/** GET /v1/prompts/assets/:id/versions */
export interface PromptVersionRecord {
  prompt_version_id: string;
  prompt_asset_id: string;
  project: ProjectKey;
  content_hash: string;
  version_number?: number;
  content?: string;
  template_vars?: PromptTemplateVar[];
  created_at: number;
}

/** GET /v1/prompts/releases */
export interface PromptReleaseRecord {
  prompt_release_id: string;
  project: ProjectKey;
  prompt_asset_id: string;
  prompt_version_id: string;
  state: string;
  rollout_percent?: number | null;
  routing_slot?: string | null;
  task_type?: string | null;
  agent_type?: string | null;
  is_project_default?: boolean;
  release_tag?: string | null;
  created_by?: string | null;
  created_at: number;
  updated_at: number;
}

/** GET /v1/prompts/assets/:id/versions/:vid/diff */
export interface PromptVersionDiff {
  added_lines: string[];
  removed_lines: string[];
  unchanged_lines: string[];
  similarity_score: number;
}

// ── Audit Log ─────────────────────────────────────────────────────────────────

export type AuditOutcome = 'success' | 'failure';

/** One entry from GET /v1/admin/audit-log */
export interface AuditRecord {
  entry_id: string;
  tenant_id: string;
  actor_id: string;
  action: string;
  resource_type: string;
  resource_id: string;
  outcome: AuditOutcome;
  occurred_at_ms: number;
  metadata: Record<string, unknown>;
}

export interface AuditLogResponse {
  items: AuditRecord[];
  has_more: boolean;
}

// ── Eval Runs ─────────────────────────────────────────────────────────────────

export type EvalRunStatus = 'pending' | 'running' | 'completed' | 'failed' | 'canceled';

/** One record from GET /v1/evals/runs */
export interface EvalRunRecord {
  eval_run_id: string;
  project: { tenant_id: string; workspace_id: string; project_id: string };
  subject_kind: string;
  evaluator_type: string;
  success: boolean | null;
  error_message: string | null;
  started_at: number;    // unix ms
  completed_at: number | null;
}

export interface EvalRunsResponse {
  items: EvalRunRecord[];
  has_more: boolean;
}

// ── Plugins ───────────────────────────────────────────────────────────────────

/** Discriminated union matching cairn_tools::PluginCapability */
export type PluginCapability =
  | { type: 'tool_provider';    tools: string[] }
  | { type: 'signal_source';    signals: string[] }
  | { type: 'channel_provider'; channels: string[] }
  | { type: 'post_turn_hook' }
  | { type: 'policy_hook' }
  | { type: 'eval_scorer' }
  | { type: 'mcp_server';       endpoint: unknown };

/** GET /v1/plugins — array item (manifest only, no lifecycle state) */
export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  command: string[];
  capabilities: PluginCapability[];
  permissions: unknown;
  limits: { max_concurrency?: number; default_timeout_ms?: number } | null;
  execution_class: string;
  description: string | null;
  homepage: string | null;
}

/** Lifecycle snapshot included in GET /v1/plugins/:id */
export interface PluginLifecycleSnapshot {
  plugin_id: string;
  /** PluginState variants: discovered | spawning | handshaking | ready | draining | stopped | failed */
  state: string;
  uptime_ms: number;
}

/** Performance metrics included in GET /v1/plugins/:id */
export interface PluginMetrics {
  plugin_id: string;
  invocation_count: number;
  error_count: number;
  avg_latency_ms: number;
}

/** One log entry from GET /v1/plugins/:id/logs */
export interface PluginLogEntry {
  plugin_id: string;
  level: string;
  message: string;
  timestamp_ms: number;
}

/** GET /v1/plugins/:id */
export interface PluginDetailResponse {
  manifest: PluginManifest;
  lifecycle: PluginLifecycleSnapshot;
  metrics: PluginMetrics;
}

// ── Credentials (RFC 011) ──────────────────────────────────────────────────────

/**
 * Credential metadata returned by GET /v1/admin/tenants/:id/credentials.
 * The actual encrypted_value is NEVER returned by the API — only metadata.
 */
export interface CredentialSummary {
  id: string;
  tenant_id: string;
  provider_id: string;
  name: string;
  /** e.g. "api_key", "oauth_token", "connection_string" */
  credential_type: string;
  key_version: string | null;
  key_id: string | null;
  /** unix ms when encryption was applied; null = stored in plaintext (dev only) */
  encrypted_at_ms: number | null;
  active: boolean;
  revoked_at_ms: number | null;
  created_at: number;
  updated_at: number;
}

/** POST /v1/admin/tenants/:id/credentials */
export interface StoreCredentialRequest {
  provider_id: string;
  plaintext_value: string;
  key_id?: string;
}

// ── Notification channels (RFC 007/014) ───────────────────────────────────────

/** Channel kind — mirrors cairn_channels::ChannelKind + pagerduty extension */
export type ChannelKind = 'webhook' | 'slack' | 'email' | 'pagerduty' | 'telegram' | 'plugin';

/** One channel entry inside a NotificationPreference */
export interface NotificationChannel {
  /** e.g. "webhook", "slack", "email", "pagerduty" */
  kind: string;
  /** Webhook URL, email address, Slack webhook URL, or PagerDuty routing key */
  target: string;
}

/** GET /v1/admin/operators/:id/notifications */
export interface NotificationPreference {
  pref_id: string;
  tenant_id: string;
  operator_id: string;
  /** Event type strings subscribed across all channels */
  event_types: string[];
  channels: NotificationChannel[];
}

/** One entry from GET /v1/admin/notifications/failed */
export interface NotificationRecord {
  record_id: string;
  tenant_id: string;
  operator_id: string;
  event_type: string;
  channel_kind: string;
  channel_target: string;
  payload: Record<string, unknown>;
  sent_at_ms: number;
  delivered: boolean;
  delivery_error: string | null;
}
