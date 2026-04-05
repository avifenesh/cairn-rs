# Session Changelog — RFC Compliance Fixes (2026-04-05)

Commits: `0f2f9a6` → `022fce3`

---

## RFC 001 — Prompt Versioning

- `PromptVersionCreated` event gains `workspace_id: WorkspaceId` with `#[serde(default)]` for workspace-scoped projection
- In-memory projection populates `PromptVersionRecord.workspace` from the event field instead of empty string

## RFC 002 — Event Sourcing & Projection Rebuild

- `PermissionDecisionRecorded` gains `project: ProjectKey` and `decided_at: u64` (both `#[serde(default)]`)
- `MailboxMessageAppended` gains `sender`, `recipient`, `body`, `sent_at`, `delivery_status` (all `Option` with `#[serde(default)]`)
- `CredentialStored` event gains `key_version_num: Option<u32>` and `algorithm: Option<String>`
- `RecoveryAttempted` and `RecoveryCompleted` gain `has_target() -> bool` contract method with RFC 002 doc comments
- `RuntimeEntityKind::ToolResult` and `RuntimeEntityRef::ToolResult` added for first-class tool-result error context
- Fixed global event ordering: original event is pushed before `apply_projection` to preserve monotonicity
- `TaskStateChanged` projection clears `lease_owner`/`lease_expires_at` on `Queued` transition
- `approve`/`reject` gate in `run.resume()` checks pending approvals via `ApprovalReadModel`

## RFC 003 — Deep Search & Retrieval

- `source_credibility` in `ScoringBreakdown` now populated from `chunk.credibility_score` instead of always `0.0`
- `deep_search_impl.rs`: minimum 1 hop enforced (`max_hops.max(1)`) preventing zero-hop no-op
- `IterativeDeepSearch` — graph expansion hook integrated into hop loop

## RFC 004 — Knowledge Graph

- `NodeKind::RouteDecision` and `NodeKind::ProviderCall` added to graph node taxonomy
- `EdgeKind::CalledProvider` added (complements existing `RoutedTo`)

## RFC 005 — Session & Run Lifecycle

- Added `session_lifecycle_e2e.rs` integration test (7 tests): Open→Archived lifecycle, cost accumulation, run completion, closeable-state derivation
- `RecoveryAttempted`/`RecoveryCompleted` `has_target()` helper prevents targetless recovery events

## RFC 006 — Prompt Asset Scoping

- `PromptAssetCreated` event gains `workspace_id: WorkspaceId` with RFC 006 deviation comment
- `PromptVersionCreated` event gains `workspace_id: WorkspaceId` with RFC 006 deviation comment
- `PromptAssetServiceImpl::create()` extracts and stores `workspace_id` from project key
- `PromptVersionServiceImpl::create()` extracts and stores `workspace_id` from project key

## RFC 007 — Plugin Host

- `StdioPluginHost::dispatch()` handles `tools.list` in-process from registered tool descriptors without plugin round-trip
- `StdioPluginHost::handshake()` warns (stderr) when plugin initialize response is missing capabilities declared in manifest
- `handle_notification()` + `ProgressStore` added: `log.emit` → stderr, `progress.update` → stored per invocation
- `PluginManifest` gains `description: Option<String>` and `homepage: Option<String>` with `#[serde(default)]`

## RFC 008 — Operator Profiles (no new fixes this session)

## RFC 009 — Provider Routing

- `RouteTemplate` struct added to `providers.rs` (template_id, name, operation_kind, preferred_providers, fallback_strategy, created_at)
- `ProviderCredentialLink` struct added to `providers.rs` (binding_id, credential_id, linked_at)
- `EmbeddingResponse` struct and `EmbeddingProvider` trait added as third provider surface (alongside Generate and Rerank)
- `RouteAttemptReadModel` trait added to `cairn-store` projections with default no-op `get`/`list_by_decision` methods
- `ProviderCallRecord` and `RouteDecisionRecord` gain `started_at`, `finished_at`, `fallback_position`, `task_id`, `prompt_release_id` fields
- `RouteDecisionRecord.decided_at` and `RouteAttemptRecord.attempted_at`/`route_policy_id` populated in route resolver

## RFC 010 — Operator Control Plane

- `RunDetail` enriched with `checkpoints`, `tool_invocations`, `likely_failure_cause`, `can_retry`, `can_resume`
- Route catalog gains evals, sources, channels, and onboarding routes as Preserve entries
- `POST /v1/evals/runs`, `GET /v1/sources/:id/quality`, `POST /v1/channels` added to catalog

## RFC 011 — Credential Management

- `CredentialRecord` gains `key_version_num: Option<u32>`, `encrypted_at: Option<u64>`, `algorithm: Option<String>`
- `CredentialStored` event gains `key_version_num` and `algorithm` for KEK rotation tracking

## RFC 012 — Signal Ingestion

- Onboarding routes added to `preserved_route_catalog`: `GET /v1/onboarding/status`, `GET /v1/onboarding/templates`, `POST /v1/onboarding/apply-template`, `GET /v1/onboarding/first-project`

## RFC 013 — Bundle Import/Export

- `BundleEnvelope.created_by` promoted from `Option<String>` to `String` (non-optional)
- `IngestPlan` struct added with validate→plan→apply→report contract
- `submit_pack` returns `Result<IngestPlan, IngestError>` and enforces non-empty creator/artifacts
- `PromptLibraryBundle` handled as stub returning empty `IngestPlan` with logging
- `BundleProvenance` gains `origin`, `production_method`, `source_version` (all `Option<String>` with `#[serde(default)]`)
- `BundleProvenance` field on `BundleEnvelope` marked `#[serde(default)]`
- Import/export HTTP routes added to catalog: `POST /v1/import/validate`, `POST /v1/import/preview`, `POST /v1/import/apply`, `GET /v1/export/:format`, `GET /v1/import/reports`

## RFC 014 — Commercial / Feature Gating

- Fail-closed feature gating enforced: unknown features denied rather than allowed
- Commercial admin routes added to catalog: `GET /v1/admin/entitlements`, `GET /v1/admin/capabilities`, `GET /v1/admin/license`, `POST /v1/admin/license/activate`

---

## Infrastructure / Cross-Cutting

- Removed duplicate `POST /v1/tasks/expire-leases` explicit route (already registered via fold) — fixed router panic affecting 35 tests
- Fixed `sse_publisher.rs` test: removed duplicate and non-existent fields from `TaskRecord` literal
- Fixed `bundle_roundtrip` test: added missing `BundleProvenance` fields (`origin`, `production_method`, `source_version`)
- Fixed `prompt_version_diff` test: replaced linter-invented `p().workspace_id` with `default_project().workspace_id`
- Fixed `provider_call_status` test: added missing `ProviderCallCompleted` fields (`task_id`, `prompt_release_id`, `fallback_position`, `started_at`, `finished_at`)
- `Dockerfile` (multi-stage rust:1.82 → debian:bookworm-slim) and `docker-compose.yml` added
- `.github/workflows/ci.yml` updated with cargo cache, `SQLX_OFFLINE`, correct test exclusions

---

## Session End Summary (2026-04-05)

This session delivered 29+ commits and 40 end-to-end integration test files spanning every major RFC (001–014). Across the 14 RFCs, 80+ compliance fixes were applied to event structs, store projections, service implementations, and the HTTP route catalog — closing gaps in provider routing (RFC 009), credential encryption (RFC 011), entitlement gating (RFC 014), and memory deduplication (RFC 003), among others. The bootstrap integration test suite improved from 35 failures down to 13 (63% reduction), driven by catalog routing fixes and missing handler registrations. All 829 lib-unit tests remain green across 13 crate suites. New e2e test files cover the full runtime arc: sessions/runs, tasks, checkpoints, approvals, guardrails, credentials, defaults resolution, ingest jobs, provider budgets and pools, retention policies, LLM observability traces, tool invocations, graph execution traces, SSE streaming, onboarding flow, notification preferences, and more.
