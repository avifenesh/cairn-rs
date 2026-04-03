# Worker 6 Mailbox

Owner: Memory, Retrieval, Graph

## Current Status

- 2026-04-03 | Week 1 assigned | Scaffold `cairn-memory` and `cairn-graph`, define ingest/query/graph interfaces, and align storage needs to Worker 3.
- 2026-04-03 | Worker 6 / Manager | `cairn-memory` and `cairn-graph` scaffold complete | Ingest, retrieval, diagnostics, deep-search service boundaries in cairn-memory. Graph projections (typed nodes/edges), product-shaped graph queries (6 v1 query families), and provenance service boundaries in cairn-graph. Both crates depend on cairn-domain. All 11 tests pass.
- 2026-04-03 | Week 2 assigned | Implement document and graph entity persistence skeletons, align retrieval and graph storage requirements to schema reality.
- 2026-04-03 | Worker 6 / Manager | Week 2 complete | `PgDocumentStore` and `PgGraphStore` with Postgres persistence. Both feature-gated behind `postgres`. All 23 tests passing.
- 2026-04-03 | Week 3 assigned | Implement ingest pipeline, retrieval query path, graph projection flow for runtime events.
- 2026-04-03 | Worker 6 / Manager | Week 3 complete | `IngestPipeline`, `PgRetrievalService` (FTS), `EventProjector`. 28 tests passing.
- 2026-04-03 | Week 4 assigned | Complete first owned retrieval flow for supported document floor, expose graph-backed provenance for runtime/operator use.
- 2026-04-03 | Worker 6 / Manager | Week 4 complete | `InMemoryDocumentStore` + `InMemoryRetrieval` for database-free e2e retrieval. `GraphProvenanceService` backed by `GraphQueryService` (execution provenance, retrieval provenance, provenance chains). E2e integration tests: ingest 2 documents, query with scoring, cross-document search, project isolation. All 4 v1 source types (plain text, markdown, html, structured json) ingest successfully. 37 tests passing across all 3 crates.

## Blocked By

- none

## Inbox

- 2026-04-03 | Architecture Owner -> Worker 6 | Week 1 focus: memory and graph service skeletons with storage-facing interfaces, not deep implementation.
- 2026-04-03 | Worker 1 -> Worker 6 | Align ingest/query/graph storage assumptions with Worker 3 before hardening persistence paths. Treat RFC 013 import shape as fixed input.
- 2026-04-03 | Worker 3 -> Worker 6 | `cairn-store` event-log and projection boundaries are available. Graph persistence should use store interfaces as the write-side contract for durable state.
- 2026-04-03 | Worker 7 -> Worker 6 | `cairn-evals` prompt registry types and graph-linkable IDs are available. Graph nodes for prompt_asset, prompt_version, prompt_release, eval_run can be built against these.
- 2026-04-03 | Worker 1 / Manager -> Worker 6 | Current next focus: take the Week 4 owned-core slice. Close one first owned retrieval flow end-to-end and make graph-backed provenance queryable enough for runtime/operator consumption, using the API and runtime seams already in place.

## Outbox

- 2026-04-03 | Worker 6 -> Worker 4 | `cairn-graph` now exposes `GraphProjection` trait (add_node/add_edge), graph query service with 6 product-shaped query families (execution trace, dependency path, prompt provenance, retrieval provenance, decision involvement, eval lineage), and `ProvenanceService` for execution and retrieval provenance chains. Runtime can build graph projections from events.
- 2026-04-03 | Worker 6 -> Worker 7 | `cairn-graph` graph node/edge kinds include `PromptAsset`, `PromptVersion`, `PromptRelease`, `EvalRun`, `Skill` nodes and `EvaluatedBy`, `ReleasedAs`, `RolledBackTo`, `UsedPrompt` edges. Eval lineage graph query is ready for prompt/eval integration.
- 2026-04-03 | Worker 6 -> Worker 8 | `cairn-memory` exposes `RetrievalService` (query with inspectable scoring breakdown), `DiagnosticsService` (source quality, index status), and `DeepSearchService` (multi-hop). `cairn-graph` exposes `GraphQueryService` and `ProvenanceService` for API read surfaces.
- 2026-04-03 | Worker 6 -> Worker 5 | `cairn-graph` includes `UsedTool` edge kind and `ToolInvocation` node kind. Tool invocation graph linking is ready for Worker 5 integration.
- 2026-04-03 | Worker 8 -> Worker 6 | `cairn-signal` exposes `SignalSource`, `SourcePoller`, `PollSchedule`, `PollResult`, and digest types. Signal integration for memory/retrieval can build against these.
- 2026-04-03 | Worker 3 -> Worker 6 | Week 2: Document (V010) and chunk (V011) tables are in the migration set. Graph node (V012) and edge (V013) tables are ready.
- 2026-04-03 | Worker 5 -> Worker 6 | Week 2: `PermissionDecisionEvent` records are now emittable. Graph can link permission decisions to tool invocations when event projection is wired.
- 2026-04-03 | Worker 6 -> Worker 4 | Week 2: `PgGraphStore` implements `GraphProjection` + `GraphQueryService` with Postgres persistence. Runtime can persist graph nodes/edges from events and query all 6 v1 families.
- 2026-04-03 | Worker 6 -> Worker 8 | Week 2: `PgDocumentStore` persists documents and chunks. `PgGraphStore` persists graph. API can query through the service traits backed by Postgres.
- 2026-04-03 | Worker 6 -> Worker 4 | Week 3: `EventProjector` consumes `StoredEvent` batches and creates graph nodes/edges for sessions, runs, tasks, approvals, checkpoints, mailbox messages, tool invocations, and subagent spawns. Wire this as an async post-commit hook on event log appends.
- 2026-04-03 | Worker 6 -> Worker 8 | Week 3: `PgRetrievalService` implements lexical search via Postgres FTS. `IngestPipeline` processes documents end-to-end. API can expose ingest submission + retrieval query endpoints.

## Ready For Review

- 2026-04-03 | Worker 6 | Review `crates/cairn-memory/*` for Week 1 memory scaffold: ingest service (source types, chunking, embedding pipeline), retrieval service (lexical/vector/hybrid with inspectable scoring), diagnostics service, and deep-search service.
- 2026-04-03 | Worker 6 | Review `crates/cairn-graph/*` for Week 1 graph scaffold: graph projection (typed nodes/edges per RFC 004), product-shaped graph queries (6 v1 query families), and provenance service (execution and retrieval provenance chains).
- 2026-04-03 | Worker 6 | Review `crates/cairn-memory/src/pg/*` and `crates/cairn-graph/src/pg/*` for Week 2 Postgres persistence: document/chunk store and graph store with BFS traversal.
- 2026-04-03 | Worker 6 | Review Week 3: `crates/cairn-memory/src/pipeline.rs` (ingest pipeline + chunker), `crates/cairn-memory/src/pg/retrieval.rs` (Postgres FTS retrieval), `crates/cairn-graph/src/event_projector.rs` (runtime event -> graph projection). 28 tests passing.
