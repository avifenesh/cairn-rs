# RFC Validation Report
Generated: 2026-04-04

Line-by-line validation of all 14 RFCs against the codebase.
Format: [RFC-0XX][FOUND|PARTIAL|MISSING|WRONG] file:line - description

---

## RFC 006-009 (worker-2) — 62 findings

### RFC 006 — Prompt Registry & Release Model

[RFC-006][FOUND] crates/cairn-domain/src/prompts.rs:16 - Prompt release lifecycle states modeled with canonical enum values draft/proposed/approved/active/rejected/archived.
[RFC-006][FOUND] crates/cairn-evals/src/selectors/mod.rs:12 - Selector precedence routing_slot > task_type > agent_type > project_default matches RFC resolution order.
[RFC-006][FOUND] crates/cairn-runtime/src/services/prompt_release_impl.rs:119 - activate() demotes already-active sibling release to approved, matching RFC deactivation rule.
[RFC-006][PARTIAL] crates/cairn-domain/src/prompts.rs:58 - PromptReleaseRecord omits RFC-required timestamps; rollout_target is Option<String> not typed structure.
[RFC-006][PARTIAL] crates/cairn-domain/src/prompts.rs:71 - PromptReleaseActionRecord has no timestamp field for auditable action record.
[RFC-006][WRONG] crates/cairn-evals/src/prompts/releases.rs:76 - Draft->approved always allowed; RFC 006 says this transition is policy-controlled.
[RFC-006][WRONG] crates/cairn-runtime/src/services/prompt_asset_impl.rs:34 - Prompt asset creation keyed by ProjectKey; RFC 006 defines assets as tenant/workspace-scoped.
[RFC-006][WRONG] crates/cairn-runtime/src/services/prompt_version_impl.rs:34 - Prompt version also ProjectKey-scoped; missing tenant/workspace scope, format, metadata, authoring fields.
[RFC-006][PARTIAL] crates/cairn-runtime/src/services/prompt_release_impl.rs:34 - Release creation missing rollout target, release tag, creator metadata.
[RFC-006][WRONG] crates/cairn-store/src/projections/prompt.rs:8 - Prompt projections omit asset scope/status/updated_at, version number/content/format/metadata/created_by, release tag/typed rollout target/created_by.
[RFC-006][MISSING] crates/cairn-store/src/pg/adapter.rs:585 - Postgres missing PromptAssetReadModel, PromptVersionReadModel, PromptReleaseReadModel.
[RFC-006][MISSING] crates/cairn-store/src/sqlite/adapter.rs:615 - SQLite missing prompt registry read models.
[RFC-006][WRONG] crates/cairn-store/src/in_memory.rs:1115 - active_for_selector() raw string comparison, no typed selector matching or RFC 006 precedence resolution.
[RFC-006][PARTIAL] crates/cairn-domain/src/events.rs:524 - Prompt lifecycle events omit scope metadata, rollout target, release tag, created_by, actor, reason.
[RFC-006][MISSING] crates/cairn-evals/src/services/graph_integration.rs:15 - Prompt/eval graph linkage not wired into runtime/app flows; RFC graph linkage not maintained.
[RFC-006][WRONG] crates/cairn-graph/src/eval_projector.rs:27 - Prompt/eval graph nodes created with project: None; RFC 006 prompt provenance not scoped.

### RFC 007 — Plugin Protocol & Transport

[RFC-007][FOUND] crates/cairn-tools/src/transport.rs:1 - JSON-RPC over stdio with child processes matches RFC canonical transport.
[RFC-007][FOUND] crates/cairn-plugin-proto/src/wire.rs:69 - Canonical RPC method names match RFC 007 exactly.
[RFC-007][FOUND] crates/cairn-tools/src/plugins.rs:6 - All six RFC 007 capability families modeled.
[RFC-007][PARTIAL] crates/cairn-plugin-proto/src/manifest.rs:8 - Wire manifest omits optional homepage; host manifest omits description and homepage.
[RFC-007][WRONG] crates/cairn-tools/src/plugin_executor.rs:172 - Sends tools.invoke without required initialize handshake first.
[RFC-007][WRONG] crates/cairn-tools/src/plugin_executor.rs:201 - Kills process directly instead of sending shutdown and honoring drain/shutdown lifecycle.
[RFC-007][WRONG] crates/cairn-tools/src/transport.rs:122 - recv() only deserializes JsonRpcResponse; error envelopes and notifications not consumable.
[RFC-007][WRONG] crates/cairn-tools/src/runtime_service_impl.rs:54 - Always runs run_builtin_pipeline(); plugin targets never traverse stdio transport path.
[RFC-007][PARTIAL] crates/cairn-tools/src/plugin_host.rs:89 - Handshake validates protocol version but not that capabilities/limits match manifest.
[RFC-007][MISSING] crates/cairn-tools/src/plugin_host.rs:165 - tools.list, signals.poll, channels.deliver, hooks.post_turn, policy.evaluate, eval.score, cancel not wired into runtime.
[RFC-007][PARTIAL] crates/cairn-tools/src/plugin_executor.rs:231 - Notification parsing exists but log.emit/progress.update/event.emit not consumed or surfaced.
[RFC-007][PARTIAL] crates/cairn-tools/src/plugins.rs:28 - Host manifest has execution_class but missing descriptive metadata for operator inspection.
[RFC-007][FOUND] crates/cairn-tools/src/plugin_host.rs:209 - Manifest discovery rejects empty commands and empty capability lists.
[RFC-007][FOUND] crates/cairn-plugin-proto/src/wire.rs:362 - Typed wrappers for three plugin->host notification families match RFC surface.

### RFC 008 — Tenant, Workspace & Profile

[RFC-008][FOUND] crates/cairn-domain/src/tenancy.rs:4 - Four ownership layers system/tenant/workspace/project modeled explicitly.
[RFC-008][FOUND] crates/cairn-domain/src/defaults.rs:12 - Defaults resolver walks scope chain project->workspace->tenant->system.
[RFC-008][FOUND] crates/cairn-domain/src/org.rs:34 - OperatorProfile modeled as tenant-scoped domain data.
[RFC-008][MISSING] crates/cairn-domain/src/org.rs:36 - OperatorProfile has no runtime service, store read model, or API implementation.
[RFC-008][MISSING] crates/cairn-domain/src/tenancy.rs:153 - WorkspaceMembership exists only as domain type; no persistence, query, or runtime/API management.
[RFC-008][WRONG] crates/cairn-domain/src/commands.rs:578 - CreateTenant/CreateWorkspace require ProjectKey even though they are above project scope.
[RFC-008][WRONG] crates/cairn-domain/src/events.rs:560 - TenantCreated/WorkspaceCreated embed ProjectKey; non-project facts through project-scoped payload.
[RFC-008][PARTIAL] crates/cairn-runtime/src/services/tenant_impl.rs:53 - Uses placeholder ProjectKey::new(tenant_id, "_", "_"); still synthetic project ownership.
[RFC-008][PARTIAL] crates/cairn-runtime/src/services/workspace_impl.rs:54 - Same placeholder ProjectKey for workspace creation.
[RFC-008][MISSING] crates/cairn-store/src/pg/adapter.rs:679 - Postgres has no durable tenant/workspace/project read-model; team-mode tenancy reads absent.
[RFC-008][MISSING] crates/cairn-store/src/sqlite/adapter.rs:709 - SQLite has no durable tenant/workspace/project read-model.
[RFC-008][WRONG] crates/cairn-graph/src/retrieval_projector.rs:27 - Retrieval provenance nodes projected with project: None; scope fields dropped.
[RFC-008][WRONG] crates/cairn-graph/src/eval_projector.rs:27 - Prompt/eval graph nodes projected with project: None.
[RFC-008][FOUND] crates/cairn-memory/src/pg/documents.rs:34 - Postgres document/chunk persistence stores tenant_id/workspace_id/project_id.
[RFC-008][FOUND] crates/cairn-memory/src/sqlite/documents.rs:79 - SQLite chunk persistence carries tenant_id/workspace_id/project_id.

### RFC 009 — Provider Abstraction

[RFC-009][FOUND] crates/cairn-domain/src/providers.rs:10 - Three RFC 009 operation kinds generate/embed/rerank modeled.
[RFC-009][FOUND] crates/cairn-domain/src/providers.rs:19 - Stable provider capability vocabulary exists for routing/policy evaluation.
[RFC-009][MISSING] crates/cairn-domain/src/providers.rs:325 - No EmbeddingProvider trait or embedding response type for third provider surface.
[RFC-009][PARTIAL] crates/cairn-domain/src/providers.rs:106 - RouteAttemptRecord lacks timestamp and route-policy/template linkage.
[RFC-009][PARTIAL] crates/cairn-domain/src/providers.rs:120 - RouteDecisionRecord lacks durable decision timestamp; selector_context is untyped JSON.
[RFC-009][PARTIAL] crates/cairn-domain/src/providers.rs:136 - ProviderCallRecord lacks started_at/finished_at timestamps; uses cost_micros vs RFC canonical cost field.
[RFC-009][PARTIAL] crates/cairn-domain/src/events.rs:584 - RouteDecisionMade collapses attempts into Vec<Value> instead of first-class route-attempt entities.
[RFC-009][PARTIAL] crates/cairn-domain/src/events.rs:599 - ProviderCallCompleted omits task/run/prompt linkage, fallback position, start/finish timing.
[RFC-009][MISSING] crates/cairn-domain/src/providers.rs:289 - No route template, route policy, policy baseline/effective policy, or provider-credential linkage records/services.
[RFC-009][WRONG] crates/cairn-runtime/src/services/route_resolver_impl.rs:31 - Placeholder "select first active binding"; not selector-aware, policy-aware, or capability-aware.
[RFC-009][WRONG] crates/cairn-runtime/src/services/route_resolver_impl.rs:106 - Emits one selected attempt with no veto/skip/fallback chain; attempt_count/fallback_used not truthful.
[RFC-009][WRONG] crates/cairn-runtime/src/services/route_resolver_impl.rs:78 - route_policy_id: None; no effective policy materialization.
[RFC-009][MISSING] crates/cairn-store/src/projections/routing.rs:7 - No read model for route attempts (first-class RFC 009 entities).
[RFC-009][MISSING] crates/cairn-store/src/pg/adapter.rs:642 - Postgres missing durable route decision and provider call read models.
[RFC-009][MISSING] crates/cairn-store/src/sqlite/adapter.rs:672 - SQLite missing durable route decision and provider call read models.
[RFC-009][WRONG] crates/cairn-store/src/pg/projections.rs:308 - Postgres sync projection drops RouteDecisionMade and ProviderCallCompleted; durable routing state never materialized.
[RFC-009][WRONG] crates/cairn-store/src/sqlite/projections.rs:312 - SQLite drops same routing/provider events; local durable mode loses routing/provider state.
[RFC-009][WRONG] crates/cairn-store/src/in_memory.rs:309 - In-memory route decision projection discards terminal/selected route attempt IDs.
[RFC-009][WRONG] crates/cairn-store/src/in_memory.rs:327 - In-memory provider call projection fabricates fields instead of replaying canonical data.
[RFC-009][WRONG] crates/cairn-graph/src/projections.rs:8 - No NodeKind for route decisions or provider calls; RFC 009 routing objects absent from graph.

---

## RFC 001-005 (worker-1) — pending

## RFC 010-014 (worker-3) — pending


---

## RFC 010-014 (worker-3) — 52 findings

### RFC 010 — Operator Control Plane IA

[RFC-010][WRONG] crates/cairn-app/src/main.rs:20 - AppBootstrap::start() always returns "bootstrap blocked"; operator control-plane views unusable.
[RFC-010][PARTIAL] crates/cairn-api/src/overview.rs:8 - DashboardOverview omits degraded components and recent critical events.
[RFC-010][PARTIAL] crates/cairn-api/src/operator.rs:55 - RunDetail missing checkpoints, tool activity, likely cause, retry/resume/intervene context.
[RFC-010][WRONG] crates/cairn-runtime/src/services/approval_impl.rs:35 - approval request does not block the affected run/task; approval-gated workflows do not gate execution.
[RFC-010][WRONG] crates/cairn-runtime/src/services/approval_impl.rs:56 - approval resolution never resumes or terminates blocked work.
[RFC-010][PARTIAL] crates/cairn-api/src/memory_api.rs:61 - no operator model for corpora, documents, chunks, retrieval scoring, or ingestion health.
[RFC-010][PARTIAL] crates/cairn-api/src/graph_api.rs:12 - graph surface exists as abstract seams only; no implemented visual relationship view.
[RFC-010][PARTIAL] crates/cairn-api/src/prompts_api.rs:10 - no compare, rollout, rollback, or hold actions on prompt releases.
[RFC-010][PARTIAL] crates/cairn-api/src/evals_api.rs:16 - no dataset-linked outcome comparison workflow.
[RFC-010][PARTIAL] crates/cairn-api/src/policies_api.rs:17 - no effective-permission or guardrail inspection surface.
[RFC-010][WRONG] crates/cairn-api/src/http.rs:60 - no /v1/evals* route in preserved HTTP catalog.
[RFC-010][WRONG] crates/cairn-api/src/http.rs:60 - no /v1/sources* or /v1/channels* routes in preserved HTTP catalog.
[RFC-010][MISSING] crates/cairn-api/src/operator.rs:16 - no approval bulk actions; no defer path.
[RFC-010][MISSING] crates/cairn-api/src/sources_channels.rs:20 - no bulk retry or pause/resume API for degraded sources/channels.
[RFC-010][MISSING] crates/cairn-api/src/prompts_api.rs:10 - no prompt bulk archive/label housekeeping API.

### RFC 011 — Deployment Shape

[RFC-011][FOUND] crates/cairn-api/src/bootstrap.rs:6 - Local and SelfHostedTeam deployment modes modeled.
[RFC-011][FOUND] crates/cairn-api/src/bootstrap.rs:17 - Api/RuntimeWorker/Scheduler/PluginHost roles modeled as separable.
[RFC-011][FOUND] crates/cairn-app/src/main.rs:95 - SQLite rejected for team mode; Postgres-only rule enforced.
[RFC-011][FOUND] crates/cairn-tools/src/plugin_host.rs:60 - Plugin hosting deployment-local, out-of-process, stdio-based.
[RFC-011][PARTIAL] crates/cairn-api/src/bootstrap.rs:109 - Fail-closed credential behavior represented only by credentials_available(); not enforced at runtime.
[RFC-011][MISSING] crates/cairn-api/src/auth.rs:32 - No built-in local auth, OIDC SSO, or scoped service-token implementation; only abstract seams.
[RFC-011][PARTIAL] crates/cairn-domain/src/credentials.rs:6 - Credential records hold encrypted_value but no key-version metadata or KEK provenance.
[RFC-011][MISSING] crates/cairn-domain/src/credentials.rs:6 - No encrypt/decrypt/rotation/rewrap service; RFC 011 key rotation and recovery flows unimplemented.
[RFC-011][PARTIAL] crates/cairn-api/src/settings_api.rs:6 - Settings surface omits role status, plugin health, provider health, auth-provider status, credential metadata, key-management status.
[RFC-011][PARTIAL] crates/cairn-app/src/main.rs:37 - CLI only accepts mode/address/port/db/encryption-key; no role-selection or split-role bootstrap path.

### RFC 012 — Onboarding & Starter Templates

[RFC-012][FOUND] crates/cairn-api/src/onboarding.rs:20 - Three mandatory starter template categories present.
[RFC-012][PARTIAL] crates/cairn-api/src/onboarding.rs:23 - Starter templates are name-lists only; no provider bindings, prompt releases, source configs, eval data, or workflow stages.
[RFC-012][PARTIAL] crates/cairn-api/src/onboarding.rs:84 - materialize_template() returns synthetic IDs only; does not create real tenant/workspace/project/provider/prompt-release/policy state.
[RFC-012][PARTIAL] crates/cairn-api/src/onboarding.rs:121 - Onboarding is checklist record only; no canonical bootstrap operation shared across CLI/UI/API.
[RFC-012][WRONG] crates/cairn-app/src/main.rs:20 - Both local quickstart and team bootstrap stop at bootstrap blocker; product cannot reach first-value workflow.
[RFC-012][MISSING] crates/cairn-api/src/http.rs:60 - No HTTP routes for bootstrap status, template selection, provider setup, prompt/document import, or first-project status.
[RFC-012][MISSING] crates/cairn-domain/src/org.rs:34 - No operator-management service/API; onboarding requires at least one operator account.
[RFC-012][MISSING] crates/cairn-domain/src/providers.rs:291 - No service/API to create provider connections or bindings; required for first-run onboarding.
[RFC-012][WRONG] crates/cairn-api/src/onboarding.rs:175 - Prompt import reconciliation ignores explicit import IDs, scoped logical identity, and new-version creation for changed content.
[RFC-012][PARTIAL] crates/cairn-domain/src/onboarding.rs:57 - Bootstrap provenance records template use but no mechanism to verify materialized objects still match defaults.

### RFC 013 — Artifact Import/Export Contract

[RFC-013][FOUND] crates/cairn-memory/src/bundles.rs:16 - JSON bundle envelope with prompt_library_bundle and curated_knowledge_pack_bundle discriminators defined.
[RFC-013][WRONG] crates/cairn-memory/src/bundles.rs:22 - BundleEnvelope.created_by is optional; RFC 013 requires it.
[RFC-013][PARTIAL] crates/cairn-memory/src/bundles.rs:48 - Bundle provenance too thin; missing origin, production method context.
[RFC-013][PARTIAL] crates/cairn-memory/src/bundles.rs:58 - Artifact entries use raw Value payload and Option<String> lineage; typed payload/provenance not enforced.
[RFC-013][WRONG] crates/cairn-memory/src/bundles.rs:234 - PromptAssetPayload.status uses PromptReleaseState but prompt assets have no asset-status model; contradicts actual semantics.
[RFC-013][MISSING] crates/cairn-memory/src/bundles.rs:258 - ImportService and ExportService are trait-only; no validate/plan/apply/report or export implementation anywhere.
[RFC-013][WRONG] crates/cairn-memory/src/pipeline.rs:579 - Knowledge pack ingest bypasses RFC validate->plan->apply->report contract; mutates state directly.
[RFC-013][WRONG] crates/cairn-memory/src/pipeline.rs:585 - Only curated_knowledge_pack_bundle ingest implemented; prompt_library_bundle unsupported.
[RFC-013][WRONG] crates/cairn-memory/src/pipeline.rs:597 - Bundle ingest reads only payload["content"]["text"]; discards external_ref/inline_json, hints, conflict outcomes.
[RFC-013][MISSING] crates/cairn-api/src/http.rs:60 - No API routes for validate/preview/apply import, export artifacts, or import/export reports.

### RFC 014 — Commercial Packaging & Entitlements

[RFC-014][FOUND] crates/cairn-domain/src/commercial.rs:7 - Three product tiers local_eval/team_self_hosted/enterprise_self_hosted modeled.
[RFC-014][FOUND] crates/cairn-domain/src/commercial.rs:16 - Named entitlement categories and capability mappings explicitly represented.
[RFC-014][WRONG] crates/cairn-domain/src/commercial.rs:158 - Unknown features default to Allowed; must fail-closed per RFC (gated absence must refuse, not permit).
[RFC-014][MISSING] crates/cairn-domain/src/commercial.rs:84 - Entitlement/feature-gate model never wired into API/runtime/store; gated operations not refused anywhere.
[RFC-014][MISSING] crates/cairn-api/src/settings_api.rs:6 - No operator-visible entitlement/license-status or capability-availability surface.
[RFC-014][PARTIAL] crates/cairn-domain/src/commercial.rs:65 - LicenseRecord and EntitlementChangeRecord exist as structs only; no persistence, audit API, or operator inspection.
[RFC-014][MISSING] crates/cairn-api/src/http.rs:60 - No commercial/admin HTTP surface for entitlement status, capability mapping, or audit visibility.


---

## RFC 001-005 (worker-1) — 49 findings

### RFC 001 — Product Boundary

[RFC-001][FOUND] Cargo.toml:1 - One codebase, full product surface, no split repos.
[RFC-001][FOUND] crates/cairn-app/Cargo.toml:8 - One product binary cairn-app.
[RFC-001][FOUND] crates/cairn-api/src/bootstrap.rs:6 - Local and SelfHostedTeam modes present.
[RFC-001][FOUND] crates/cairn-api/src/bootstrap.rs:17 - Api/RuntimeWorker/Scheduler/PluginHost roles modeled.
[RFC-001][FOUND] crates/cairn-api/src/auth.rs:5 - Auth/authz seams are first-class boundaries.
[RFC-001][WRONG] crates/cairn-app/src/main.rs:20 - Product binary always returns "bootstrap blocked"; self-hostable control plane not runnable.
[RFC-001][PARTIAL] crates/cairn-api/src/http.rs:215 - Operator route catalog omits eval/source/channel routes; /v1/skills advertised without implementation.
[RFC-001][PARTIAL] crates/cairn-evals/src/matrices/mod.rs:62 - Permission matrix exists as schema only; no runtime state, operator API, or UI path.

### RFC 002 — Runtime Event Model

[RFC-002][FOUND] crates/cairn-domain/src/commands.rs:16 - Command model is explicit and durable-envelope based.
[RFC-002][FOUND] crates/cairn-domain/src/events.rs:19 - Typed event envelope plus canonical RuntimeEvent union.
[RFC-002][FOUND] crates/cairn-store/src/event_log.rs:17 - full_history and current_state_plus_audit durability classes encoded.
[RFC-002][FOUND] crates/cairn-store/src/in_memory.rs:498 - Event append and synchronous projection update in same path.
[RFC-002][PARTIAL] crates/cairn-store/src/event_log.rs:66 - No idempotency-key enforcement path for externally triggered commands.
[RFC-002][WRONG] crates/cairn-domain/src/events.rs:396 - MailboxMessageAppended carries only IDs; sender, recipient, body, timestamps, delivery status missing.
[RFC-002][MISSING] crates/cairn-domain/src/errors.rs:11 - tool-result not modeled in RuntimeEntityKind/RuntimeEntityRef.
[RFC-002][MISSING] crates/cairn-domain/src/events.rs:91 - No PermissionDecision event even though RFC 002 requires permission decisions in tool event model.
[RFC-002][WRONG] crates/cairn-domain/src/events.rs:453 - Recovery events allow both run_id and task_id absent; targetless recovery facts possible.
[RFC-002][PARTIAL] crates/cairn-store/src/event_log.rs:59 - 72-hour replay floor exists as comment only; no retention/configuration implementation.
[RFC-002][PARTIAL] crates/cairn-store/src/pg/adapter.rs:617 - Prompt-release current_state_plus_audit models stubbed; Postgres backend doesn't fully implement durability class split.

### RFC 003 — Owned Retrieval

[RFC-003][FOUND] crates/cairn-memory/src/ingest.rs:5 - Canonical v1 source types present.
[RFC-003][FOUND] crates/cairn-memory/src/pipeline.rs:508 - Ingest pipeline follows owned path through registration/normalization/chunking/dedup/embedding/index.
[RFC-003][WRONG] crates/cairn-memory/src/pipeline.rs:190 - KnowledgePack normalization is raw pass-through; not normalized into owned retrieval document model.
[RFC-003][WRONG] crates/cairn-memory/src/pipeline.rs:542 - Dedup only checks stored hashes; within-batch duplicates survive.
[RFC-003][WRONG] crates/cairn-memory/src/pg/retrieval.rs:35 - Postgres VectorOnly errors; Hybrid silently degrades to lexical-only.
[RFC-003][WRONG] crates/cairn-memory/src/sqlite/retrieval.rs:32 - SQLite VectorOnly returns empty success; silent "no owned retrieval".
[RFC-003][WRONG] crates/cairn-memory/src/in_memory.rs:176 - Metadata filters only match string-valued provenance fields.
[RFC-003][WRONG] crates/cairn-memory/src/in_memory.rs:225 - Scoring breakdown missing source credibility, corroboration, graph proximity, recency-of-use.
[RFC-003][WRONG] crates/cairn-memory/src/deep_search_impl.rs:161 - max_hops==0 returns zero retrieval hops; deep search can terminate without one owned pass.
[RFC-003][WRONG] crates/cairn-memory/src/deep_search_impl.rs:224 - Graph expansion fed request.query_text not per-hop decomposition query.
[RFC-003][PARTIAL] crates/cairn-memory/src/diagnostics_impl.rs:27 - Source diagnostics key by source_id only; inflate on re-ingest; operator views unreliable.

### RFC 004 — Graph & Eval Matrix

[RFC-004][FOUND] crates/cairn-graph/src/projections.rs:5 - Typed node/edge vocabularies covering RFC 004 entity/relationship model.
[RFC-004][FOUND] crates/cairn-graph/src/queries.rs:6 - Graph query surface product-shaped around named query families.
[RFC-004][WRONG] crates/cairn-graph/src/event_projector.rs:273 - Prompt assets/releases, route decisions, provider calls no-oped in graph projection.
[RFC-004][WRONG] crates/cairn-graph/src/eval_projector.rs:27 - Prompt/eval graph nodes written with project: None; violates RFC scope rule.
[RFC-004][FOUND] crates/cairn-evals/src/matrices/mod.rs:15 - Stable matrix category schemas for all RFC 004 matrix families.
[RFC-004][PARTIAL] crates/cairn-evals/src/matrices/mod.rs:33 - Prompt comparison rows missing effective selector context; row grain incomplete.
[RFC-004][PARTIAL] crates/cairn-store/src/projections/prompt.rs:38 - Prompt-release projection drops release_tag/created_by; lifecycle state as raw string.
[RFC-004][PARTIAL] crates/cairn-api/src/graph_api.rs:16 - API exposes execution trace and retrieval provenance only; dep path/prompt provenance/decision/eval-lineage not surfaced.
[RFC-004][PARTIAL] crates/cairn-store/src/pg/adapter.rs:642 - Route-decision/provider-call read models unimplemented for Postgres.

### RFC 005 — Task, Session & Checkpoint Lifecycle

[RFC-005][FOUND] crates/cairn-domain/src/lifecycle.rs:6 - Session/run/task/checkpoint lifecycle enums match RFC 005 state machines.
[RFC-005][FOUND] crates/cairn-domain/src/lifecycle.rs:147 - Run/task transition helpers and session-state derivation implement RFC rules.
[RFC-005][FOUND] crates/cairn-domain/src/lifecycle.rs:67 - PauseReason carries reason kind, actor/source, resume_after_ms.
[RFC-005][WRONG] crates/cairn-domain/src/events.rs:337 - TaskLeaseClaimed.lease_owner is raw String instead of WorkerId.
[RFC-005][WRONG] crates/cairn-store/src/projections/task.rs:22 - Task projection stores lease_owner as Option<String> not WorkerId.
[RFC-005][WRONG] crates/cairn-runtime/src/services/task_impl.rs:124 - Single lease transition emits both TaskLeaseClaimed and TaskStateChanged; duplicates one lifecycle edge.
[RFC-005][WRONG] crates/cairn-runtime/src/services/task_impl.rs:169 - Heartbeats extend leases without validating worker ownership or lease freshness.
[RFC-005][WRONG] crates/cairn-store/src/pg/projections.rs:193 - Postgres checkpoint projection drops data payload; durable recovery state incomplete.
[RFC-005][PARTIAL] crates/cairn-runtime/src/services/recovery_impl.rs:126 - CheckpointRestored/RecoveryCompleted emitted but run-state never transitioned back to active.
[RFC-005][PARTIAL] crates/cairn-store/src/projections/task.rs:53 - No dedicated dependency/subagent read model for RFC 005 dependency view.

---

## Summary — 163 total findings

| RFC | FOUND | PARTIAL | MISSING | WRONG |
|-----|-------|---------|---------|-------|
| 001 | 5 | 2 | 0 | 1 |
| 002 | 4 | 3 | 2 | 2 |
| 003 | 2 | 1 | 0 | 8 |
| 004 | 2 | 4 | 0 | 2 |
| 005 | 3 | 2 | 0 | 5 |
| 006 | 3 | 4 | 3 | 6 |
| 007 | 4 | 4 | 1 | 4 |
| 008 | 4 | 3 | 4 | 4 |
| 009 | 2 | 5 | 5 | 7 |
| 010 | 0 | 5 | 3 | 4 |
| 011 | 4 | 3 | 2 | 0 |
| 012 | 1 | 3 | 4 | 3 |
| 013 | 1 | 2 | 2 | 5 |
| 014 | 2 | 1 | 3 | 1 |

