# Changelog

All notable changes to cairn-rs are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added

- **`POST /v1/runs/:id/claim`** — activates a run's FlowFabric execution lease
  so downstream FCALLs (`enter_waiting_approval`, `pause`, signal delivery)
  accept it. NOT idempotent on the Fabric path: re-claiming an already-active
  run fails at FF's grant gate with `execution_not_eligible`. A second claim
  after a suspend/resume cycle dispatches through `ff_claim_resumed_execution`
  and is legitimate.

### Changed

- **RFC-011 phase-1 mechanical sweep** — FF rev bump `a098710` → `1b19dd10`
  (RFC-011 exec/flow hash-slot co-location, phases 1-3). Consumer-side
  adoptions in cairn-fabric:
  - `num_execution_partitions` renamed to `num_flow_partitions`; default
    raised 64 → 256. **Operator action required** if `FF_EXEC_PARTITIONS`
    is set: rename env var to `FF_FLOW_PARTITIONS` before deploying, or
    accept the new default of 256.
  - `ExecutionId` construction migrated to deterministic mint helpers
    (`deterministic_solo` / `for_flow`). The `::new()`, `::from_uuid()`,
    and `Default` constructors are removed upstream.
  - Parallel `parse_spend_result` deleted from `budget_service.rs`;
    replaced with `ff_sdk::task::parse_report_usage_result` (FF #16 closed).
  - Hardcoded `format!("ff:usagededup:…")` sites replaced with
    `ff_core::keys::usage_dedup_key` helper.
  - API-boundary validation added: run/session/project IDs now reject
    control characters at the HTTP handler layer.
  - `FabricError` detail stripping: 500 responses no longer leak Valkey
    key names or Lua error internals.

- **`TaskFrameSink` orchestrator integration** (#30) — orchestrator logs
  tool calls, tool results, LLM responses, and checkpoints through a
  non-consuming sink on the active `CairnTask`, removing the need to thread
  a separate `FrameSink` handle alongside the task. Lease-health gate aborts
  the loop before irreversible side effects when FF reports 3 consecutive
  renewal misses. Checkpoint-snapshot serialize failures degrade to a WARN
  log instead of aborting the step.

### Removed

- **`ActiveTaskRegistry`** (#29) — retired in favour of FlowFabric-owned lease
  state. `CairnTask` now carries the underlying `ClaimedTask` directly; the
  cairn-side registry was a cache of state FF already holds atomically, and
  kept drifting out of sync under lease expiry. Event-emission gate in the
  orchestrator now reads lease health through `TaskFrameSink::is_lease_healthy`
  (the worker-sdk accessor) rather than a cairn-local flag.

---

## [0.1.0] — 2026-04-05

First complete, test-verified milestone. The core control-plane infrastructure
is implemented and RFC-compliant across all ten specified contracts.

### Added

#### Runtime and domain

- **Event-sourced runtime** — 111 `RuntimeEvent` variants covering sessions, runs,
  tasks, approvals, checkpoints, provider calls, credentials, channels, evals,
  signals, knowledge, and commercial events. Every state change is an append;
  no in-place mutation.
- **RFC 002 event-log contract** — append-only log with monotonically ordered
  `EventPosition`, causation-ID idempotency, cursor-based replay, and a
  72-hour SSE replay window. `find_by_causation_id` prevents duplicate command
  application across retries.
- **RFC 005 approval blocking** — `ApprovalRequested` gates run/task progression.
  Pending approvals surface in the operator inbox; `ApprovalResolved` unblocks
  the run atomically and increments the approval record version.
- **RFC 006 prompt release lifecycle** — `draft → active` state machine with
  `PromptReleaseCreated` / `PromptReleaseTransitioned` events; per-asset
  scorecard aggregation across releases.
- **RFC 007 provider health** — `ProviderConnectionRegistered`,
  `ProviderHealthChecked`, `ProviderMarkedDegraded`, `ProviderRecovered` events
  drive the health read model; consecutive failure tracking and per-tenant
  isolation.
- **RFC 008 multi-tenant isolation** — all read-model queries are scoped to
  `ProjectKey` (tenant + workspace + project); cross-tenant data does not
  appear in any listing.
- **RFC 009 provider routing and cost** — `FallbackChainResolver` with
  capability checking; `RouteDecisionRecord` persisted with `fallback_used`
  flag; per-run and per-session cost accumulation in USD micros; derived
  `RunCostUpdated` / `SessionCostUpdated` events emitted into the log.
- **RFC 013 eval rubrics and bundles** — rubric scoring (ExactMatch, Contains,
  Similarity, Plugin); baseline comparison with 5 % regression tolerance;
  `BundleEnvelope` import/export with `PromptLibraryBundle` and
  `CuratedKnowledgePackBundle` discriminators.
- **RFC 014 commercial feature gating** — `ProductTier` (LocalEval,
  TeamSelfHosted, EnterpriseSelfHosted), `Entitlement` categories,
  `DefaultFeatureGate` with fail-closed unknown-feature semantics,
  `EntitlementOverrideSet` events for operator-applied overrides.
- **Durability class contract** — `EntityDurabilityClass::FullHistory` for
  Session/Run/Task (full replay required); `CurrentStatePlusAudit` for all
  other entities. Defined in `cairn-domain` so domain tests can reason about
  durability without depending on the store crate.

#### Storage backends

- **`InMemoryStore`** — full `EventLog` + 51 read-model trait implementations;
  synchronous `apply_projection` within the same lock as `append`; broadcast
  channel for SSE live delivery; `subscribe()` for real-time event fan-out.
- **`PgEventLog`** — durable Postgres append-only event log; events stored in
  `event_log` table with JSON payload; `find_by_causation_id` scans for
  idempotency.
- **`PgAdapter`** — Postgres read models for Session, Run, Task, Approval,
  Checkpoint, Mailbox, ToolInvocation (7 of 51; remainder tracked as gap list
  for follow-on work).
- **`PgSyncProjection`** — synchronous projection applier runs within the same
  Postgres transaction as the append; all new `RuntimeEvent` variants have
  no-op arms.
- **`PgMigrationRunner`** — 17 embedded SQL migrations (V001–V017); applied
  atomically within a transaction on first boot; migration history recorded in
  `_cairn_migrations`.

#### HTTP server (`cairn-app`)

- **16 routes** wired with axum 0.7:
  - `GET /health` — liveness probe (auth-exempt)
  - `GET /v1/stream` — SSE event stream with `Last-Event-ID` replay (auth-exempt)
  - `GET /v1/status` — runtime + store health; Postgres health check when configured
  - `GET /v1/dashboard` — active runs, tasks, pending approvals, system health
  - `GET /v1/runs` + `GET /v1/runs/:id` — run listing and lookup
  - `GET /v1/sessions` — active session listing
  - `GET /v1/approvals/pending` + `POST /v1/approvals/:id/resolve` — approval inbox and resolution
  - `GET /v1/prompts/assets` + `GET /v1/prompts/releases` — prompt asset and release listing
  - `GET /v1/costs` — aggregate cost summary (calls, tokens, USD micros)
  - `GET /v1/providers` — provider binding listing
  - `GET /v1/events` — cursor-based event log replay
  - `POST /v1/events/append` — idempotent event append with causation-ID guard
  - `GET /v1/db/status` — Postgres connectivity and migration state
- **Bearer token auth middleware** (RFC 008) — all `/v1/*` routes except `/v1/stream`
  require `Authorization: Bearer <token>`; `ServiceTokenRegistry` supports
  multiple concurrent tokens.
- **SSE protocol** — `connected` event on open; replay up to 1 000 events after
  `Last-Event-ID`; 15-second keepalive comments; SSE `id:` field carries log
  position for resume.
- **Postgres wiring** — `--db postgres://...` flag creates a `PgPool`, runs
  pending migrations, and enables dual-write: events appended to Postgres
  (durability) and InMemory (read models + SSE broadcast). `GET /v1/events`
  served from Postgres log when configured.
- **CLI flags** — `--mode`, `--port`, `--addr`, `--db`, `--encryption-key-env`.
  Team mode binds `0.0.0.0` and requires `CAIRN_ADMIN_TOKEN`.

#### Knowledge pipeline (`cairn-memory`)

- **Ingest pipeline** — `IngestPipeline<S, C>` with `ParagraphChunker`;
  normalization for PlainText, Markdown, Html; chunk deduplication by
  content hash; no-op `NoOpEmbeddingProvider` for tests.
- **Retrieval scoring** — lexical relevance, freshness decay (`e^(-age/decay_days)`),
  staleness penalty (linear beyond threshold), source credibility, corroboration,
  graph proximity from `InMemoryGraphStore` neighbor overlap.
- **`InMemoryRetrieval`** — `with_graph()` now actually wires the graph store
  and computes proximity; `explain_result()` returns a `ResultExplanation` with
  all scoring dimensions and a human-readable summary.
- **Source quality diagnostics** — `InMemoryDiagnostics` tracks chunk counts,
  retrieval hits, average relevance per source; `index_status()` aggregates
  across all sources for a project.
- **Bundle import/export** — `InMemoryImportService` validates `KnowledgeDocument`
  artifacts, deduplicates by content hash, infers `ImportOutcome` (Create/Skip).
  `InMemoryExportService` bundles documents with origin scope and provenance metadata.

#### Eval system (`cairn-evals`)

- **`EvalRunService`** — in-memory eval run lifecycle: Pending → Running →
  Completed/Failed; `complete_run()` stores `EvalMetrics`;
  `build_scorecard()` aggregates across releases per asset;
  `set_dataset_id()` links a dataset to a run post-creation.
- **`EvalBaselineServiceImpl`** — `set_baseline()`, `compare_to_baseline()`;
  regression detection with ±5 % tolerance band; `fallback_used` flag on locked
  baselines; `select_baseline()` prefers locked over most-recent.
- **`EvalRubricServiceImpl`** — rubric scoring across ExactMatch, Contains,
  Similarity, Plugin dimensions; `score_against_rubric()` requires a dataset
  link; `PluginRubricScorer` trait for custom scoring backends.
- **`BanditServiceImpl`** (GAP-013) — `EpsilonGreedy` and `UCB1` selection
  strategies; `record_reward()` updates `pulls` and `reward_sum`; `with_fixed_rng()`
  for deterministic testing; `list_by_tenant()` for per-tenant experiment views.
- **Provider binding cost stats** — `ProviderBindingCostStatsReadModel`
  implemented with real event-log scan (replaces the stub that returned `None`);
  `list_by_tenant()` groups by `provider_binding_id` via raw event scan.

#### Docs

- **`docs/api-reference.md`** — 769-line operator API reference: all 16 routes,
  request/response shapes, curl examples, auth guide, error codes, server
  configuration, route summary table.
- **`docs/deployment.md`** — Docker Compose, Postgres setup, environment
  variables, team/local mode, TLS, production hardening.

### Architecture

- **12 Rust crates** — `cairn-domain`, `cairn-store`, `cairn-runtime`,
  `cairn-api`, `cairn-app`, `cairn-memory`, `cairn-graph`, `cairn-evals`,
  `cairn-tools`, `cairn-signal`, `cairn-channels`, `cairn-plugin-proto`.
  No circular dependencies.
- **Event log + synchronous projections** — the same `apply_projection` logic
  drives both InMemory and Postgres backends; there is no dual-implementation
  drift. Appends within a transaction guarantee projection consistency.
- **RFC 002–014 compliance** — ten RFC contracts verified by executable
  integration tests. `rfc_compliance_summary.rs` in `cairn-store/tests/`
  contains one focused test per RFC verifying the single most critical MUST
  requirement against the real store backend.

### Test suite

| Category | Count | Failures |
|----------|-------|----------|
| Lib tests (all crates except cairn-app) | 796 | 0 |
| Integration tests (new this session) | ~230 | 0 |
| Previously-broken tests (fixed) | 33 | 0 |
| **Total** | **~1 059** | **0** |

**40+ integration test files** across cairn-store (15 files), cairn-runtime (3),
cairn-memory (8), cairn-evals (3), cairn-api (1), cairn-domain (3).

Notable integration suites:
- `rfc_compliance_summary.rs` — one test per RFC (6 tests)
- `entity_scoped_reads.rs` — RFC 002 entity-scoped event pagination
- `idempotency.rs` — causation-ID idempotency contract (7 tests)
- `event_log_compaction.rs` — 50-event scale proof with cursor pagination
- `approval_blocking.rs` — RFC 005 approval gate lifecycle
- `provider_routing_e2e.rs` — RFC 009 fallback chain with FallbackChainResolver
- `cost_aggregation_accuracy.rs` — per-call micros precision, zero-cost isolation
- `durability_classes.rs` — RFC 002 entity durability contract
- `product_tier_gating.rs` — RFC 014 commercial gating across all three tiers

### Fixed

- **9 pre-existing integration test failures** across cairn-evals
  (`baseline_flow`, `dataset_flow`, `rubric_flow`), cairn-runtime
  (`binding_cost_stats`), and cairn-memory (`ingest_retrieval_pipeline`,
  `entity_extraction`, `explain_result`, `graph_proximity`,
  `provenance_tracking`). Root causes: wrong-crate `EvalSubjectKind` imports,
  extra argument to `create_run`, missing `IngestRequest` fields added in
  later RFCs, stub `ProviderBindingCostStatsReadModel` returning `None`,
  missing `explain_result()` method on `InMemoryRetrieval`, missing graph
  proximity implementation.
- **`DashboardOverview` initializers** in `cairn-api/src/overview.rs` — four
  internal test constructors updated to include the six new RFC 010
  observability fields added during the GAP implementation phase.
- **`PgSyncProjection` non-exhaustive patterns** — `ApprovalPolicyCreated` and
  `PromptRolloutStarted` were missing no-op arms; added to resolve the
  `--features postgres` compile error.

---

*This changelog was generated at the close of the implementation session.*
*Session date: 2026-04-05. Workspace: cairn-rs.*
