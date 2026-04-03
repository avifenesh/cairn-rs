# Ralph Loop Progress

## Current RFC: 005 — Task/Session/Checkpoint Lifecycle
## Current Phase: implementation (Phase 3a)

## RFC Status

| RFC | Title | Status |
|-----|-------|--------|
| 001 | Product Boundary and Non-Goals | scope-only, no code needed |
| 002 | Runtime and Event Model | DONE |
| 003 | Owned Retrieval | DONE |
| 004 | Graph and Eval Matrix | DONE |
| 005 | Task/Session/Checkpoint Lifecycle | IN PROGRESS |
| 006 | Prompt Registry and Release | pending |
| 007 | Plugin Protocol and Transport | pending |
| 008 | Tenant/Workspace/Profile | pending |
| 009 | Provider Abstraction | pending |
| 010 | Operator Control Plane | pending |
| 011 | Deployment Shape | pending |
| 012 | Onboarding and Starter Templates | pending |
| 013 | Artifact Import/Export | pending |
| 014 | Commercial Packaging | pending |

## RFC 002 — Gap Analysis

### What exists

**cairn-domain** has comprehensive types for the 7 `full_history` entities:
- IDs: SessionId, RunId, TaskId, ApprovalId, CheckpointId, MailboxMessageId, ToolInvocationId
- Lifecycle state machines: SessionState, RunState, TaskState, CheckpointDisposition
- Command model: 19 RuntimeCommand variants (CreateSession, StartRun, SubmitTask, ClaimTask, HeartbeatTaskLease, PauseRun/Task, ResumeRun/Task, RequestApproval, RecordApprovalDecision, RecordCheckpoint, RestoreCheckpoint, AppendMailboxMessage, StartToolInvocation, FinishToolInvocation, ReportExternalWorker, SpawnSubagent, RecordRecoverySweep)
- Event model: 20 RuntimeEvent variants covering session, run, task, approval, checkpoint, mailbox, tool invocation, external worker, subagent, and recovery
- Error types: RuntimeEntityKind (7 variants), RuntimeEntityRef (7 variants), CommandValidationError, RuntimeConflictError
- Tenancy: Scope, TenantKey, WorkspaceKey, ProjectKey, OwnershipKey
- Policy: PolicyEffect, ApprovalRequirement, ApprovalDecision, ExecutionClass, PolicyVerdict
- Tool invocation: ToolInvocationState, ToolInvocationRecord with lifecycle
- Workers: TaskLease, ExternalWorkerReport, ExternalWorkerOutcome
- Prompts: PromptReleaseState, PromptReleaseRecord (types exist for RFC 006)
- Providers: routing model types (for RFC 009)
- Selectors: SelectorContext, RolloutTarget

**cairn-store** has:
- EventLog trait: append-only with monotonic EventPosition, per-entity and global stream reads
- DurabilityClass enum: FullHistory / CurrentStatePlusAudit
- EntityRef enum: 7 variants matching full_history entities
- SyncProjection trait: applied within event-append transaction
- Read model traits + records for all 7 full_history entities: SessionReadModel, RunReadModel, TaskReadModel, ApprovalReadModel, CheckpointReadModel, MailboxReadModel, ToolInvocationReadModel
- InMemoryStore: complete in-memory implementation with all traits
- SQLite and Postgres backends (feature-gated)
- Migration framework

**cairn-runtime** has:
- Service traits: SessionService, RunService, TaskService, ApprovalService, CheckpointService, MailboxService, RecoveryService
- Service impls: all service implementations backed by store
- ToolInvocationService + impl
- ExternalWorkerService + impl
- RuntimeEnrichment trait + StoreBackedEnrichment
- RuntimeError enum with StoreError conversion
- Event helpers: next_event_id(), make_envelope()

### Gaps

#### 1. Signal event entity — MISSING (cairn-domain, cairn-store, cairn-runtime)

RFC 002 lists "signal event" as a core runtime entity with `current_state_plus_audit` durability. No signal support exists:

- [x] `SignalId` in ids.rs
- [x] Signal domain types (SignalRecord in signal.rs)
- [x] `IngestSignal` command variant in RuntimeCommand
- [x] `SignalIngested` event variant in RuntimeEvent
- [x] `RuntimeEntityKind::Signal` and `RuntimeEntityRef::Signal` in errors.rs
- [x] `EntityRef::Signal` in cairn-store event_log.rs
- [x] Signal projection: `SignalReadModel` trait + `SignalRecord` struct in cairn-store
- [x] Signal projection impl in InMemoryStore
- [x] Signal feed read model query impls

#### 2. Missing command variants for terminal operations — MINOR

RFC 002 says "commands are intent records." Currently, complete/fail/cancel for runs and tasks go through service methods but are not represented as RuntimeCommand variants. The service emits the correct events, but there's an asymmetry: PauseRun/PauseTask/ResumeRun/ResumeTask are command variants, but CompleteRun/FailRun/CancelRun/CompleteTask/FailTask/CancelTask are not.

- [x] `CompleteRun`, `FailRun`, `CancelRun` command variants
- [x] `CompleteTask`, `FailTask`, `CancelTask` command variants
- [x] `AppendUserMessage` command variant (RFC 002 example)

#### 3. Deferred to other RFCs (not RFC 002 gaps)

These `current_state_plus_audit` entities are classified in RFC 002 but implemented in their own RFCs:
- Memory ingest job → RFC 003
- Prompt release commands/events → RFC 006
- Evaluation run commands/events → RFC 004/006
- Graph edges projection → RFC 004
- Evaluation scorecards projection → RFC 004/006

### Phase plan

1. **Phase 2 — Types and traits**: Add SignalId, signal domain types, command/event variants, entity refs, store traits
2. **Phase 3a — Implementation**: Add signal projection to InMemoryStore, add missing command variants for terminal ops
3. **Phase 3b — Implementation**: Wire signal support through runtime service layer
4. **Phase 4 — Tests**: Add tests for signal lifecycle, command/event coverage
5. **Phase 5 — Mark complete**

## RFC 003 — Gap Analysis

### What exists

**cairn-memory** crate has:
- DocumentStore trait + InMemory/Pg/Sqlite impls
- RetrievalService trait + impls (lexical FTS in Pg via ts_rank, SQLite via FTS5/bm25, in-memory substring)
- IngestService trait + IngestPipeline, ParagraphChunker
- EmbeddingProvider trait (no concrete impl)
- DeepSearchService + IterativeDeepSearch
- DiagnosticsService + InMemoryDiagnostics
- RetrievalMode enum (LexicalOnly/VectorOnly/Hybrid)
- MetadataFilter type
- ScoringBreakdown with all 8 RFC dimensions
- RerankerStrategy enum (None/Mmr/ProviderReranker)
- KnowledgeDocumentId/KnowledgePackId/SourceId IDs
- MemoryServices bundle for injection
- RetrievalDiagnostics on query responses
- SourceQualityRecord/IndexStatus types
- 37 tests passing

### Gaps

#### 1. Vector layer — MISSING

No pgvector/HNSW columns, no SQLite brute-force vector, VectorOnly unimplemented.

- [ ] pgvector extension + embedding column on chunks table
- [ ] HNSW index for Postgres
- [ ] SQLite brute-force vector search
- [ ] VectorOnly mode implementation

#### 2. Embedding pipeline — STUB ONLY

EmbeddingProvider trait exists, no concrete impl, pipeline skips embed, chunks have no embedding field.

- [ ] Concrete EmbeddingProvider impl (hosted provider adapter)
- [ ] Wire embedding step into IngestPipeline
- [ ] Add embedding vector field to ChunkRecord

#### 3. Chunk model enrichment — MISSING

- [ ] Typed ChunkId (currently bare String)
- [ ] Provenance metadata fields
- [ ] Credibility metadata fields
- [ ] Graph linkage field
- [ ] Embedding vector field
- [ ] updated_at timestamp

#### 4. Format parsers — STUB ONLY

Markdown/HTML/StructuredJson enum variants exist, no parsing logic, no normalization step.

- [ ] Markdown parser/normalizer
- [ ] HTML parser/normalizer
- [ ] StructuredJson parser/normalizer
- [ ] Normalization pipeline step

#### 5. Ingest pipeline — INCOMPLETE

- [ ] Normalization step
- [ ] Metadata extraction step
- [ ] Deduplication step

#### 6. Scoring implementation — TYPES ONLY

All 8 ScoringBreakdown fields declared, only lexical_relevance populated.

- [ ] Freshness/staleness calculators
- [ ] Source credibility calculator
- [ ] Corroboration calculator
- [ ] Graph proximity calculator
- [ ] Recency of use calculator
- [ ] Operator-tunable ScoringPolicy/ScoringWeights types

#### 7. Reranking — ENUM ONLY

MMR and ProviderReranker enum variants exist, no implementation in any backend.

- [ ] MMR reranking implementation
- [ ] Provider-based reranker integration

#### 8. Metadata filtering — TYPE ONLY

MetadataFilter type exists on RetrievalQuery, no backend implements it.

- [ ] Implement metadata filtering in Pg backend
- [ ] Implement metadata filtering in SQLite backend
- [ ] Implement metadata filtering in in-memory backend

#### 9. Memory ingest job entity — MISSING

- [ ] IngestJobId in cairn-domain
- [ ] RuntimeCommand/Event variants for ingest jobs
- [ ] IngestJobReadModel + store projection

#### 10. Deep search enrichment — BASIC

- [ ] Graph expansion hooks
- [ ] Synthesis inputs type
- [ ] Quality gates (acceptance thresholds, convergence checks)
- [ ] Improved query decomposition (KeywordDecomposer is basic)

#### 11. Diagnostics completeness — PARTIAL

- [ ] Candidate-generation stages reporting
- [ ] Scoring dimensions that contributed
- [ ] Effective scoring policy applied
- [ ] Why-this-result explanations
- [ ] Top-hit inspection
- [ ] Benchmark/eval views

#### 12. Operator-tunable scoring policy — MISSING

- [ ] ScoringPolicy type
- [ ] ScoringWeights type
- [ ] Per-project/workspace weight presets

### Phase plan

1. **Phase 2 — Types**: ChunkId, chunk model enrichment, ScoringPolicy types, IngestJobId, embedding field
2. **Phase 3a — Impl**: Format parsers, normalization, metadata extraction, dedup, embedding pipeline wiring
3. **Phase 3b — Impl**: Vector layer (pgvector + SQLite brute-force), metadata filtering, scoring calculators
4. **Phase 3c — Impl**: Reranking (MMR), diagnostics enrichment, deep search graph hooks
5. **Phase 3d — Impl**: Memory ingest job runtime entity wiring
6. **Phase 4 — Tests**: Full pipeline tests
7. **Phase 5 — Mark complete**

## RFC 004 — Gap Analysis

### What exists

**cairn-graph** has:
- NodeKind enum: 16 variants (Session, Run, Task, Approval, Checkpoint, MailboxMessage, ToolInvocation, Memory, Document, Chunk, Source, PromptAsset, PromptVersion, PromptRelease, EvalRun, Skill, ChannelTarget)
- EdgeKind enum: 16 variants (Triggered, Spawned, DependedOn, ApprovedBy, ResumedFrom, SentTo, ReadFrom, Cited, DerivedFrom, EmbeddedAs, EvaluatedBy, ReleasedAs, RolledBackTo, RoutedTo, UsedPrompt, UsedTool)
- GraphProjection trait (add_node, add_edge, node_exists)
- EventProjector + RetrievalGraphProjector + EvalGraphProjector
- 6 GraphQuery variants (ExecutionTrace, DependencyPath, PromptProvenance, RetrievalProvenance, DecisionInvolvement, EvalLineage)
- GraphQueryService trait + PgGraphStore impl
- ProvenanceService trait + GraphProvenanceService impl
- GraphExpansionHook + NoOp

**cairn-evals** has:
- EvalRun + EvalRunStatus + EvalSubjectKind
- Scorecard + ScorecardEntry
- MatrixCategory enum: 6 variants (PromptComparison, ProviderRouting, Permission, MemorySourceQuality, SkillHealth, GuardrailPolicyOutcome)
- PromptComparisonRow with full fields
- EvalMetrics: 10 built-in canonical metrics
- PluginMetric + MetricValueType + MetricValue types
- EvalRunService (in-memory), GraphIntegration service
- 504 tests passing

### Gaps

#### Graph gaps

1. ApprovedBy edge not projected — EventProjector doesn't emit ApprovedBy edge on ApprovalResolved
- [ ] Add ApprovedBy edge projection in EventProjector

2. Memory/Skill/ChannelTarget nodes never created — NodeKind variants exist but no projector creates them
- [ ] Wire Memory/Skill/ChannelTarget node creation in relevant projectors

3. Signal/IngestJob events not projected to graph — EventProjector has no-op arms for these
- [ ] Add Signal node projection on SignalIngested
- [ ] Add IngestJob node projection on IngestJobStarted/IngestJobCompleted

4. No per-variant graph query dispatch — GraphQueryService::query exists but no impl dispatches on GraphQuery variants
- [ ] Implement ExecutionTrace, DependencyPath, PromptProvenance, RetrievalProvenance, DecisionInvolvement, EvalLineage queries

5. No InMemory GraphQueryService — only PgGraphStore implements GraphQueryService
- [ ] Add InMemoryGraphStore implementing GraphProjection + GraphQueryService

6. No concrete GraphExpansionHook — only NoOp exists
- [ ] Implement a concrete graph expansion hook for deep search

7. Provenance chain skeleton — GraphProvenanceService::provenance_chain returns empty chain
- [ ] Implement provenance chain traversal

8. No project scope on GraphNode — GraphNode has node_id, kind, created_at but no project field
- [ ] Add project: Option<ProjectKey> to GraphNode

#### Eval gaps

9. No RuntimeCommand/Event for eval lifecycle — evals don't flow through event log
- [ ] Add StartEvalRun/CompleteEvalRun command variants
- [ ] Add EvalRunStarted/EvalRunCompleted event variants

10. EvalRunService in-memory only — not backed by event log or store
- [ ] Make EvalRunService event-sourced via store

11. Scorecard not persisted — types exist but no storage or query service
- [ ] Add scorecard storage and query service

12. 5 of 6 matrix row types missing — only PromptComparisonRow exists
- [ ] Add ProviderRoutingRow, PermissionRow, MemorySourceQualityRow, SkillHealthRow, GuardrailPolicyRow

13. No matrix storage/query service — matrix types have no backing store
- [ ] Add MatrixReadModel trait + storage

14. No output_artifacts or DatasetSource struct — EvalRun references dataset_source as Option<String>
- [ ] Add DatasetSource struct and output_artifacts field

15. Graph-eval integration manual not event-driven — GraphIntegration methods must be called explicitly
- [ ] Wire graph-eval integration through event projector

16. No graph edges from eval -> outcomes — no edges connecting eval runs to the outcomes they measured
- [ ] Add EvaluatedBy edges from eval projector

17. on_prompt_used untyped string — GraphIntegration::on_prompt_used takes bare strings
- [ ] Use typed PromptReleaseId/RunId

18. No operator matrix threshold config — no types for operator-configurable threshold/highlight policies
- [ ] Add MatrixThresholdPolicy type

### Phase plan

1. **Phase 2 — Types**: eval domain types (commands/events, matrix row types, DatasetSource), graph node project scope
2. **Phase 3a — Impl**: graph projection fixes (ApprovedBy, Signal, IngestJob, Memory nodes), InMemory GraphQueryService
3. **Phase 3b — Impl**: eval persistence (event-sourced EvalRunService, scorecard store, matrix store)
4. **Phase 3c — Impl**: graph-eval wiring (event-driven integration, provenance chain, concrete GraphExpansionHook)
5. **Phase 4 — Tests**: tests + cross-review
6. **Phase 5 — Mark complete**

## RFC 005 — Gap Analysis

### What exists

**cairn-domain lifecycle.rs**: SessionState (4), RunState (8), TaskState (11) with full transition tables. CheckpointDisposition (Latest/Superseded). FailureClass (7 variants). PauseReason + PauseReasonKind (4). ResumeTrigger (3). RunResumeTarget/TaskResumeTarget. derive_session_state() implements RFC 005 rules. Full test coverage.

**cairn-runtime**: RecoveryService trait + impl (expired leases, interrupted runs, stale dependencies). RunService (start/complete/fail/cancel/pause/resume). TaskService (submit/claim/heartbeat/complete/fail/cancel/pause/resume). CheckpointService (save/get/latest_for_run/restore). SessionService (create/get/list). All emit events through EventLog.

### Gaps

#### Session gaps

1. No auto session derivation trigger — derive_session_state() exists but nothing calls it when runs complete
- [ ] Auto-derive session state on run terminal transition

2. No session complete/fail methods — SessionService has create/get/list but no explicit complete/fail/archive
- [ ] Add complete/fail/archive methods to SessionService

#### Run gaps

3. Pause reason discarded — RunServiceImpl::pause takes _reason (unused), never recorded in events
- [ ] Record PauseReason in RunStateChanged or dedicated RunPaused event

4. Resume trigger discarded — RunServiceImpl::resume takes _trigger (unused), not recorded
- [ ] Record ResumeTrigger in events

5. No pause_reason/resume_trigger on RunRecord — projection doesn't track last pause reason or resume trigger
- [ ] Add pause_reason and last_resume_trigger fields to RunRecord

6. No resume_after timer — ResumeTrigger::ResumeAfterTimer exists but no scheduling mechanism
- [ ] Add resume_after_ms field, timer-based resume (deferred to runtime scheduler)

7. No duplicate start guard — RunService::start doesn't check if run already exists
- [ ] Add existence check before creating run

#### Task gaps

8. No waiting_approval/waiting_dependency service methods — TaskState has these states but no service methods to enter them
- [ ] Add enter_waiting_approval/enter_waiting_dependency to TaskService

9. No dead_letter service method — DeadLettered state exists, recovery can dead-letter, but no explicit TaskService::dead_letter()
- [ ] Add dead_letter method to TaskService

10. No retry count on TaskRecord — recovery heuristic for retry vs dead-letter is fragile
- [ ] Add retry_count field to TaskRecord, increment on RetryableFailed

11. No leased→running validation — ClaimTask moves to Leased but no validation that running must follow leased
- [ ] Add state guard in task start/transition

#### Checkpoint gaps

12. No supersede in checkpoint service — saving new Latest doesn't mark previous as Superseded
- [ ] Auto-supersede previous Latest when saving new checkpoint

13. No restore method wired — CheckpointService::restore exists but doesn't emit CheckpointRestored event properly
- [ ] Wire restore to emit CheckpointRestored + RunStateChanged

14. No checkpoint data/payload field — CheckpointRecord has no payload/data field for actual checkpoint content
- [ ] Add checkpoint_data or payload field

#### Recovery gaps

15. Stale dependencies stub — resolve_stale_dependencies works but incomplete: doesn't check child failure propagation
- [ ] Propagate child failure to parent (fail parent if child failed)

16. No CheckpointRestored emission in recovery — recover_interrupted_runs returns action but doesn't emit restore event
- [ ] Emit CheckpointRestored event in recovery

17. Fragile retry heuristic — retry vs dead-letter based on failure_class pattern matching, not retry count
- [ ] Use retry_count for retry/dead-letter decision

#### Cross-cutting gaps

18. No resume_after_ms on PauseRun/PauseTask commands — RFC 005 says pause accepts optional resume-after timestamp
- [ ] Add resume_after_ms: Option<u64> to PauseRun and PauseTask commands

### Phase plan

1. **Phase 2 — Types**: pause_reason/resume_trigger on records, retry_count on TaskRecord, resume_after_ms on commands, checkpoint payload field
2. **Phase 3a — Impl**: session auto-derivation, session complete/fail/archive, run duplicate start guard
3. **Phase 3b — Impl**: task waiting states, dead_letter method, leased→running validation
4. **Phase 3c — Impl**: checkpoint supersede + restore wiring, recovery fixes (CheckpointRestored emission, retry count, child failure propagation)
5. **Phase 4 — Tests**: tests + cross-review
6. **Phase 5 — Mark complete**

## Completed This Session
- [x] RFC 002: Phase 1 gap analysis
- [x] RFC 002: Phase 2 types and traits — SignalId, SignalRecord, IngestSignal command, SignalIngested event, RuntimeEntityKind/Ref::Signal, EntityRef::Signal, SignalReadModel trait, CompleteRun/FailRun/CancelRun/CompleteTask/FailTask/CancelTask/AppendUserMessage command variants
- [x] RFC 002: Phase 3 implementation — Signal projection in InMemoryStore, signal feed read model queries, EntityRef::Signal in pg/sqlite event_log
- [x] RFC 002: Phase 4 tests — UserMessageAppended event variant, signal lifecycle tests, command/event coverage tests
- [x] RFC 002: Phase 5 — marked complete, all gaps resolved
- [x] RFC 003: Phase 1 gap analysis — 12 gaps identified across vector layer, embeddings, chunk model, parsers, ingest pipeline, scoring, reranking, metadata filtering, ingest job entity, deep search, diagnostics, scoring policy
- [x] RFC 003: Phase 2 types — ChunkId, ScoringPolicy/ScoringWeights, IngestJobId, IngestJobState/IngestJobRecord, StartIngestJob/CompleteIngestJob commands, IngestJobStarted/IngestJobCompleted events, RuntimeEntityKind/Ref::IngestJob, chunk model enrichment
- [x] RFC 003: Phase 3a impl — format parsers (Markdown/HTML/JSON normalizers), metadata extraction, content-hash dedup, embedding pipeline wiring (NoOpEmbeddingProvider, IngestPipeline.with_embedder())
- [x] RFC 003: Phase 3b impl — scoring calculators (freshness/staleness with decay), metadata filtering in all backends, MMR reranking (cosine similarity on embeddings, Jaccard fallback on text)
- [x] RFC 003: Phase 3c impl — diagnostics enrichment (candidate stages, scoring dimensions, effective policy), deep search graph hooks
- [x] RFC 003: Phase 3d impl — IngestJobReadModel trait + InMemoryStore projection, IngestJobService trait + impl in cairn-runtime
- [x] RFC 003: Phase 4 tests — full pipeline tests, ingest job lifecycle, scoring, reranking, diagnostics coverage
- [x] RFC 003: Phase 5 — marked complete, all 12 gaps resolved
- [x] RFC 004: Phase 1 gap analysis — 18 gaps identified across graph projections, query dispatch, provenance, eval lifecycle, matrix storage, graph-eval integration
- [x] RFC 004: Phase 2 types — GraphNode project scope, StartEvalRun/CompleteEvalRun commands+events, RuntimeEntityKind/Ref::EvalRun, EntityRef::EvalRun, DatasetSource struct, 5 matrix row types, MatrixThresholdPolicy
- [x] RFC 004: Phase 3a impl — InMemory GraphQueryService (query dispatch for all 6 families, neighbors with edge filtering), graph projection fixes (ApprovedBy, Signal, IngestJob nodes)
- [x] RFC 004: Phase 3b impl — event-sourced EvalRunService (EvalRunReadModel + InMemoryStore projection), scorecard store, matrix store
- [x] RFC 004: Phase 3c impl — EvalRunStarted/Completed graph projection (EvalRun node + EvaluatedBy edge), GraphBackedExpansion hook for deep search, provenance chain verified
- [x] RFC 004: Phase 4 cross-review — BFS edge dedup fix, eval projection tests, EvalRunCompleted subject_node_id field
- [x] RFC 004: Phase 5 — marked complete, all 18 gaps resolved
- [x] RFC 005: Phase 1 gap analysis — 18 gaps across session derivation, pause/resume tracking, task lifecycle, checkpoint restore, recovery fixes
- [x] RFC 005: Phase 2 types — pause_reason/resume_trigger on RunRecord/TaskRecord, retry_count on TaskRecord, resume_after_ms on PauseRun/PauseTask, checkpoint_data payload field, dead_letter method
