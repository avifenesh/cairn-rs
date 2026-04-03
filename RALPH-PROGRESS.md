# Ralph Loop Progress

## Current RFC: 003 — Owned Retrieval
## Current Phase: implementation (Phase 3a)

## RFC Status

| RFC | Title | Status |
|-----|-------|--------|
| 001 | Product Boundary and Non-Goals | scope-only, no code needed |
| 002 | Runtime and Event Model | DONE |
| 003 | Owned Retrieval | IN PROGRESS |
| 004 | Graph and Eval Matrix | pending |
| 005 | Task/Session/Checkpoint Lifecycle | pending |
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

## Completed This Session
- [x] RFC 002: Phase 1 gap analysis
- [x] RFC 002: Phase 2 types and traits — SignalId, SignalRecord, IngestSignal command, SignalIngested event, RuntimeEntityKind/Ref::Signal, EntityRef::Signal, SignalReadModel trait, CompleteRun/FailRun/CancelRun/CompleteTask/FailTask/CancelTask/AppendUserMessage command variants
- [x] RFC 002: Phase 3 implementation — Signal projection in InMemoryStore, signal feed read model queries, EntityRef::Signal in pg/sqlite event_log
- [x] RFC 002: Phase 4 tests — UserMessageAppended event variant, signal lifecycle tests, command/event coverage tests
- [x] RFC 002: Phase 5 — marked complete, all gaps resolved
- [x] RFC 003: Phase 1 gap analysis — 12 gaps identified across vector layer, embeddings, chunk model, parsers, ingest pipeline, scoring, reranking, metadata filtering, ingest job entity, deep search, diagnostics, scoring policy
- [x] RFC 003: Phase 2 types — ChunkId, ScoringPolicy/ScoringWeights, IngestJobId, IngestJobState/IngestJobRecord, StartIngestJob/CompleteIngestJob commands, IngestJobStarted/IngestJobCompleted events, RuntimeEntityKind/Ref::IngestJob, chunk model enrichment
