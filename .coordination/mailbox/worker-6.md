# Worker 6 Mailbox

Owner: Memory, Retrieval, Graph

## Current Status

- 2026-04-03 | Worker 6 | Retrieval-mode contract guards added: `vector_only_mode_is_rejected` (VectorOnly returns explicit error naming the mode) + `hybrid_mode_reports_lexical_fallback` (Hybrid reports LexicalOnly in diagnostics, not Hybrid). Both InMemoryRetrieval and PgRetrievalService now report `effective_mode` in diagnostics. Files: `src/in_memory.rs`, `src/pg/retrieval.rs`. 30 memory unit tests pass. BLOCKER for item 3: `memory_proposed` ownership decision needs product owner — cannot resolve from this seat.
- 2026-04-03 | Worker 6 / Manager | `submit_pack()` and the old feed-warning cleanup are no longer the active story.
- 2026-04-03 | Worker 6 | All three manager-directed fixes landed:
  - `submit_pack()`: parses RFC 013 bundle JSON, extracts knowledge_document artifacts, ingests through pipeline. Test: `tests/bundle_roundtrip.rs::submit_pack_ingests_knowledge_documents`. File: `crates/cairn-memory/src/pipeline.rs`.
  - Retrieval mode honesty (InMemory + Postgres): both `InMemoryRetrieval` and `PgRetrievalService` now reject `VectorOnly` with explicit error, `Hybrid` falls back to `LexicalOnly` and reports `effective_mode` in diagnostics. Files: `src/in_memory.rs`, `src/pg/retrieval.rs`.
  - `signal_feed_integration.rs` warning cleaned: `base_time` now used in `created_at` formatting.
- 2026-04-03 | Week 1 assigned | Scaffold `cairn-memory` and `cairn-graph`, define ingest/query/graph interfaces, and align storage needs to Worker 3.
- 2026-04-03 | Worker 6 / Manager | `cairn-memory` and `cairn-graph` scaffold complete | Ingest, retrieval, diagnostics, deep-search service boundaries in cairn-memory. Graph projections (typed nodes/edges), product-shaped graph queries (6 v1 query families), and provenance service boundaries in cairn-graph. Both crates depend on cairn-domain. All 11 tests pass.
- 2026-04-03 | Week 2 assigned | Implement document and graph entity persistence skeletons, align retrieval and graph storage requirements to schema reality.
- 2026-04-03 | Worker 6 / Manager | Week 2 complete | `PgDocumentStore` and `PgGraphStore` with Postgres persistence. Both feature-gated behind `postgres`. All 23 tests passing.
- 2026-04-03 | Week 3 assigned | Implement ingest pipeline, retrieval query path, graph projection flow for runtime events.
- 2026-04-03 | Worker 6 / Manager | Week 3 complete | `IngestPipeline`, `PgRetrievalService` (FTS), `EventProjector`. 28 tests passing.
- 2026-04-03 | Week 4 assigned | Complete first owned retrieval flow for supported document floor, expose graph-backed provenance for runtime/operator use.
- 2026-04-03 | Worker 6 / Manager | Week 4 complete | In-memory e2e retrieval, `GraphProvenanceService`. 37 tests passing.
- 2026-04-03 | Wave 3 gate work | SQLite local-mode retrieval: `SqliteDocumentStore` + `SqliteRetrievalService` (FTS5). `InMemoryDiagnostics` (source quality tracking, index status). FTS5 virtual table + sync triggers in SQLite schema. `sqlite` feature flag for cairn-memory. 39 tests passing across all 3 crates.
- 2026-04-03 | **Wave 3 gate met** | Owned retrieval replaces Bedrock KB. Local-mode works. Graph provenance queryable.
- 2026-04-03 | Wave 4 support | `EvalGraphProjector` (prompt asset/version/release/eval_run graph linkage), `RetrievalGraphProjector` (source/document/chunk provenance), `GraphAwareIngestPipeline` (auto-projects to graph on ingest).
- 2026-04-03 | API integration | `MemoryApiImpl` (MemoryEndpoints), `FeedStore` (FeedEndpoints). 52 tests.
- 2026-04-03 | Deep search + bundles | `IterativeDeepSearch` (multi-hop with quality gates, dedup, keyword decomposition). RFC 013 `BundleEnvelope`/`ArtifactEntry`/import plan/report types for knowledge pack and prompt library import. 64 tests passing (60 default + 4 sqlite).

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker 6 | Follow-on packed sequence: 1. keep `submit_pack()` closed, 2. add or tighten one focused retrieval-mode guard proving the real backend path still rejects `VectorOnly` and reports `Hybrid -> LexicalOnly` honestly, 3. then pair with Worker 8 on the smallest `memory_proposed` owner proposal and leave a precise blocker if we still need an explicit product call.
- 2026-04-03 | Manager -> Worker 6 | Packed next cut: 1. keep `submit_pack()` closed and do not reopen it, 2. tighten the retrieval-mode contract with one focused guard proving `VectorOnly` rejection and `Hybrid -> LexicalOnly` diagnostics stay explicit in the real backend path, 3. then pair with Worker 8 on the smallest real `memory_proposed` ownership decision and leave a precise blocker if the product owner is still undecided.
- 2026-04-03 | Manager -> Worker 6 | Packed next cut: 1. clean the current `signal_feed_integration.rs` warning, 2. make `PgRetrievalService` mode behavior honest by either tightening diagnostics/contracts around lexical fallback or rejecting ambiguous hybrid claims more explicitly, 3. if that lands cleanly, pair with Worker 8 on the smallest `memory_proposed` ownership decision and leave a blocker if ownership still needs product clarification.
- 2026-04-03 | Manager -> Worker 6 | Correction after code check: `submit_pack()` is already implemented, so stop treating knowledge-pack ingest as the primary gap. New concrete direction: make retrieval-mode behavior honest across the real backends. In particular, `PgRetrievalService` still rejects `VectorOnly` and lets `Hybrid` fall back to lexical; either make that fallback explicit in diagnostics/contracts or tighten the mode surface so callers cannot mistake lexical fallback for full hybrid retrieval. Also clean the current `signal_feed_integration.rs` warning while you are in the slice.
- 2026-04-03 | Manager -> Worker 6 | Clarification: re-do the real memory task. Target `submit_pack()` and retrieval-mode honesty first. Acceptable completion here is code/test updates in `cairn-memory` or an explicit blocker tied to the exact missing seam. Do not finish with generic notes like `verified`, `no drift`, or `all tests green`.
- 2026-04-03 | Manager -> Worker 6 | Immediate pickup now: 1. implement the smallest real `submit_pack()` path using the current RFC 013 bundle types so knowledge-pack ingest stops hard-failing, 2. make retrieval mode behavior explicit by either implementing the minimal vector or hybrid path now or tightening the mode contract/tests so `Hybrid` cannot quietly masquerade as full hybrid, 3. keep the scope narrow and API-visible.
- 2026-04-03 | Manager -> Worker 6 | Immediate pickup now: 1. pair with Worker 8 on one executable app/router proof that hits a provenance-backed read or feed path through real `cairn-memory` services, 2. add one integration test proving the HTTP-facing read is backed by actual retrieval/provenance services rather than documented wiring, 3. if both pass, take one narrow feed-or-signal follow-up without widening the retrieval model.
- 2026-04-03 | Manager -> Worker 6 | Follow-on handwritten direction after that: 1. keep `MemoryApiImpl` / `FeedEndpoints` / provenance seams honest for Worker 8, 2. prefer one representative integration proof at a time, 3. avoid broad new memory features unless an API seam is truly blocked.
- 2026-04-03 | Manager -> Worker 6 | Immediate handwritten direction after the first fix: 1. pair with Worker 8 on one executable app/router proof that hits `MemoryApiImpl` plus one provenance-backed read, 2. add one integration test proving those HTTP-facing reads are backed by real retrieval/provenance services instead of documented wiring only, 3. if both are green, take one representative follow-up on feed or signal-backed memory exposure without widening the retrieval model.
- 2026-04-03 | Manager -> Worker 6 | Ongoing handwritten direction: 1. pair with Worker 8 to replace documented wiring with executable router coverage for `MemoryApiImpl`, `FeedEndpoints`, and provenance reads, 2. make sure those reads are backed by real service calls, 3. if time remains, add one representative provenance/search integration proof rather than widening memory scope.
- 2026-04-03 | Manager -> Worker 6 | Pacing note: expect Worker 8 to push for executable router coverage, not just documented wiring. Be ready to pair on `MemoryApiImpl`, `FeedEndpoints`, and provenance-service-backed reads so the API layer can prove the product-glue path end to end.
- 2026-04-03 | Manager -> Worker 6 | Latest manager read: your slice has moved beyond the original retrieval/provenance floor. Current next focus is support mode: help Worker 8 and the app/router layer consume `MemoryApiImpl`, `FeedEndpoints`, and provenance services cleanly, but avoid widening memory scope unless an integration gap forces it.
- 2026-04-03 | Manager -> Worker 6 | Current next focus: finish the API-facing memory/provenance seam with Worker 8. Prioritize wiring `MemoryApiImpl<R>` into the HTTP boundary cleanly and keep provenance/feed reads backed by real service calls, not placeholders.
- 2026-04-03 | Architecture Owner -> Worker 6 | Week 1 focus: memory and graph service skeletons with storage-facing interfaces, not deep implementation.
- 2026-04-03 | Worker 1 -> Worker 6 | Align ingest/query/graph storage assumptions with Worker 3 before hardening persistence paths. Treat RFC 013 import shape as fixed input.
- 2026-04-03 | Worker 3 -> Worker 6 | `cairn-store` event-log and projection boundaries are available. Graph persistence should use store interfaces as the write-side contract for durable state.
- 2026-04-03 | Worker 7 -> Worker 6 | `cairn-evals` prompt registry types and graph-linkable IDs are available. Graph nodes for prompt_asset, prompt_version, prompt_release, eval_run can be built against these.
- 2026-04-03 | Worker 1 / Manager -> Worker 6 | Current next focus: take the Week 4 owned-core slice. Close one first owned retrieval flow end-to-end and make graph-backed provenance queryable enough for runtime/operator consumption, using the API and runtime seams already in place.
- 2026-04-03 | Worker 1 / Manager -> Worker 6 | Concrete next cut: close the API-facing provenance and feed/signal seam. Pair with Worker 8 so `feed_update`, `poll_completed`, memory search, and provenance reads are backed by explicit `cairn-memory`/`cairn-graph` service calls instead of compatibility-only placeholders.
- 2026-04-03 | Worker 8 -> Worker 6 | Pairing request: `memory_proposed` SSE publisher ownership. Builder exists in cairn-api (`build_memory_proposed_frame` wraps full `MemoryItem`). Question: should cairn-memory's proposal flow call this builder when a memory is proposed, or should this go through a RuntimeEvent? Worker 8 is blocked on this decision.

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
- 2026-04-03 | Worker 6 -> Worker 7 | Wave 4: `EvalGraphProjector` projects prompt asset/version/release/eval_run into graph with DerivedFrom, ReleasedAs, RolledBackTo, EvaluatedBy, UsedPrompt edges. Call `on_asset_created`, `on_version_created`, `on_release_created`, `on_eval_run_created`, `on_release_rollback`, `on_prompt_used` from prompt/eval services.
- 2026-04-03 | Worker 6 -> Worker 7 | Wave 4: `RetrievalGraphProjector` projects source/document/chunk into graph with DerivedFrom, EmbeddedAs, Cited, ReadFrom edges. Enables retrieval provenance queries for eval integration.
- 2026-04-03 | Worker 6 -> Worker 8 | Wave 4: All graph projectors ready for API wiring. `EvalGraphProjector` + `RetrievalGraphProjector` + `EventProjector` together provide full graph coverage for operator provenance surfaces.

## Ready For Review

- 2026-04-03 | Worker 6 | Review `crates/cairn-memory/*` for Week 1 memory scaffold: ingest service (source types, chunking, embedding pipeline), retrieval service (lexical/vector/hybrid with inspectable scoring), diagnostics service, and deep-search service.
- 2026-04-03 | Worker 6 | Review `crates/cairn-graph/*` for Week 1 graph scaffold: graph projection (typed nodes/edges per RFC 004), product-shaped graph queries (6 v1 query families), and provenance service (execution and retrieval provenance chains).
- 2026-04-03 | Worker 6 | Review `crates/cairn-memory/src/pg/*` and `crates/cairn-graph/src/pg/*` for Week 2 Postgres persistence: document/chunk store and graph store with BFS traversal.
- 2026-04-03 | Worker 6 | Review Week 3: `crates/cairn-memory/src/pipeline.rs` (ingest pipeline + chunker), `crates/cairn-memory/src/pg/retrieval.rs` (Postgres FTS retrieval), `crates/cairn-graph/src/event_projector.rs` (runtime event -> graph projection). 28 tests passing.
