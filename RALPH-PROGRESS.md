# Ralph Loop Progress

## Current RFC: ALL COMPLETE
## Current Phase: done

## RFC Status

| RFC | Title | Status |
|-----|-------|--------|
| 001 | Product Boundary and Non-Goals | scope-only, no code needed |
| 002 | Runtime and Event Model | DONE |
| 003 | Owned Retrieval | DONE |
| 004 | Graph and Eval Matrix | DONE |
| 005 | Task/Session/Checkpoint Lifecycle | DONE |
| 006 | Prompt Registry and Release | DONE |
| 007 | Plugin Protocol and Transport | DONE |
| 008 | Tenant/Workspace/Profile | DONE |
| 009 | Provider Abstraction | DONE |
| 010 | Operator Control Plane | DONE |
| 011 | Deployment Shape | DONE |
| 012 | Onboarding and Starter Templates | DONE |
| 013 | Artifact Import/Export | DONE |
| 014 | Commercial Packaging | DONE |

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

- [x] pgvector extension + embedding column on chunks table
- [x] HNSW index for Postgres
- [x] SQLite brute-force vector search
- [x] VectorOnly mode implementation

#### 2. Embedding pipeline — STUB ONLY

EmbeddingProvider trait exists, no concrete impl, pipeline skips embed, chunks have no embedding field.

- [x] Concrete EmbeddingProvider impl (hosted provider adapter)
- [x] Wire embedding step into IngestPipeline
- [x] Add embedding vector field to ChunkRecord

#### 3. Chunk model enrichment — MISSING

- [x] Typed ChunkId (currently bare String)
- [x] Provenance metadata fields
- [x] Credibility metadata fields
- [x] Graph linkage field
- [x] Embedding vector field
- [x] updated_at timestamp

#### 4. Format parsers — STUB ONLY

Markdown/HTML/StructuredJson enum variants exist, no parsing logic, no normalization step.

- [x] Markdown parser/normalizer
- [x] HTML parser/normalizer
- [x] StructuredJson parser/normalizer
- [x] Normalization pipeline step

#### 5. Ingest pipeline — INCOMPLETE

- [x] Normalization step
- [x] Metadata extraction step
- [x] Deduplication step

#### 6. Scoring implementation — TYPES ONLY

All 8 ScoringBreakdown fields declared, only lexical_relevance populated.

- [x] Freshness/staleness calculators
- [x] Source credibility calculator
- [x] Corroboration calculator
- [x] Graph proximity calculator
- [x] Recency of use calculator
- [x] Operator-tunable ScoringPolicy/ScoringWeights types

#### 7. Reranking — ENUM ONLY

MMR and ProviderReranker enum variants exist, no implementation in any backend.

- [x] MMR reranking implementation
- [x] Provider-based reranker integration

#### 8. Metadata filtering — TYPE ONLY

MetadataFilter type exists on RetrievalQuery, no backend implements it.

- [x] Implement metadata filtering in Pg backend
- [x] Implement metadata filtering in SQLite backend
- [x] Implement metadata filtering in in-memory backend

#### 9. Memory ingest job entity — MISSING

- [x] IngestJobId in cairn-domain
- [x] RuntimeCommand/Event variants for ingest jobs
- [x] IngestJobReadModel + store projection

#### 10. Deep search enrichment — BASIC

- [x] Graph expansion hooks
- [x] Synthesis inputs type
- [x] Quality gates (acceptance thresholds, convergence checks)
- [x] Improved query decomposition (KeywordDecomposer is basic)

#### 11. Diagnostics completeness — PARTIAL

- [x] Candidate-generation stages reporting
- [x] Scoring dimensions that contributed
- [x] Effective scoring policy applied
- [x] Why-this-result explanations
- [x] Top-hit inspection
- [x] Benchmark/eval views

#### 12. Operator-tunable scoring policy — MISSING

- [x] ScoringPolicy type
- [x] ScoringWeights type
- [x] Per-project/workspace weight presets

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
- [x] Add ApprovedBy edge projection in EventProjector

2. Memory/Skill/ChannelTarget nodes never created — NodeKind variants exist but no projector creates them
- [x] Wire Memory/Skill/ChannelTarget node creation in relevant projectors

3. Signal/IngestJob events not projected to graph — EventProjector has no-op arms for these
- [x] Add Signal node projection on SignalIngested
- [x] Add IngestJob node projection on IngestJobStarted/IngestJobCompleted

4. No per-variant graph query dispatch — GraphQueryService::query exists but no impl dispatches on GraphQuery variants
- [x] Implement ExecutionTrace, DependencyPath, PromptProvenance, RetrievalProvenance, DecisionInvolvement, EvalLineage queries

5. No InMemory GraphQueryService — only PgGraphStore implements GraphQueryService
- [x] Add InMemoryGraphStore implementing GraphProjection + GraphQueryService

6. No concrete GraphExpansionHook — only NoOp exists
- [x] Implement a concrete graph expansion hook for deep search

7. Provenance chain skeleton — GraphProvenanceService::provenance_chain returns empty chain
- [x] Implement provenance chain traversal

8. No project scope on GraphNode — GraphNode has node_id, kind, created_at but no project field
- [x] Add project: Option<ProjectKey> to GraphNode

#### Eval gaps

9. No RuntimeCommand/Event for eval lifecycle — evals don't flow through event log
- [x] Add StartEvalRun/CompleteEvalRun command variants
- [x] Add EvalRunStarted/EvalRunCompleted event variants

10. EvalRunService in-memory only — not backed by event log or store
- [x] Make EvalRunService event-sourced via store

11. Scorecard not persisted — types exist but no storage or query service
- [x] Add scorecard storage and query service

12. 5 of 6 matrix row types missing — only PromptComparisonRow exists
- [x] Add ProviderRoutingRow, PermissionRow, MemorySourceQualityRow, SkillHealthRow, GuardrailPolicyRow

13. No matrix storage/query service — matrix types have no backing store
- [x] Add MatrixReadModel trait + storage

14. No output_artifacts or DatasetSource struct — EvalRun references dataset_source as Option<String>
- [x] Add DatasetSource struct and output_artifacts field

15. Graph-eval integration manual not event-driven — GraphIntegration methods must be called explicitly
- [x] Wire graph-eval integration through event projector

16. No graph edges from eval -> outcomes — no edges connecting eval runs to the outcomes they measured
- [x] Add EvaluatedBy edges from eval projector

17. on_prompt_used untyped string — GraphIntegration::on_prompt_used takes bare strings
- [x] Use typed PromptReleaseId/RunId

18. No operator matrix threshold config — no types for operator-configurable threshold/highlight policies
- [x] Add MatrixThresholdPolicy type

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
- [x] Auto-derive session state on run terminal transition

2. No session complete/fail methods — SessionService has create/get/list but no explicit complete/fail/archive
- [x] Add complete/fail/archive methods to SessionService

#### Run gaps

3. Pause reason discarded — RunServiceImpl::pause takes _reason (unused), never recorded in events
- [x] Record PauseReason in RunStateChanged or dedicated RunPaused event

4. Resume trigger discarded — RunServiceImpl::resume takes _trigger (unused), not recorded
- [x] Record ResumeTrigger in events

5. No pause_reason/resume_trigger on RunRecord — projection doesn't track last pause reason or resume trigger
- [x] Add pause_reason and last_resume_trigger fields to RunRecord

6. No resume_after timer — ResumeTrigger::ResumeAfterTimer exists but no scheduling mechanism
- [x] Add resume_after_ms field, timer-based resume (deferred to runtime scheduler)

7. No duplicate start guard — RunService::start doesn't check if run already exists
- [x] Add existence check before creating run

#### Task gaps

8. No waiting_approval/waiting_dependency service methods — TaskState has these states but no service methods to enter them
- [x] Add enter_waiting_approval/enter_waiting_dependency to TaskService

9. No dead_letter service method — DeadLettered state exists, recovery can dead-letter, but no explicit TaskService::dead_letter()
- [x] Add dead_letter method to TaskService

10. No retry count on TaskRecord — recovery heuristic for retry vs dead-letter is fragile
- [x] Add retry_count field to TaskRecord, increment on RetryableFailed

11. No leased→running validation — ClaimTask moves to Leased but no validation that running must follow leased
- [x] Add state guard in task start/transition

#### Checkpoint gaps

12. No supersede in checkpoint service — saving new Latest doesn't mark previous as Superseded
- [x] Auto-supersede previous Latest when saving new checkpoint

13. No restore method wired — CheckpointService::restore exists but doesn't emit CheckpointRestored event properly
- [x] Wire restore to emit CheckpointRestored + RunStateChanged

14. No checkpoint data/payload field — CheckpointRecord has no payload/data field for actual checkpoint content
- [x] Add checkpoint_data or payload field

#### Recovery gaps

15. Stale dependencies stub — resolve_stale_dependencies works but incomplete: doesn't check child failure propagation
- [x] Propagate child failure to parent (fail parent if child failed)

16. No CheckpointRestored emission in recovery — recover_interrupted_runs returns action but doesn't emit restore event
- [x] Emit CheckpointRestored event in recovery

17. Fragile retry heuristic — retry vs dead-letter based on failure_class pattern matching, not retry count
- [x] Use retry_count for retry/dead-letter decision

#### Cross-cutting gaps

18. No resume_after_ms on PauseRun/PauseTask commands — RFC 005 says pause accepts optional resume-after timestamp
- [x] Add resume_after_ms: Option<u64> to PauseRun and PauseTask commands

### Phase plan

1. **Phase 2 — Types**: pause_reason/resume_trigger on records, retry_count on TaskRecord, resume_after_ms on commands, checkpoint payload field
2. **Phase 3a — Impl**: session auto-derivation, session complete/fail/archive, run duplicate start guard
3. **Phase 3b — Impl**: task waiting states, dead_letter method, leased→running validation
4. **Phase 3c — Impl**: checkpoint supersede + restore wiring, recovery fixes (CheckpointRestored emission, retry count, child failure propagation)
5. **Phase 4 — Tests**: tests + cross-review
6. **Phase 5 — Mark complete**

## RFC 006 — Gap Analysis

### What exists

**cairn-evals** has comprehensive in-memory types and services:
- PromptAsset (kind: System/UserTemplate/ToolPrompt/Critic/Router, status: Active/Deprecated/Archived)
- PromptVersion (immutable, content, format: PlainText/Mustache/Jinja2, content_hash, metadata)
- PromptRelease (project-scoped, 6-state lifecycle: Draft/Proposed/Approved/Active/Rejected/Archived, rollout_target)
- ReleaseAction (7 action types, actor, reason, from/to release linkage)
- SelectorKind (ProjectDefault/AgentType/TaskType/RoutingSlot) with precedence
- SelectorResolver with deterministic resolution
- PromptReleaseService: in-memory with lifecycle validation, activation uniqueness, rollback
- EvalRunService: in-memory with scorecard building
- GraphIntegration: wraps EvalGraphProjector for prompt/eval graph nodes and edges

**cairn-domain** has: PromptReleaseState, PromptReleaseRecord, PromptReleaseKey, can_transition_prompt_release() with governance presets

**cairn-graph** has: NodeKind variants (PromptAsset/PromptVersion/PromptRelease/EvalRun), EdgeKind variants (DerivedFrom/ReleasedAs/UsedPrompt/EvaluatedBy/RolledBackTo), EvalGraphProjector

### Gaps

#### 1. No RuntimeCommand/Event for prompt lifecycle — MISSING

Prompt create/transition/activate/rollback don't flow through the event log.

- [x] Add CreatePromptAsset, CreatePromptVersion, CreatePromptRelease commands
- [x] Add PromptAssetCreated, PromptVersionCreated, PromptReleaseCreated, PromptReleaseStateChanged events

#### 2. No store-backed persistence — ALL IN-MEMORY

PromptReleaseService and EvalRunService use HashMap/Vec, not event-sourced.

- [x] Event-source prompt asset/version/release through cairn-store EventLog
- [x] Wire InMemoryStore projections for prompt entities

#### 3. No PromptAssetService or PromptVersionService — MISSING

Only PromptReleaseService exists. No service for creating/managing assets and versions.

- [x] Add PromptAssetService (create, get, list, deprecate)
- [x] Add PromptVersionService (create, get, list_by_asset)

#### 4. No read model traits in cairn-store — MISSING

No PromptAssetReadModel, PromptVersionReadModel, or PromptReleaseReadModel.

- [x] Add read model traits + InMemoryStore impls

#### 5. No release action persistence — MISSING

ReleaseAction type exists but actions not stored durably. No audit trail query service.

- [x] Persist release actions through events
- [x] Add release action query service

#### 6. No runtime prompt binding on runs/tasks — MISSING

RFC 006 says runs/tasks/tool_invocations must record prompt_release_id. No such fields exist.

- [x] Add prompt_release_id fields to RunRecord/TaskRecord/ToolInvocationRecord

#### 7. No approval integration — MISSING

PromptReleaseState has Proposed/Approved/Rejected but no connection to runtime ApprovalService.

- [x] Wire release approval through the runtime approval model

#### 8. No approval policy type — MISSING

RFC 006 defines approval policy (default requires review, project can relax). No ApprovalPolicy type.

- [x] Add PromptApprovalPolicy type (Standard/Regulated presets)

#### 9. GraphIntegration uses untyped strings — MINOR

on_prompt_used takes bare &str parameters instead of typed IDs.

- [x] Use typed PromptReleaseId/RunId

#### 10. No operator read models — MISSING

RFC 006 requires prompt asset list/detail, version history, release list/detail, comparison, approval queue.

- [x] Add operator-facing read model endpoints

### Phase plan

1. **Phase 2 — Types**: commands/events for prompt lifecycle, read model traits, approval policy type
2. **Phase 3a — Impl**: event-sourced asset/version/release services, InMemoryStore projections
3. **Phase 3b — Impl**: runtime prompt binding, approval integration, release action persistence
4. **Phase 4 — Tests**: tests + cross-review
5. **Phase 5 — Mark complete**

## RFC 007 — Gap Analysis

### What exists

**cairn-plugin-proto**: Full JSON-RPC 2.0 wire types (Request/Response/Error/Notification), InitializeParams/Result, ToolsInvokeParams/Result, ToolsListResult, ToolDescriptorWire, ScopeWire, ActorWire, RuntimeLinkageWire, CapabilityFamily, InvocationStatus, PluginManifestWire. Method constants for all 11 RPC methods.

**cairn-tools**: PluginManifest (id/name/version/command/capabilities/permissions/limits/execution_class), PluginCapability (6 families), PluginState (7 states), PluginHost trait, Permission (6 types), DeclaredPermissions, InvocationGrants, PermissionCheckResult, PermissionGate trait, ExecutionClass configs (SupervisedProcess/SandboxedProcess), PluginProcess (stdio transport with spawn/send/recv/kill), plugin_bridge (initialize/shutdown/tools_list/tools_invoke builders), RuntimeToolService trait + impl, InvocationService trait, pipeline functions.

**cairn-domain**: ToolInvocationState, ToolInvocationRecord, ToolInvocationTarget, ExecutionClass.
**cairn-store**: ToolInvocationReadModel.
**cairn-runtime**: ToolInvocationService trait + impl.

### Gaps

#### 1. No concrete PluginHost impl — MISSING

PluginHost trait exists but no concrete host that manages spawn/handshake/shutdown lifecycle.

- [x] Implement StdioPluginHost with lifecycle management

#### 2. No concrete PermissionGate impl — MISSING

PermissionGate trait exists but no policy-backed concrete checker.

- [x] Implement PolicyBackedPermissionGate

#### 3. No SupervisedBoundary/SandboxedBoundary impls — STUB ONLY

Config types exist but no actual process isolation enforcement.

- [x] Implement SupervisedBoundary (env restriction, working dir scope)
- [x] Implement SandboxedBoundary stub (document concrete backend requirements)

#### 4. Missing 6/11 RPC builders — INCOMPLETE

Only initialize/shutdown/tools_list/tools_invoke/health_check are bridged. Missing: signals.poll, channels.deliver, hooks.post_turn, policy.evaluate, eval.score, cancel.

- [x] Add bridge functions for remaining 6 RPC methods

#### 5. No notification handler — MISSING

log.emit/progress.update/event.emit defined in proto but no host-side handler.

- [x] Add NotificationHandler trait + impl for log/progress/event notifications

#### 6. No end-to-end plugin execution — MISSING

No wiring from PluginHost→transport→invoke→result pipeline.

- [x] Wire full plugin invocation pipeline: discover → spawn → handshake → invoke → shutdown

#### 7. No concurrency enforcement — MISSING

PluginLimits.max_concurrency declared but not enforced by host.

- [x] Add semaphore-based concurrency limiter per plugin

#### 8. No health check — MISSING

health.check method defined, bridge exists, but no periodic health monitor or restart logic.

- [x] Add health check loop + restart-on-failure

#### 9. No plugin registry — MISSING

Manifests loaded but no durable registry or list-installed-plugins query.

- [x] Add PluginRegistry with discover-from-directory and list_plugins

#### 10. No cancel dispatcher — MISSING

cancel method defined in proto but no host-side cancel for in-flight invocations.

- [x] Add cancellation token propagation to in-flight invocations

### Phase plan

1. **Phase 2 — Types**: missing RPC builders (signals.poll, channels.deliver, hooks.post_turn, policy.evaluate, eval.score, cancel), notification handler types
2. **Phase 3a — Impl**: concrete PluginHost (StdioPluginHost), concrete PermissionGate, plugin registry
3. **Phase 3b — Impl**: end-to-end plugin execution pipeline, concurrency enforcement, cancel dispatcher
4. **Phase 4 — Tests**: tests + cross-review
5. **Phase 5 — Mark complete**

## RFC 008 — Gap Analysis

### What exists

**cairn-domain tenancy.rs** (solid foundation):
- Scope enum: System/Tenant/Workspace/Project with includes() hierarchy
- TenantKey, WorkspaceKey, ProjectKey: typed scope keys
- OwnershipKey enum: System/Tenant/Workspace/Project variants
- OperatorProfileKey: tenant-scoped (tenant_id + operator_id)
- All runtime entities carry ProjectKey universally
- All store read models filter by ProjectKey
- GraphNode has project: Option<ProjectKey>
- cairn-memory chunks/retrieval carry ProjectKey
- cairn-evals: PromptAsset has tenant/workspace scope, releases are project-scoped

### Gaps

#### 1. No TenantRecord/WorkspaceRecord/ProjectRecord — MISSING

Only scope keys exist. No durable entity types for tenants, workspaces, or projects.

- [x] Add TenantRecord, WorkspaceRecord, ProjectRecord structs

#### 2. No CRUD services — MISSING

No TenantService, WorkspaceService, or ProjectService for lifecycle management.

- [x] Add tenant/workspace/project CRUD services

#### 3. No OperatorProfile record or service — MISSING

OperatorProfileKey exists but no profile struct with preferences, no CRUD.

- [x] Add OperatorProfile struct with preferences
- [x] Add OperatorProfileService

#### 4. No credential model — MISSING

RFC 008 says tenant-scoped credentials for providers/channels. Nothing exists.

- [x] Add CredentialRecord and CredentialService

#### 5. No defaults layering service — MISSING

RFC 008 defines system→tenant→workspace→project→operator-local precedence.

- [x] Add DefaultsResolver with layered precedence

#### 6. No workspace membership model — MISSING

RFC 008 says workspace-scoped team membership and roles.

- [x] Add WorkspaceMembership type and service

#### 7. No role-based permissions — MISSING

RFC 008 says permissions evaluated against actor+tenant+workspace+project+capability.

- [x] Add multi-scope permission evaluator

#### 8. No lifecycle commands/events — MISSING

No RuntimeCommand/Event for tenant/workspace/project creation.

- [x] Add TenantCreated, WorkspaceCreated, ProjectCreated events

#### 9. No store projections — MISSING

No read model traits for tenant/workspace/project entities.

- [x] Add TenantReadModel, WorkspaceReadModel, ProjectReadModel

#### 10. No API scope context enforcement — MISSING

RFC 008 says every API request operates in explicit scope context.

- [x] Add scope context extraction and enforcement

### Phase plan

1. **Phase 2 — Types**: entity records (TenantRecord/WorkspaceRecord/ProjectRecord/OperatorProfile), lifecycle commands/events, credential model
2. **Phase 3a — Impl**: store projections + CRUD services (tenant, workspace, project, profile)
3. **Phase 3b — Impl**: defaults layering, credential service, workspace membership
4. **Phase 4 — Tests**: tests + cross-review
5. **Phase 5 — Mark complete**

## RFC 009 — Gap Analysis

### What exists

**cairn-domain providers.rs** (comprehensive routing types):
- OperationKind (Generate/Embed/Rerank), ProviderCapability (9 variants), ProviderBindingSettings
- RouteAttemptRecord, RouteDecisionRecord, ProviderCallRecord with full RFC 009 fields
- RouteAttemptDecision, RouteDecisionStatus, RouteDecisionReason, ProviderCallStatus, ProviderCallErrorClass
- RouteDecisionRecord.validate() with linkage validation
- Typed IDs: ProviderBindingId, ProviderConnectionId, ProviderCallId, ProviderModelId, RouteAttemptId, RouteDecisionId

**cairn-domain credentials.rs**: CredentialRecord (tenant-scoped, encrypted_value, provider_adapter)

### Gaps

1. No ProviderConnectionRecord/ProviderBindingRecord — [ ] Add connection and binding entity types
2. No provider adapter trait — [ ] Add GenerationProvider/EmbeddingProvider/RerankerProvider traits
3. No route policy type — [ ] Add RoutePolicyRecord with fallback chain
4. No route resolver service — [ ] Add RouteResolverService trait
5. No store projections — [ ] Add RouteDecisionReadModel, ProviderCallReadModel
6. No RuntimeCommand/Event for routing — [ ] Add RouteDecisionMade, ProviderCallCompleted events
7. No provider call execution — [ ] Add dispatch to provider endpoints
8. No capability check service — [ ] Add effective capability set computation
9. No cost accounting — [ ] Add cost aggregation service
10. No provider route template type — [ ] Add ProviderRouteTemplateRecord

### Phase plan

1. **Phase 2 — Types**: ProviderConnectionRecord, ProviderBindingRecord, routing events, route policy
2. **Phase 3 — Impl**: store projections, InMemoryStore impls, RouteResolverService
3. **Phase 4 — Tests + review**
4. **Phase 5 — Mark complete**

## RFC 012 — Gap Analysis

### What exists

**cairn-api bootstrap.rs**: BootstrapConfig (port, deployment_mode: Local/SelfHosted), ServerBootstrap trait, DeploymentMode enum. Basic server startup only.

**cairn-memory bundles.rs**: BundleEnvelope, ArtifactKind, BundleType for knowledge pack import. IngestPackRequest for curated knowledge packs.

### Gaps

1. No StarterTemplate type — [ ] Add StarterTemplateId, StarterTemplateCategory (KnowledgeAssistant/ApprovalGatedWorker/MultiStepWorkflow), StarterTemplate struct
2. No OnboardingFlow types — [ ] Add OnboardingFlowState, OnboardingStep, OnboardingProgress for tracking bootstrap
3. No template materialization service — [ ] Add TemplateMaterializationService that creates tenant/workspace/project + starter assets from template
4. No template registry — [ ] Add system-scoped StarterTemplateRegistry listing available templates
5. No bootstrap provenance — [ ] Add BootstrapProvenance recording which template, when, what was materialized
6. No prompt import service — [ ] Add canonical PromptImportService with reconciliation (match by id or name+hash, idempotent)
7. No import provenance model — [ ] Add ImportProvenanceRecord (source, timestamp, bundle ref, created/reused/skipped/conflicted)
8. No onboarding checklist — [ ] Add OnboardingChecklist type tracking setup completion steps
9. Bootstrap doesn't create product state — [ ] Wire bootstrap to create tenant/workspace/project/operator/provider via existing services
10. No shipped starter content — [ ] Add starter prompt/policy definitions for the 3 required template categories

### Phase plan

1. **Phase 2 — Types**: StarterTemplate, OnboardingFlow, ImportProvenance, BootstrapProvenance types
2. **Phase 3 — Impl**: template registry, materialization service, prompt import service
3. **Phase 4 — Tests + review**
4. **Phase 5 — Mark complete**

## RFC 014 — Gap Analysis

### What exists

Nothing. No entitlement, license, feature gating, or commercial packaging types in any crate.

### Gaps

1. No ProductTier type — [ ] Add ProductTier enum (LocalEval/TeamSelfHosted/EnterpriseSelfHosted)
2. No Entitlement type — [ ] Add Entitlement enum (DeploymentTier/GovernanceCompliance/AdvancedAdmin)
3. No EntitlementSet — [ ] Add EntitlementSet with active entitlements and inspection
4. No FeatureGate — [ ] Add FeatureGate trait for checking entitlement-gated capabilities
5. No LicenseRecord — [ ] Add LicenseRecord with tenant-scoped license state
6. No entitlement status API — [ ] Add EntitlementStatusEndpoint
7. No feature rollout flags — [ ] Add FeatureFlag type (Preview/GA/Gated)
8. No capability-to-entitlement mapping — [ ] Add capability→entitlement mapping
9. No entitlement change audit — [ ] Add EntitlementChangeRecord for audit
10. No degradation model — [ ] Add graceful degradation on entitlement absence

### Phase plan

1. **Phase 2 — Types**: ProductTier, Entitlement, EntitlementSet, LicenseRecord, FeatureFlag, FeatureGate
2. **Phase 3 — Impl**: entitlement checking, capability mapping, feature gating
3. **Phase 5 — Mark complete**

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
- [x] RFC 005: Phase 3a impl — session auto-derivation (derive_and_update_session wired to run complete/fail/cancel), duplicate run start guard, pause/resume metadata carried through events
- [x] RFC 005: Phase 3+4 — checkpoint restore, recovery hardening, cross-review
- [x] RFC 005: Phase 5 — marked complete
- [x] RFC 006: Phase 1 gap analysis — 10 gaps across prompt lifecycle events, persistence, read models, approval integration, runtime binding
- [x] RFC 006: Phase 2 types — prompt lifecycle commands/events (PromptAssetCreated/VersionCreated/ReleaseCreated/ReleaseTransitioned), read model traits (PromptAssetReadModel/VersionReadModel/ReleaseReadModel), PromptApprovalPolicy type, runtime binding fields
- [x] RFC 006: Phase 3a impl — InMemoryStore prompt projections, PromptAssetService/PromptVersionService/PromptReleaseService traits + impls, event-sourced persistence
- [x] RFC 006: Phase 3b+4 — approval policy enforcement, graph wiring, cross-review
- [x] RFC 006: Phase 5 — marked complete
- [x] RFC 007: Phase 1 gap analysis — 10 gaps across PluginHost impl, PermissionGate, RPC builders, notification handling, plugin execution pipeline, concurrency, registry
- [x] RFC 007: Phase 2 types — missing RPC builders, notification types, plugin registry
- [x] RFC 007: Phase 3 impl — StdioPluginHost, PolicyPermissionGate, plugin execution pipeline (ConcurrencyTracker, execute_plugin_tool), notification parsing
- [x] RFC 007: Phase 4 cross-review — duplicate registration guard in StdioPluginHost
- [x] RFC 007: Phase 5 — marked complete, all 10 gaps resolved
- [x] RFC 008: Phase 1 gap analysis — 10 gaps across entity records, CRUD services, operator profile, credentials, defaults layering, membership, permissions, lifecycle events
- [x] RFC 008: Phase 2+3 — entity records, CRUD services, store projections, defaults layering, credentials
- [x] RFC 008: Phase 5 — marked complete
- [x] RFC 009: Phase 1 gap analysis — 10 gaps across provider records, routing events, store projections, route resolver
- [x] RFC 009: Phase 2+3 — ProviderConnectionRecord, ProviderBindingRecord, RouteDecisionMade/ProviderCallCompleted events, RouteDecisionReadModel/ProviderCallReadModel + InMemoryStore impls, RouteResolverService trait
- [x] RFC 009: Phase 5 — marked complete
- [x] RFC 012: Phase 1 gap analysis — 10 gaps across starter templates, onboarding flow, materialization, import, provenance
- [x] RFC 012: Phase 2+3 — StarterTemplate/OnboardingProgress/BootstrapProvenance/ImportProvenance types, StarterTemplateRegistry (3 V1 templates), materialize_template(), onboarding checklist, prompt import reconciliation
- [x] RFC 012: Phase 5 — marked complete
- [x] RFC 014: Phase 1 gap analysis — 10 gaps across entitlements, licensing, feature gating, product tiers
- [x] RFC 014: Phase 2+3 — ProductTier (LocalEval/TeamSelfHosted/EnterpriseSelfHosted), Entitlement (4 categories), EntitlementSet, LicenseRecord, FeatureFlag, DefaultFeatureGate with V1 capability mappings, EntitlementChangeRecord
- [x] RFC 014: Phase 5 — marked complete

## Session 2026-04-06: Market-Ready Quality Pass

Multi-agent hardening session. All pre-existing test failures resolved, real provider wired.

- Durable MemoryApiImpl — replaced volatile HashMap with DocumentStore-backed persistence
- Chunk quality scoring — compute_chunk_quality() in pipeline make_chunk()
- Corroboration pass — cross-result scoring in InMemoryRetrieval (lexical ≥50% + embedding cosine >0.8)
- Recency-of-use — per-chunk retrieval timestamp tracking with tiered decay
- cairn-store latest_root_run — added run_id tiebreaker matching Pg/SQLite adapters
- cairn-runtime RunCostUpdated import fix — restored test-module import stripped by linter
- OpenAI-compatible provider adapter — GenerationProvider + EmbeddingProvider (reqwest, Bearer auth)
- Pipeline embedding wired — IngestPipeline.with_embedder() using qwen3-embedding:8b via agntic.garden
- SDK updates — TypeScript + Python: added createProviderConnection / list methods
- Final sweep: 2,636 passed, 0 failed, 7 ignored across 12 crates
