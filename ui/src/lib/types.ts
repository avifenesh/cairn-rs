/**
 * TypeScript interfaces matching cairn-rs backend JSON shapes.
 *
 * Field names match the serde output exactly (snake_case for Rust structs,
 * camelCase only where the Rust side uses #[serde(rename_all = "camelCase")]).
 */

// ── Provider connections (GET/POST /v1/providers/connections) ────────────────

export interface ProviderConnectionRecord {
  provider_connection_id: string;
  tenant_id: string;
  provider_family: string;
  adapter_type: string;
  supported_models: string[];
  status: "active" | "disabled";
  created_at: number;
}

// ── Agent templates (GET /v1/agent-templates) ────────────────────────────────

export interface AgentTemplate {
  id: string;
  name: string;
  description: string;
  icon: string;
  default_prompt: string;
  default_tools: string[];
  approval_policy: "none" | "sensitive" | "all";
  agent_role: string;
}

// ── Skills (GET /v1/skills) ─────────────────────────────────────────────────

export interface SkillRecord {
  id?: string;
  name?: string;
  description?: string;
  enabled?: boolean;
  source?: string;
  [key: string]: unknown;
}

export interface SkillsSummary {
  total: number;
  enabled: number;
  disabled: number;
}

export interface SkillsResponse {
  items: SkillRecord[];
  summary: SkillsSummary;
  currently_active: string[];
}

// ── Graph (GET /v1/graph/trace) ─────────────────────────────────────────────

export type GraphNodeKind =
  | "session"
  | "run"
  | "task"
  | "approval"
  | "checkpoint"
  | "mailbox_message"
  | "tool_invocation"
  | "memory"
  | "document"
  | "chunk"
  | "source"
  | "prompt_asset"
  | "prompt_version"
  | "prompt_release"
  | "eval_run"
  | "skill"
  | "channel_target"
  | "signal"
  | "ingest_job"
  | "route_decision"
  | "provider_call";

export type GraphEdgeKind =
  | "triggered"
  | "spawned"
  | "depended_on"
  | "approved_by"
  | "resumed_from"
  | "sent_to"
  | "read_from"
  | "cited"
  | "derived_from"
  | "embedded_as"
  | "evaluated_by"
  | "released_as"
  | "rolled_back_to"
  | "routed_to"
  | "used_prompt"
  | "used_tool"
  | "called_provider";

export interface GraphNodeRecord {
  node_id: string;
  kind: GraphNodeKind;
  project?: ProjectKey | null;
  created_at: number;
}

export interface GraphEdgeRecord {
  source_node_id: string;
  target_node_id: string;
  kind: GraphEdgeKind;
  created_at: number;
  confidence?: number | null;
}

export interface GraphTraceResponse {
  nodes: GraphNodeRecord[];
  edges: GraphEdgeRecord[];
  root?: string | null;
}

// ── Provider registry (GET /v1/providers/registry) ──────────────────────────

export interface ProviderRegistryModel {
  id: string;
  context_window: number;
  capabilities: {
    streaming: boolean;
    tool_use: boolean;
    vision: boolean;
    thinking: boolean;
  };
  /** Cost in USD per 1 million input tokens. 0 = free / unknown. */
  input_cost_per_1m?: number;
  /** Cost in USD per 1 million output tokens. 0 = free / unknown. */
  output_cost_per_1m?: number;
}

export interface ProviderRegistryEntry {
  id: string;
  name: string;
  api_base: string;
  api_format: string;
  default_model: string;
  available: boolean;
  requires_key: boolean;
  env_keys: string[];
  models: ProviderRegistryModel[];
}

// ── Provider health (GET /v1/providers/health) ───────────────────────────────

export interface ProviderHealthEntry {
  connection_id: string;
  status: string;
  healthy: boolean;
  last_checked_at: number;
  consecutive_failures: number;
  error_message: string | null;
}

// ── Health ────────────────────────────────────────────────────────────────────

export interface HealthResponse {
  ok?: boolean;
  status?: string;
  store_ok?: boolean;
  version?: string;
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
  ollama?: HealthCheckEntry;
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
export interface SystemStatusComponent {
  name: string;
  status: string;
  message: string | null;
}

export interface SystemStatus {
  status: string;
  version?: string;
  uptime_secs: number;
  components: SystemStatusComponent[];
}

/** Derive overall runtime health from status response. */
export function isRuntimeHealthy(s: SystemStatus | null | undefined): boolean {
  if (!s) return false;
  return s.status === 'ok';
}

/** Derive store health from status response. */
export function isStoreHealthy(s: SystemStatus | null | undefined): boolean {
  if (!s) return false;
  const store = s.components?.find(c => c.name === 'event_store');
  return store ? store.status === 'ok' : true;
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

// ── Workspaces ────────────────────────────────────────────────────────────────

/** GET /v1/admin/tenants/:tenant_id/workspaces — persisted workspace record. */
export interface WorkspaceRecord {
  workspace_id: string;
  tenant_id: string;
  name: string;
  created_at: number; // unix ms
  updated_at: number; // unix ms
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
  /** RFC 018: execution mode (direct/plan/execute) */
  mode?: string | { type: string; plan_run_id?: string };
  /** RFC 022: trigger that created this run */
  created_by_trigger_id?: string;
  /** RFC 016: sandbox ID if run has a sandbox */
  sandbox_id?: string;
  /** RFC 016: sandbox filesystem path */
  sandbox_path?: string;
}

// ── Run sub-resources ─────────────────────────────────────────────────────────

/** One entry from GET /v1/runs/:id/events */
export interface RunEventSummary {
  position: number;
  /** Backend field name is occurred_at_ms; stored_at kept for compatibility. */
  occurred_at_ms: number;
  stored_at: number;
  event_type: string;
  description?: string;
}

/** Paginated wrapper returned by GET /v1/runs/:id/events (without legacy `from` param). */
export interface EventsPage {
  events: RunEventSummary[];
  next_cursor: number | null;
  has_more: boolean;
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

/** GET /v1/settings — actual response is sparse; optional fields may be absent */
export interface DeploymentSettings {
  deployment_mode: string;
  store_backend: string;
  plugin_count: number;
  system_health?: SystemHealthSettings;
  key_management?: KeyManagementStatus;
}

// ── Overview ──────────────────────────────────────────────────────────────────

/** GET /v1/overview — combined status + deployment info */
export interface OverviewResponse {
  deployment_mode: string;
  store_backend: string;
  uptime_secs: number;
  status?: string;
  components?: SystemStatusComponent[];
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

/** One entry from GET /v1/events/recent. */
export interface RecentEvent {
  position?: number;
  seq?: number;
  event_type: string;
  message?: string;
  data?: unknown;
  timestamp?: string;
  stored_at?: number;
  run_id?: string | null;
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
  project_id?: string;
  project?: { tenant_id: string; workspace_id: string; project_id: string };
  subject_kind: string;
  evaluator_type: string;
  status?: string;
  success: boolean | null;
  error_message: string | null;
  started_at: number;    // unix ms — mapped from created_at
  completed_at: number | null;
}

export interface EvalRunsResponse {
  items: EvalRunRecord[];
  has_more?: boolean;
  hasMore?: boolean;
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

// ── Request log (GET /v1/admin/logs) ──────────────────────────────────────────

/** One structured request log entry from the in-memory ring buffer. */
export interface RequestLogEntry {
  timestamp:  string;
  level:      'info' | 'warn' | 'error';
  message:    string;
  request_id: string;
  method:     string;
  path:       string;
  query:      string | null;
  status:     number;
  latency_ms: number;
}

export interface RequestLogsResponse {
  entries: RequestLogEntry[];
  total:   number;
  limit:   number;
}

// ── Notifications ─────────────────────────────────────────────────────────────

export type NotifType =
  | 'approval_requested'
  | 'approval_resolved'
  | 'run_completed'
  | 'run_failed'
  | 'task_stuck';

export interface Notification {
  id:         string;
  type:       NotifType;
  message:    string;
  entity_id?: string;
  href:       string;
  read:       boolean;
  created_at: number; // unix ms
}

export interface NotifListResponse {
  notifications: Notification[];
  unread_count:  number;
}

// ── System Info (GET /v1/system/info) ─────────────────────────────────────────

export interface SystemInfoFeatures {
  sse_buffer_size:           number;
  rate_limit_per_minute:     number;
  ip_rate_limit_per_minute:  number;
  max_body_size_mb:          number;
  websocket_enabled:         boolean;
  ollama_connected?:         boolean;
  store_type:                string;
  postgres_enabled:          boolean;
  sqlite_enabled:            boolean;
  notification_buffer:       number;
}

export interface SystemInfoEnvironment {
  admin_token_set:   boolean;
  ollama_host?:      string;
  listen_addr:       string;
  deployment_mode:   string;
  uptime_seconds:    number;
}

export interface SystemInfo {
  version:      string;
  rust_version: string;
  build_date:   string;
  git_commit:   string;
  os:           string;
  arch:         string;
  features:     SystemInfoFeatures;
  environment:  SystemInfoEnvironment;
}

// ── Marketplace Catalog (RFC 015) ──────────────────────────────────────────────

/** GET /v1/plugins/catalog — one entry per marketplace plugin */
export interface CatalogEntry {
  id: string;
  name: string;
  version: string;
  description: string;
  category: string;
  vendor: string;
  state: string;
  tools_count: number;
  signals_count: number;
  download_url: string | null;
  has_signal_source: boolean;
}

/** Credential spec from plugin descriptor */
export interface CredentialSpec {
  key: string;
  label: string;
  scope: string;
  required: boolean;
}

/** Marketplace plugin detail (installed state) */
export interface MarketplacePluginDetail {
  plugin_id: string;
  state: string;
  descriptor: CatalogEntry & { required_credentials: CredentialSpec[] };
  installed_at: number | null;
}

// ── Plan Review (RFC 018) ─────────────────────────────────────────────────────

export interface PlanReviewRequest {
  approved_by?: string;
  rejected_by?: string;
  reason?: string;
  reviewer_comments?: string;
}

// ── Changelog ─────────────────────────────────────────────────────────────────

export interface ChangelogEntry {
  version: string;
  date:    string;
  changes: string[];
}

// ── Workers / Fleet (GAP-005) ─────────────────────────────────────────────────

/**
 * Lifecycle status for a registered external worker.
 *
 * The backend (`cairn-runtime::fleet::WorkerState::status`) is typed as
 * `String`, but the value set is closed: only "active", "suspended", or
 * "offline" are ever emitted. The `string & {}` branch preserves
 * forward-compatibility without collapsing the union to bare `string`
 * — call sites keep literal autocomplete on the known values while
 * still accepting a future backend addition without a type break.
 */
export type WorkerStatus = "active" | "suspended" | "offline" | (string & {});

/** Live health snapshot for a registered external worker. */
export interface WorkerHealth {
  /** Epoch-ms of the last received heartbeat (0 if no heartbeat yet). */
  last_heartbeat_ms: number;
  /** True when the worker sent a heartbeat within the configured TTL window. */
  is_alive: boolean;
  /** Number of tasks currently leased to this worker. */
  active_task_count: number;
}

/**
 * Registered external worker as returned by `GET /v1/workers` and
 * `GET /v1/workers/:id`. Mirrors `ExternalWorkerRecord` in cairn-domain.
 */
export interface WorkerRecord {
  worker_id:     string;
  tenant_id:     string;
  display_name:  string;
  status:        WorkerStatus;
  /** Epoch-ms when the worker first registered with the control plane. */
  registered_at: number;
  /** Epoch-ms of the last status/health mutation on the registry row. */
  updated_at:    number;
  health:        WorkerHealth;
  /**
   * The task currently leased to this worker, or `null` when idle.
   * The Rust handler serialises `Option<TaskId>` without
   * `skip_serializing_if`, so the field is always present in the wire
   * shape.
   */
  current_task_id: string | null;
}

/** Per-worker snapshot inside a fleet report. Mirrors `WorkerState` in cairn-runtime. */
export interface FleetWorkerState {
  worker_id:    string;
  display_name: string;
  status:       WorkerStatus;
  health:       WorkerHealth;
  /** Always present; `null` when the worker holds no lease. */
  current_task_id: string | null;
}

/**
 * Fleet aggregate as returned by `GET /v1/fleet`. Mirrors `FleetReport`
 * in cairn-runtime.
 */
export interface FleetReport {
  workers: FleetWorkerState[];
  total:   number;
  /** Workers whose status is "active". */
  active:  number;
  /** Workers that reported a heartbeat recently. */
  healthy: number;
}

// ── Project repos (RFC 016 — repo allowlist) ─────────────────────────────────

/** One entry in a project's repo allowlist. Mirrors
 *  `crates/cairn-app/src/repo_routes.rs :: RepoAllowlistEntryResponse`. */
export interface ProjectRepoEntry {
  /** Canonical "owner/repo" identifier. */
  repo_id: string;
  /** "present" once the local clone cache has hydrated; "missing" otherwise. */
  clone_status: string;
  added_by?: string | null;
  added_at?: number | null;
  last_used_at?: number | null;
}

/** Response from `POST /v1/projects/:project/repos` (`RepoMutationResponse`). */
export interface ProjectRepoMutation {
  project: string;
  repo_id: string;
  allowlisted: boolean;
  clone_status: string;
  clone_created: boolean;
}

/** Response from `GET /v1/projects/:project/repos/:owner/:repo`
 *  (`RepoDetailResponse`). */
export interface ProjectRepoDetail {
  project: string;
  repo_id: string;
  allowlisted: boolean;
  clone_status: string;
  added_by?: string | null;
  added_at?: number | null;
  last_used_at?: number | null;
  recent_sandbox_usage: string[];
  recent_register_repo_decisions: string[];
}
