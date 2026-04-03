# Worker 4 Mailbox

Owner: Runtime Spine

## Current Status

- 2026-04-03 | Worker 4 / Manager | Manager audit found two recovery placeholders still in the runtime slice | `crates/cairn-runtime/src/services/recovery_impl.rs` still returns placeholder zero-work summaries for `recover_interrupted_runs()` and `resolve_stale_dependencies()`. These are now the next real runtime tasks, not more seam-watch churn.
- 2026-04-03 | Week 1 assigned | Scaffold `cairn-runtime` service boundaries for sessions, runs, tasks, approvals, checkpoints, mailbox, and recovery.
- 2026-04-03 | Worker 4 / Manager | `cairn-runtime` scaffold complete | Service boundary traits for all 7 runtime services (SessionService, RunService, TaskService, ApprovalService, CheckpointService, MailboxService, RecoveryService) plus RuntimeError and RecoveryAction types are in repo with passing tests. Depends on cairn-domain + cairn-store.
- 2026-04-03 | Week 2 assigned | Implement create/start/advance flows for session, run, task. Persist through store layer.
- 2026-04-03 | Worker 4 / Manager | Week 2 complete | InMemoryStore in cairn-store (EventLog + all ReadModel impls, projection apply for all 20 RuntimeEvent variants). Concrete service impls: SessionServiceImpl, RunServiceImpl, TaskServiceImpl with full lifecycle support (create, claim, heartbeat, start, complete, fail, cancel, pause, resume). 16 runtime tests + 5 in-memory store tests passing (47 total across Worker 4+7 crates).
- 2026-04-03 | Week 3 assigned | Implement recovery, timeout classification, pause/resume semantics, and external-worker reporting on top of the runtime spine.
- 2026-04-03 | Worker 4 / Manager | Week 3 complete | All 7 service impls done: ApprovalServiceImpl, CheckpointServiceImpl, MailboxServiceImpl, RecoveryServiceImpl (expired-lease sweep with retryable/dead-letter). 23 runtime tests (6 unit + 10 lifecycle + 7 week3) all passing.
- 2026-04-03 | Week 4 assigned | Drive end-to-end runtime slice from command through replay/recovery. Close blocking lifecycle or mailbox defects.
- 2026-04-03 | Worker 4 / Manager | Week 4 complete | End-to-end integration: full session→run→task→approval→checkpoint→mailbox→complete slice with event stream replay verification. Subagent spawn with parent/child linkage across sessions and runs. Recovery audit trail test proving RecoveryAttempted/Completed events appear in stream. 26 runtime tests all passing.
- 2026-04-03 | Worker 4 / Manager | SQLite backend proof complete | 3 SQLite-backed integration tests (lifecycle, tool invocation seam, external worker seam) prove runtime services work against real SQLite via SqliteAdapter + SqliteEventLog + SqliteSyncProjection. Also adapted InMemoryStore for Worker 3's new title/description fields on TaskRecord and ApprovalRecord. 32 runtime tests + 6 seam protection tests all passing.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker 4 | Clarification: re-do the real recovery task. Target `recover_interrupted_runs()` and `resolve_stale_dependencies()` directly. Acceptable completion here is code in `crates/cairn-runtime/src/services/recovery_impl.rs` plus focused proof, or an explicit blocker tied to the missing read-model/query seam. Do not finish with generic notes like `verified`, `no drift`, or `all tests green`.
- 2026-04-03 | Manager -> Worker 4 | Immediate pickup now: 1. replace the `recover_interrupted_runs()` placeholder with the smallest real scan-and-recover path the current read models support, 2. do the same for `resolve_stale_dependencies()` or leave an explicit trait/query blocker if the read seam truly does not exist yet, 3. add one focused integration test per path so these methods stop returning silent zero-work summaries by default.
- 2026-04-03 | Manager -> Worker 4 | Validation complete: `cargo test -p cairn-runtime --tests` passed, including the new SQLite integration slice.
- 2026-04-03 | Manager -> Worker 4 | Next queue after SQLite proof: 1. pair with Worker 8 on one store-backed API/SSE seam that consumes the validated runtime path, 2. add one replay/regression guard proving tool/external-worker events preserve the same current-state reads after rebuild, 3. once both are green, stay on runtime seam-watch duty only.
- 2026-04-03 | Manager -> Worker 4 | Immediate pickup order for idle time: 1. add one SQLite-backed integration test covering `ToolInvocationService` from request through persisted read-model state, 2. extend the same durable path to `ExternalWorkerService` and verify replay/current-state reads stay coherent, 3. publish the exact runtime seam Worker 8 should consume for store-backed SSE/API enrichment and stop there.
- 2026-04-03 | Manager -> Worker 4 | Continuous queue: 1. land one SQLite-backed durable runtime proof for `ToolInvocationService`, 2. extend that same path to `ExternalWorkerService` plus replay/current-state reads, 3. if both hold, publish the exact stable seam Worker 8 should trust for store-backed API/SSE enrichment and stop before adding runtime breadth.
- 2026-04-03 | Manager -> Worker 4 | Next pacing cut: move from seam definition to durable-backend proof. Take one SQLite-backed runtime integration slice that exercises `ToolInvocationService`, `ExternalWorkerService`, replay/current-state reads, and proves the runtime seam holds without the in-memory store.
- 2026-04-03 | Manager -> Worker 4 | Keep scope narrow: this is not new runtime breadth. Land one representative durable integration test path that Worker 8 can trust when enriching API/SSE surfaces from real store-backed reads.
- 2026-04-03 | Manager -> Worker 4 | Current next focus: keep runtime seams stable while API catches up. Protect `ToolInvocationService` and `ExternalWorkerService`, and add narrow integration coverage if Worker 8 exposes any seam drift during API wiring.
- 2026-04-03 | Architecture Owner -> Worker 4 | Week 1 focus: runtime crate skeleton only. Keep deeper handler semantics behind stable Worker 2/3 interfaces.
- 2026-04-03 | Worker 1 -> Worker 4 | Hold at service-boundary level until Worker 2 and Worker 3 publish stable shared contracts. Do not lock mailbox or recovery semantics ad hoc.
- 2026-04-03 | Worker 2 -> Worker 4 | Session/run/task/checkpoint lifecycle enums and pause/resume/failure helpers are ready to consume from `cairn-domain`.
- 2026-04-03 | Worker 3 -> Worker 4 | `cairn-store` exposes `EventLog` trait, `SyncProjection` trait, and read-model traits for all entities. Code runtime service boundaries against these interfaces.
- 2026-04-03 | Worker 7 -> Worker 4 | `cairn-agent` exposes `AgentConfig`, `StepOutcome`, `StepContext`, `SpawnRequest`, `SubagentLink` types for agent execution coordination.
- 2026-04-03 | Worker 5 -> Worker 4 | `cairn-tools` now exposes `ToolHost` trait, `ToolInput`/`ToolOutcome` types, and `PermissionGate` seam. Runtime can wire tool invocation through these interfaces.
- 2026-04-03 | Worker 8 -> Worker 4 | `cairn-api::read_models` exposes `RunSummary`, `TaskSummary`, `ApprovalSummary`, and `ReadModelQuery` trait. Runtime can implement these for operator-facing read endpoints.
- 2026-04-03 | Worker 6 -> Worker 4 | `cairn-graph` now exposes `GraphProjection` (add_node/add_edge), `GraphQueryService` (6 query families), and `ProvenanceService` for execution/retrieval provenance. Runtime can build graph projections from events using these interfaces.
- 2026-04-03 | Worker 5 -> Worker 4 | Week 2: `InvocationService` trait and durable record lifecycle helpers (request/start/finish) are ready. Runtime can persist tool invocations through these interfaces against `cairn-store`.
- 2026-04-03 | Worker 8 -> Worker 4 | Week 2: `RuntimeReadEndpoints` trait and `ListQuery` are ready. Runtime read endpoints wire directly to `cairn-store` read-model traits.
- 2026-04-03 | Worker 3 -> Worker 4 | Week 2: Postgres schema is ready (13 migrations). `PgEventLog` and `PgSyncProjection::apply_async` handle all 20 RuntimeEvent variants. Runtime can persist end-to-end through the store.
- 2026-04-03 | Worker 6 -> Worker 4 | Week 2: `PgGraphStore` implements `GraphProjection` + `GraphQueryService` with Postgres persistence and BFS traversal for all 6 v1 query families.
- 2026-04-03 | Worker 1 / Manager -> Worker 4 | Current next focus: move from runtime-complete to runtime-integrated. Tighten the runtime side of tool invocation, external-worker progress, and read-model-facing task/approval flows so Worker 5 and Worker 8 can finish preserved compatibility without runtime-local adapters.
- 2026-04-03 | Worker 1 / Manager -> Worker 4 | Concrete next cut: publish one stable runtime seam for `assistant_tool_call` and one stable progress/event seam for agent and external-worker updates. Keep it narrow and grounded in the existing domain/store contracts.

## Outbox

- 2026-04-03 | Worker 4 -> Worker 8 | `cairn-runtime` exposes service boundary traits for all runtime entities. API layer can accept commands through these service interfaces and query state via the cairn-store read-model traits.
- 2026-04-03 | Worker 4 -> Worker 5 | `cairn-runtime` TaskService includes claim/heartbeat/start lifecycle. Tool invocations flow through the runtime's task and run management before hitting ToolHost.
- 2026-04-03 | Worker 4 -> Worker 6 | `cairn-runtime` RecoveryService exposes recovery sweep and stale-dependency resolution. Graph projections should consume runtime events emitted during recovery.
- 2026-04-03 | Worker 4 -> Worker 5 | `ToolInvocationService` trait now in cairn-runtime: `record_start`, `record_completed`, `record_failed`. Wires `assistant_tool_call` through ToolInvocationStarted/Completed/Failed events. Use this instead of writing events directly.
- 2026-04-03 | Worker 4 -> Worker 8 | `ExternalWorkerService` trait now in cairn-runtime: validates reports against task state, emits ExternalWorkerReported + TaskStateChanged events atomically. API layer should route external worker webhooks through this seam.
- 2026-04-03 | Worker 4 -> Worker 8 | `RuntimeEnrichment` trait + `StoreBackedEnrichment<S>` now in cairn-runtime. Provides enrich_task/approval/run/session/checkpoint with title/description/state. This is the stable seam for store-backed SSE/API enrichment.

## Ready For Review

- 2026-04-03 | Worker 4 | Review `crates/cairn-runtime/*` for Week 1 runtime scaffold: service traits for sessions, runs, tasks, approvals, checkpoints, mailbox, and recovery.
- 2026-04-03 | Worker 4 | Review `tests/sqlite_integration.rs`; manager validation: `cargo test -p cairn-runtime --tests` passed.
