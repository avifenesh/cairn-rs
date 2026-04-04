# cairn-rs Code Review Report
Generated: 2026-04-04

This report accumulates findings from a line-by-line review of every file
in the cairn-rs workspace. Rounds cover: domain, store, runtime, memory,
graph/evals, tools/api/app. No fixes applied yet — findings only.

---

## Round 1 — cairn-domain, cairn-store, cairn-runtime

### cairn-domain (worker-1)

- `crates/cairn-domain/src/commercial.rs:157` — DefaultFeatureGate returns Allowed for unknown features; a missing mapping or typo silently disables entitlement enforcement
- `crates/cairn-domain/src/commands.rs:322` — ClaimTask.lease_owner is a raw String while WorkerId type exists; should use WorkerId
- `crates/cairn-domain/src/commands.rs:455` — RecordRecoverySweep allows both run_id and task_id to be None simultaneously; callers can emit an invalid sweep command with no target
- `crates/cairn-domain/src/commands.rs:544` — CreatePromptAsset.kind is a raw String even though PromptAssetKind enum exists; unvalidated at the boundary
- `crates/cairn-domain/src/commands.rs:569` — TransitionPromptRelease.to_state is a raw String; should be typed PromptReleaseState
- `crates/cairn-domain/src/commands.rs:576` — CreateTenant/CreateWorkspace/CreateProject all use ProjectKey for project() but these entities are not project-scoped; creates misleading ownership
- `crates/cairn-domain/src/defaults.rs:25` — LayeredDefaultsResolver relies entirely on caller to supply the correct scope chain; no validation that the chain is in the right order
- `crates/cairn-domain/src/errors.rs:11` — RuntimeEntityKind/RuntimeEntityRef cannot represent the newer entity types (IngestJob, EvalRun, PromptAsset, PromptVersion, PromptRelease, RouteDecision, ProviderCall, Tenant, Workspace, Project); error types lag the entity set
- `crates/cairn-domain/src/events.rs:417` — ToolInvocationCompleted and ToolInvocationFailed carry tool_name as a plain String; no validation it matches the invocation target
- `crates/cairn-domain/src/events.rs:453` — RecoveryAttempted allows both run_id and task_id to be None; same target ambiguity as the command
- `crates/cairn-domain/src/events.rs:470` — RecoveryCompleted same issue
- `crates/cairn-domain/src/events.rs:550` — PromptReleaseTransitioned.from_state and to_state are raw Strings; should be PromptReleaseState
- `crates/cairn-domain/src/events.rs:583` — RouteDecisionMade omits RFC 009 required fields: route_policy_id, selector_context, attempt_records
- `crates/cairn-domain/src/events.rs:595` — ProviderCallCompleted omits RFC 009 required fields: error_class, raw_error_message, retry_count
- `crates/cairn-domain/src/ids.rs:11` — All ID newtypes accept empty strings and whitespace-only strings; no validation on construction
- `crates/cairn-domain/src/lifecycle.rs:67` — PauseReason lacks the actor/source field RFC 005 specifies for audit trails
- `crates/cairn-domain/src/policy.rs:102` — Unused ApprovalMode import in test module
- `crates/cairn-domain/src/prompts.rs:58` — PromptReleaseRecord missing RFC 006 required fields: release_tag, rollout_target, created_by
- `crates/cairn-domain/src/prompts.rs:70` — PromptReleaseActionRecord missing RFC 006 required fields: reason, actor_id
- `crates/cairn-domain/src/providers.rs:120` — RouteDecisionRecord missing RFC 009 fields: route_policy_id, selector_context
- `crates/cairn-domain/src/providers.rs:135` — ProviderCallRecord missing RFC 009 fields: error_class, raw_error_message, retry_count
- `crates/cairn-domain/src/providers.rs:184` — validate_route_decision never checks that selected_binding_id is present when status is Selected
- `crates/cairn-domain/src/providers.rs:210` — Status-specific validation incomplete: FailedAfterDispatch doesn't require at least one failed attempt
- `crates/cairn-domain/src/selectors.rs:34` — RolloutTarget serializes as variant-specific objects; inconsistent with other tagged enums in the codebase
- `crates/cairn-domain/src/tenancy.rs:163` — Unused WorkspaceRole import in test module
- `crates/cairn-domain/src/tool_invocation.rs:80` — mark_started/mark_finished do not validate that timestamps are monotonically increasing
- `crates/cairn-domain/src/tool_invocation.rs:123` — Terminal validation never requires a finished_at timestamp
- `crates/cairn-domain/src/tool_invocation.rs:213` — ToolInvocationOutcomeKind does not distinguish between Timeout (infrastructure) and PermanentFailure (logic)
- `crates/cairn-domain/src/workers.rs:61` — validate_external_worker_report ignores lease_token; expired/stolen leases can report outcomes

---

### cairn-store (worker-2)

- `crates/cairn-store/src/pg/adapter.rs:548` — PgAdapter does not compile under postgres feature; RunRecord initialized without pause_reason, resume_trigger, prompt_release_id fields added by RFC 005/006
- `crates/cairn-store/src/pg/adapter.rs:627` — TaskRecord initialized without retry_count, pause_reason, resume_trigger, prompt_release_id
- `crates/cairn-store/src/pg/adapter.rs:737` — CheckpointRecord initialized without data field added in RFC 005
- `crates/cairn-store/src/pg/event_log.rs:220` — entity_ref_filter is non-exhaustive for newer EntityRef variants (Signal, IngestJob, EvalRun, RouteDecision, etc.); those entities cannot be filtered by entity
- `crates/cairn-store/src/pg/event_log.rs:26` — PgEventLog::append commits events without synchronous projection update; violates RFC 002 "projections must be synchronous with event commit"
- `crates/cairn-store/src/sqlite/event_log.rs:23` — SqliteEventLog::append has the same missing synchronous projection issue
- `crates/cairn-store/src/in_memory.rs:489` — append mutates projections and pushes events incrementally; a panic midway leaves projections partially updated and inconsistent
- `crates/cairn-store/src/in_memory.rs:1265` — set_task_lease mutates canonical task state without emitting an event; history/projection drift
- `crates/cairn-store/src/in_memory.rs:553` — event_matches_entity omits ExternalWorkerReported, SubagentSpawned, RecoveryAttempted, RecoveryCompleted, CheckpointRestored
- `crates/cairn-store/src/pg/event_log.rs:98` — read_by_entity relies on one top-level JSON column filter; does not handle events where entity_id is nested (e.g. SubagentSpawned.child_task_id)
- `crates/cairn-store/src/sqlite/event_log.rs:80` — SQLite has the same per-entity filtering gap
- `crates/cairn-store/src/event_log.rs:29` — EntityRef is missing variants for newer entities: EvalRun, RouteDecision, ProviderCall, IngestJob, Tenant, Workspace, Project, PromptRelease
- `crates/cairn-store/src/pg/projections.rs:44` — all UPDATE handlers ignore affected-row count; a missing record silently succeeds instead of returning NotFound
- `crates/cairn-store/src/sqlite/projections.rs:42` — same silent 0-row-updated issue
- `crates/cairn-store/src/in_memory.rs:95` — apply_projection silently drops events for missing records (if-let without else); should surface as error in strict mode
- `crates/cairn-store/src/pg/rebuild.rs` — rebuild swallows projection errors, keeps going; a corrupt event poisons the read model silently
- `crates/cairn-store/src/pg/projections.rs` — projection timestamps come from wall-clock on the writer, not from the event; replay produces wrong timestamps
- `crates/cairn-store/src/sqlite/projections.rs` — every ToolInvocationFailed becomes Completed state due to wrong enum arm mapping
- `crates/cairn-store/src/in_memory.rs` — TaskLeaseClaimed only sets lease fields in the record; does not transition state to Leased (state stays Queued after claim)
- `crates/cairn-store/src/in_memory.rs` — tool-invocation versioning diverges by outcome type; Completed increments version differently than Failed
- `crates/cairn-store/src/projections/mailbox.rs` — MailboxRecord only stores IDs and timestamps; missing message body, sender, delivery status that RFC 002 mailbox model requires
- `crates/cairn-store/src/projections/prompt.rs` — PromptReleaseRecord omits rollout_target, release_tag, created_by; query for active_for_selector cannot work correctly
- `crates/cairn-store/src/in_memory.rs` — active_for_selector ignores its selector argument; returns first active release regardless of selector
- `crates/cairn-store/src/pg/projections.rs` — signal/ingest/eval/prompt/route/provider events all treated as no-op; those entities have no Postgres read models
- `crates/cairn-store/src/sqlite/projections.rs` — same missing projection families
- `crates/cairn-store/src/pg/adapter.rs` — PgAdapter implements only the seven full-history read models; all RFC 003-009 read model traits unimplemented for Postgres
- `crates/cairn-store/src/sqlite/adapter.rs` — SqliteAdapter has the same missing read model impls
- `crates/cairn-store/src/sqlite/adapter.rs` — SQLite rehydrates RunRecord without pause_reason, resume_trigger, prompt_release_id
- `crates/cairn-store/src/sqlite/adapter.rs` — SQLite rehydrates TaskRecord without retry_count, pause_reason, resume_trigger, prompt_release_id
- `crates/cairn-store/src/sqlite/adapter.rs` — SQLite rehydrates ToolInvocationRecord with wrong state mapping (see Failed→Completed bug above)
- `crates/cairn-store/src/sqlite/adapter.rs` — SQLite rehydrates approvals with title and description as None always; those fields exist in schema but never read back
- `crates/cairn-store/src/sqlite/adapter.rs` — SQLite rehydrates checkpoints with data: None always; checkpoint payload never persisted or restored
- `crates/cairn-store/src/sqlite/adapter.rs` — sqlite::memory: with a pool creates per-connection in-memory databases; each connection sees a different empty database
- `crates/cairn-store/src/pg/event_log.rs` — head_position decodes MAX(position) into wrong type; returns None for non-empty log
- `crates/cairn-store/src/sqlite/event_log.rs` — SQLite has the same empty-log/wrong-type head_position bug
- `crates/cairn-store/src/error.rs` — StoreError has no variant for projection errors, poisoned mutex, or invariant violations; all such failures become generic Internal
- `crates/cairn-store/src/migration_check.rs` — validate_migration_files never inspects file content; only checks filenames
- `crates/cairn-store/src/migrations.rs` — pending() uses only max_applied; if migration N-1 was skipped, it will never be applied
- `crates/cairn-store/src/migrations.rs` — MigrationRunner trait is synchronous; blocks async runtime thread during schema migrations
- `crates/cairn-store/src/pg/migration_runner.rs` — semicolon-splitting migration SQL breaks for PL/pgSQL functions and $$ bodies
- `crates/cairn-store/src/in_memory.rs` — in-memory CheckpointReadModel::list_by_run sorts by created_at but checkpoint IDs are UUIDs; ordering is non-deterministic for same-millisecond checkpoints
- `crates/cairn-store/src/in_memory.rs` — route-decision projection fabricates RouteDecisionRecord fields from event; missing route_policy_id, selector_context
- `crates/cairn-store/src/in_memory.rs` — provider-call projection hardcodes None for error_class, raw_error_message, retry_count
- `crates/cairn-store/src/in_memory.rs` — now_millis() panics on clock skew (duration_since UNIX_EPOCH fails if system clock is before epoch)
- `crates/cairn-store/src/in_memory.rs` — every self.state.lock().unwrap() panics on mutex poison; one panic in a concurrent test poisons the store for all subsequent tests

---

### cairn-runtime (worker-3)

- `crates/cairn-runtime/src/services/event_helpers.rs:4` — EVENT_COUNTER resets to 1 on every process restart; event IDs are not globally unique across restarts
- `crates/cairn-runtime/src/tasks.rs:36` — TaskService contract never carries the lease token back to callers; callers cannot verify lease ownership
- `crates/cairn-runtime/src/services/task_impl.rs:77` — submit() has no duplicate guard; submitting the same task_id twice emits two TaskCreated events
- `crates/cairn-runtime/src/services/task_impl.rs:109` — claim timestamp uses SystemTime::now().unwrap(); should use unwrap_or_default
- `crates/cairn-runtime/src/services/task_impl.rs:139` — heartbeat() extends leases based on a caller-supplied expires_at with no server-side validation; callers can set arbitrarily far-future expirations
- `crates/cairn-runtime/src/services/task_impl.rs:154` — heartbeat timestamp generation same unwrap issue
- `crates/cairn-runtime/src/services/approval_impl.rs:35` — request() only emits ApprovalRequested; does not call ApprovalService to create the record in the store
- `crates/cairn-runtime/src/services/approval_impl.rs:74` — resolve() only updates the event log; does not verify the approval is in a resolvable state
- `crates/cairn-runtime/src/services/mailbox_impl.rs:34` — append() has no duplicate guard; same message_id can be appended multiple times
- `crates/cairn-runtime/src/services/signal_impl.rs:32` — ingest() has no duplicate/idempotency guard; same signal_id can be ingested multiple times
- `crates/cairn-runtime/src/services/signal_impl.rs:40` — ingest() assumes synchronous read-after-write consistency; InMemory projection is synchronous but other backends may not be
- `crates/cairn-runtime/src/services/ingest_job_impl.rs:39` — start() has no duplicate guard
- `crates/cairn-runtime/src/services/ingest_job_impl.rs:46` — start() assumes read-after-write; same issue as signal_impl
- `crates/cairn-runtime/src/services/ingest_job_impl.rs:61` — complete() does not load/verify current job state before completing; can double-complete
- `crates/cairn-runtime/src/services/external_worker_impl.rs:72` — report() never calls lease validation; expired or stolen leases can report outcomes
- `crates/cairn-runtime/src/services/external_worker_impl.rs:103` — Suspended { reason } arm does nothing; suspended tasks never transition back
- `crates/cairn-runtime/src/services/external_worker_impl.rs:106` — terminal worker outcomes do not transition the run state when all tasks are done
- `crates/cairn-runtime/src/services/recovery_impl.rs:112` — "interrupted run with checkpoint" path emits RunResumedFromCheckpoint action but never emits CheckpointRestored event or transitions the run
- `crates/cairn-runtime/src/services/recovery_impl.rs:177` — stale-dependency recovery resumes parent run without verifying parent is in a resumable state
- `crates/cairn-runtime/src/services/route_resolver_impl.rs:55` — resolve() returns an in-memory RouteDecisionRecord without emitting a RouteDecisionMade event; route decisions are not durable
- `crates/cairn-runtime/src/services/route_resolver_impl.rs:79` — route_decision_id/route_attempt_id generated inside the impl; not exposed to callers, can't be correlated
- `crates/cairn-runtime/src/checkpoints.rs:21` — CheckpointService::save() has no way to pass checkpoint data/payload; the data field added in RFC 005 is unreachable via the trait
- `crates/cairn-runtime/src/services/checkpoint_impl.rs:33` — save() never validates that the run exists or is in a checkpointable state
- `crates/cairn-runtime/src/services/run_impl.rs:78` — start() does not verify that the session referenced by session_id exists
- `crates/cairn-runtime/src/approvals.rs:20` — ApprovalService::request() allows both run_id and task_id to be None; invalid approval with no context
- `crates/cairn-runtime/src/eval_runs.rs:25` — EvalRunService::complete() has no subject-type validation; can mark a prompt eval complete on a retrieval eval run
- `crates/cairn-runtime/src/services/eval_run_impl.rs:22` — now_millis() uses .unwrap() on SystemTime
- `crates/cairn-runtime/src/services/eval_run_impl.rs:51` — start() assumes read-after-write consistency
- `crates/cairn-runtime/src/prompt_assets.rs:11` — PromptAssetService is project-scoped but RFC 006 says assets are workspace/tenant-scoped; wrong scope on the trait
- `crates/cairn-runtime/src/prompt_versions.rs:11` — same wrong scope issue
- `crates/cairn-runtime/src/services/prompt_asset_impl.rs:51` — create() assumes read-after-write consistency
- `crates/cairn-runtime/src/services/prompt_version_impl.rs:34` — create() never validates that the referenced prompt_asset_id exists
- `crates/cairn-runtime/src/services/prompt_version_impl.rs:51` — create() assumes read-after-write consistency
- `crates/cairn-runtime/src/prompt_releases.rs:16` — PromptReleaseService::create() missing RFC 006 required fields: release_tag, rollout_target, created_by
- `crates/cairn-runtime/src/services/prompt_release_impl.rs:51` — create() assumes read-after-write consistency
- `crates/cairn-runtime/src/services/prompt_release_impl.rs:78` — transition() accepts any string as to_state; no state machine validation
- `crates/cairn-runtime/src/services/prompt_release_impl.rs:95` — activate() deactivates previous release by archiving it; RFC 006 says deactivation should return to Approved, not Archive
- `crates/cairn-runtime/src/services/prompt_release_impl.rs:138` — rollback() archives the current release then activates target; if activation fails the current is already archived with no rollback
- `crates/cairn-runtime/src/mailbox.rs:20` — MailboxService::append() only accepts IDs and no message body/sender; cannot carry actual mailbox content
- `crates/cairn-runtime/src/services/tenant_impl.rs:48` — direct SystemTime conversion uses unwrap; should use unwrap_or_default
- `crates/cairn-runtime/src/services/tenant_impl.rs:60` — TenantCreated emitted under a ProjectKey but tenants are not project-scoped; ownership mismatch
- `crates/cairn-runtime/src/services/tenant_impl.rs:60` — create() assumes read-after-write consistency
- `crates/cairn-runtime/src/services/workspace_impl.rs:49` — same unwrap on SystemTime
- `crates/cairn-runtime/src/services/workspace_impl.rs:61` — WorkspaceCreated emitted under ProjectKey; same ownership mismatch
- `crates/cairn-runtime/src/services/workspace_impl.rs:61` — create() assumes read-after-write consistency
- `crates/cairn-runtime/src/services/project_impl.rs:48` — same unwrap on SystemTime
- `crates/cairn-runtime/src/services/project_impl.rs:53` — create() assumes read-after-write consistency
- `crates/cairn-runtime/src/services/tool_invocation_impl.rs:65` — now_ms() uses .unwrap() on SystemTime
- `crates/cairn-runtime/src/services/tool_invocation_impl.rs:104` — record_completed() does not check invocation is in Started state before completing
- `crates/cairn-runtime/src/services/tool_invocation_impl.rs:125` — record_failed() has same missing state check

---

## Round 2 — cairn-memory, cairn-graph, cairn-evals, cairn-tools, cairn-plugin-proto

### cairn-memory (worker-1)

- `crates/cairn-memory/src/api_impl.rs:22` — MemoryApiImpl uses a local HashMap for CRUD instead of DocumentStore; changes are not persisted
- `crates/cairn-memory/src/api_impl.rs:67` — list() ignores the project argument entirely; returns all documents across all projects
- `crates/cairn-memory/src/api_impl.rs:108` — search() maps retrieval chunks into MemoryItem without provenance or credibility fields
- `crates/cairn-memory/src/api_impl.rs:122` — create() ignores the project argument
- `crates/cairn-memory/src/bundles.rs:14` — BundleEnvelope.artifact_count is a separate field but never validated against artifacts.len()
- `crates/cairn-memory/src/bundles.rs:232` — PromptAssetPayload.kind and status are raw Strings even though typed enums exist
- `crates/cairn-memory/src/deep_search.rs:8` — DeepSearchRequest/Response do not carry project scope
- `crates/cairn-memory/src/deep_search_impl.rs:66` — KeywordDecomposer recomputes sub-queries by appending to the original; identical to the original query after 1 hop
- `crates/cairn-memory/src/deep_search_impl.rs:71` — KeywordDecomposer appends expansion terms unconditionally; no dedup of expansion terms
- `crates/cairn-memory/src/deep_search_impl.rs:158` — search() iterates 0..request.max_hops; a max_hops of 0 runs one hop anyway
- `crates/cairn-memory/src/deep_search_impl.rs:205` — graph expansion only attempted for NeedsExpansion status; quality gate ignores actual result quality
- `crates/cairn-memory/src/deep_search_impl.rs:208` — graph hook receives full RetrievalResponse but only uses results; no access to metadata
- `crates/cairn-memory/src/deep_search_impl.rs:218` — merged deep-search results are re-scored without freshness or staleness context
- `crates/cairn-memory/src/diagnostics.rs:6` — SourceQualityRecord only stores chunk_count and last_ingested; missing credibility_score, coverage, error_rate
- `crates/cairn-memory/src/diagnostics_impl.rs:28` — diagnostics keyed only by source_id string; project scope not enforced
- `crates/cairn-memory/src/diagnostics_impl.rs:61` — record_ingest() increments counters without thread safety (Mutex wraps the whole impl but inner DiagnosticsStore is separate)
- `crates/cairn-memory/src/feed_impl.rs:32` — FeedStore is global and ignores the project scope argument
- `crates/cairn-memory/src/feed_impl.rs:32` — push_item() appends duplicate IDs into the order list without dedup
- `crates/cairn-memory/src/feed_impl.rs:92` — has_more = results.len() >= limit is incorrect; returns true when exactly limit items returned, even if there are no more
- `crates/cairn-memory/src/graph_expansion.rs:68` — GraphBackedExpansion only traverses DerivedFrom/Cited/ReadFrom/EmbeddedAs edges; misses Spawned/Triggered for cross-run provenance
- `crates/cairn-memory/src/graph_expansion.rs:80` — graph expansion feeds raw node IDs back as additional queries; nodes may not be searchable text
- `crates/cairn-memory/src/graph_expansion.rs:103` — unused import in test module
- `crates/cairn-memory/src/graph_ingest.rs:39` — graph projection is best-effort (errors silently ignored); graph can fall out of sync with documents
- `crates/cairn-memory/src/graph_ingest.rs:51` — chunk graph projection is a stub; chunks are never linked to their source document nodes
- `crates/cairn-memory/src/in_memory.rs:49` — insert_document() overwrites existing documents without checking for conflicts
- `crates/cairn-memory/src/in_memory.rs:64` — update_status() returns Ok(()) even when document does not exist
- `crates/cairn-memory/src/in_memory.rs:75` — insert_chunks() blindly appends chunks; same chunk_id can be inserted multiple times
- `crates/cairn-memory/src/in_memory.rs:125` — InMemoryRetrieval rejects VectorOnly with an error; this is correct but the error message says "not supported" rather than surfacing a proper capability check
- `crates/cairn-memory/src/in_memory.rs:147` — metadata filters only match values that exist as string keys in provenance_metadata; numeric or boolean values never match
- `crates/cairn-memory/src/in_memory.rs:157` — lexical matching uses substring checks (contains); returns false positives for queries like "at" matching "batch"
- `crates/cairn-memory/src/in_memory.rs:176` — recency_enabled is never used; recency_of_use in ScoringBreakdown is always None
- `crates/cairn-memory/src/in_memory.rs:245` — MMR reranking hardcodes lambda = 0.5; ignores ScoringPolicy if present
- `crates/cairn-memory/src/pg/documents.rs:22` — PgDocumentStore does not implement the new chunk model fields (embedding, provenance_metadata, credibility_score, graph_linkage)
- `crates/cairn-memory/src/pg/documents.rs:90` — Postgres chunk persistence writes only a subset of ChunkRecord fields
- `crates/cairn-memory/src/pg/retrieval.rs:30` — Postgres retrieval diagnostics are thinner than InMemory; missing stages_used, scoring_dimensions_used
- `crates/cairn-memory/src/pg/retrieval.rs:67` — Postgres retrieval ignores ScoringPolicy, metadata filters, and reranker settings
- `crates/cairn-memory/src/pg/retrieval.rs:112` — retrieved Postgres ChunkRecords are missing new fields; embedding, provenance_metadata always None
- `crates/cairn-memory/src/pipeline.rs:83` — ParagraphChunker mishandles blank lines: consecutive blank lines produce empty chunks
- `crates/cairn-memory/src/pipeline.rs:102` — a single input line longer than max_chunk_size is returned as-is without splitting
- `crates/cairn-memory/src/pipeline.rs:139` — compute_content_hash() uses DefaultHasher which is not stable across Rust versions; hashes differ between builds
- `crates/cairn-memory/src/pipeline.rs:158` — provenance stores source_type via format!("{:?}") debug string, not serialized enum value
- `crates/cairn-memory/src/pipeline.rs:188` — normalize(KnowledgePack) is a pass-through; KnowledgePack JSON structure is not normalized
- `crates/cairn-memory/src/pipeline.rs:198` — strip_html() is a naive tag stripper; does not handle CDATA, comments, or script/style content
- `crates/cairn-memory/src/pipeline.rs:256` — strip_markdown() is heuristic and can lose content inside nested formatting
- `crates/cairn-memory/src/pipeline.rs:350` — extract_json_text() only collects string values; arrays of non-strings are skipped silently
- `crates/cairn-memory/src/pipeline.rs:542` — dedup only checks hashes already in the store before this batch; within-batch duplicates are not detected
- `crates/cairn-memory/src/pipeline.rs:559` — a single embedding failure aborts the whole ingest batch; no partial-commit strategy
- `crates/cairn-memory/src/pipeline.rs:582` — submit_pack() reads document.id as the external document reference but does not validate uniqueness within the pack
- `crates/cairn-memory/src/pipeline.rs:602` — submit_pack() hardcodes bundle source_type as KnowledgePack regardless of actual content type
- `crates/cairn-memory/src/reranking.rs:17` — mmr_rerank() never validates or clamps lambda; negative lambda inverts the diversity/relevance trade-off
- `crates/cairn-memory/src/reranking.rs:59` — tie handling in mmr_rerank() relies on partial_cmp which is non-deterministic for equal floats
- `crates/cairn-memory/src/reranking.rs:131` — cosine_similarity() silently truncates to the shorter vector's length; mismatched embedding dimensions produce wrong scores
- `crates/cairn-memory/src/retrieval.rs:57` — ScoringPolicy and ScoringWeights have no validation; weights can sum to >1 or be negative
- `crates/cairn-memory/src/retrieval.rs:177` — freshness_score() returns 1.0 for future created_at; no clamping for time-travel data
- `crates/cairn-memory/src/retrieval.rs:191` — staleness_penalty() returns 0.0 for negative threshold; no guard against misconfigured policy
- `crates/cairn-memory/src/retrieval.rs:211` — compute_final_score() clamps only the floor (max(0.0)); no ceiling, scores can exceed 1.0
- `crates/cairn-memory/src/services.rs:54` — InMemoryServices::new() builds deep_search without graph hook; always uses NoOpGraphExpansion
- `crates/cairn-memory/src/services.rs:57` — InMemoryDiagnostics is created as a standalone instance not shared with IngestPipeline; ingest telemetry is lost
- `crates/cairn-memory/src/sqlite/documents.rs:22` — SqliteDocumentStore does not implement new chunk model fields either

---

### cairn-graph (worker-2)

- `crates/cairn-graph/src/in_memory.rs:33` — every mutex access unwraps; any panic poisons the graph store
- `crates/cairn-graph/src/in_memory.rs:65` — add_node() overwrites existing nodes silently; version/history is lost
- `crates/cairn-graph/src/in_memory.rs:73` — add_edge() never deduplicates edges; same edge can be added multiple times
- `crates/cairn-graph/src/in_memory.rs:85` — query paths return empty subgraphs when root node doesn't exist rather than returning an error
- `crates/cairn-graph/src/in_memory.rs:90` — ExecutionTrace ignores root_kind parameter; traverses from any node regardless of type
- `crates/cairn-graph/src/in_memory.rs:285` — bfs_bidirectional off-by-one: starts depth at 0 but max_depth guard fires before first hop
- `crates/cairn-graph/src/in_memory.rs:339` — bfs_downstream same off-by-one issue
- `crates/cairn-graph/src/in_memory.rs:388` — bfs_upstream same off-by-one issue
- `crates/cairn-graph/src/event_projector.rs:57` — RunCreated ignores prompt_release_id field added in RFC 006; no UsedPrompt edge emitted at run start
- `crates/cairn-graph/src/event_projector.rs:79` — TaskCreated ignores prompt_release_id; same gap
- `crates/cairn-graph/src/event_projector.rs:90` — parent-task lineage encoded as DependedOn edge but RFC 004 specifies Spawned for subagent tasks
- `crates/cairn-graph/src/event_projector.rs:102` — ApprovalRequested creates Approval node but uses wrong edge kind; should be RequiresApproval not ApprovedBy
- `crates/cairn-graph/src/event_projector.rs:149` — ApprovalResolved is a no-op; ApprovedBy edge never emitted after approval decision
- `crates/cairn-graph/src/event_projector.rs:201` — ToolInvocationStarted ignores prompt_release_id; UsedPrompt edge not emitted for tool invocations
- `crates/cairn-graph/src/event_projector.rs:228` — SubagentSpawned ignores prompt_release_id; spawned session/run not linked to the prompt that triggered them
- `crates/cairn-graph/src/event_projector.rs:282` — prompt registry, org/profile, route decision, and provider call events are all no-ops; no graph nodes/edges for those entity families
- `crates/cairn-graph/src/event_projector.rs:311` — EvalRunCompleted writes eval_run_id into subject_node_id field of EvaluatedBy edge; should be the subject's own node id
- `crates/cairn-graph/src/retrieval_projector.rs:20` — source/document/chunk nodes use None for project scope; graph can't be filtered by project
- `crates/cairn-graph/src/retrieval_projector.rs:43` — document ingest adds SourceId→DocumentId edge as Cited; should be DerivedFrom or a dedicated IngestedInto edge
- `crates/cairn-graph/src/retrieval_projector.rs:70` — chunk creation adds Document→Chunk as DerivedFrom; correct direction but should be EmbeddedAs per RFC 004
- `crates/cairn-graph/src/retrieval_projector.rs:98` — on_chunk_cited and on_chunk_read are stubs that do nothing
- `crates/cairn-graph/src/eval_projector.rs:27` — prompt asset/version/release/eval node creation ignores project scope; all nodes have project: None
- `crates/cairn-graph/src/eval_projector.rs:98` — rollback adds RolledBackTo edge from new active to old; direction is inverted per RFC convention
- `crates/cairn-graph/src/eval_projector.rs:124` — eval-run creation only links to the subject release; does not link to dataset or evaluator
- `crates/cairn-graph/src/eval_projector.rs:146` — on_prompt_used takes bare strings; not validated as real node IDs
- `crates/cairn-graph/src/graph_provenance.rs:31` — query failures flattened to empty results; corrupt graph state is silent
- `crates/cairn-graph/src/graph_provenance.rs:117` — provenance_chain always deduplicates by node_id but not by edge; same node can appear via different edges
- `crates/cairn-graph/src/graph_provenance.rs:152` — compute_depth traverses entire graph for each node; O(N²) for large graphs
- `crates/cairn-graph/src/projections.rs:8` — NodeKind has no variant for RouteDecision, ProviderCall, Tenant, Workspace, Project, PromptVersion
- `crates/cairn-graph/src/pg/store.rs:27` — Postgres persistence ignores GraphNode.project scope field; nodes not scoped to tenant/workspace/project in DB
- `crates/cairn-graph/src/pg/store.rs:77` — per-variant query dispatch collapses all 6 GraphQuery types into a single generic BFS; no variant-specific traversal
- `crates/cairn-graph/src/pg/store.rs:155` — downstream/upstream traversal returns all edges regardless of direction requested
- `crates/cairn-graph/src/pg/store.rs:160` — both Postgres BFS traversals have the same off-by-one as InMemory
- `crates/cairn-graph/src/pg/store.rs:261` — unknown node kinds coerced to Session; data corruption for newer entity types
- `crates/cairn-graph/src/pg/store.rs:262` — Postgres graph reads always rehydrate project as None

---

### cairn-evals (worker-2)

- `crates/cairn-evals/src/matrices/mod.rs:33` — PromptComparisonRow.metrics is EvalMetrics but all fields are Option; no validation that at least task_success_rate is populated
- `crates/cairn-evals/src/matrices/mod.rs:62` — PermissionRow only stores eval_run_id and metrics; missing subject (policy_id, tool_id, actor_scope)
- `crates/cairn-evals/src/matrices/mod.rs:76` — MemorySourceQualityRow documented as having source_id but struct has no such field
- `crates/cairn-evals/src/matrices/mod.rs:88` — SkillHealthRow has no explicit skill identifier; skill_id field missing
- `crates/cairn-evals/src/matrices/mod.rs:100` — GuardrailPolicyRow has no policy identifier field
- `crates/cairn-evals/src/matrices/mod.rs:112` — every matrix row only stores EvalMetrics; no matrix-specific metric sets as RFC 004 specifies
- `crates/cairn-evals/src/scorecards/mod.rs:25` — EvalSubjectKind includes provider route and retrieval policy but no eval subject for those exists in cairn-domain
- `crates/cairn-evals/src/scorecards/mod.rs:75` — Scorecard is hard-coded to prompt_asset_id as the grouping key; cannot build scorecards for other subject kinds
- `crates/cairn-evals/src/prompts/releases.rs:77` — lifecycle rules always permit Draft→Proposed regardless of ApprovalPolicy; policy is ignored
- `crates/cairn-evals/src/services/selector_resolver.rs:27` — duplicate active releases for the same selector resolve non-deterministically (HashMap iteration order)
- `crates/cairn-evals/src/services/eval_service.rs:53` — create_run never validates that the subject (prompt_release_id etc.) exists
- `crates/cairn-evals/src/services/eval_service.rs:59` — eval service API has no update/delete; completed runs cannot be annotated
- `crates/cairn-evals/src/services/eval_service.rs:84` — create_run overwrites existing run_id silently
- `crates/cairn-evals/src/services/eval_service.rs:91` — no list by subject_kind or by date range; operator cannot filter eval runs
- `crates/cairn-evals/src/services/eval_service.rs:151` — build_scorecard iterates all completed runs; O(N) scan with no index
- `crates/cairn-evals/src/services/eval_service.rs:159` — build_scorecard silently skips runs with no prompt linkage; non-prompt evals disappear from scorecards
- `crates/cairn-evals/src/services/eval_service.rs:183` — now_millis() unwraps SystemTime
- `crates/cairn-evals/src/services/release_service.rs:81` — create silently overwrites existing release with same id
- `crates/cairn-evals/src/services/release_service.rs:135` — Active→Approved deactivation path is never reachable; deactivate_current is only called from activate()
- `crates/cairn-evals/src/services/release_service.rs:149` — activation demotes current active to Approved, but RFC 006 says the prior state should be restored, not forced to Approved
- `crates/cairn-evals/src/services/release_service.rs:197` — rollback never verifies the target release exists or is in a restorable state
- `crates/cairn-evals/src/services/release_service.rs:234` — rollback demotes current to Archived; RFC 006 says rollback should set it to Approved
- `crates/cairn-evals/src/services/release_service.rs:298` — now_millis() unwraps SystemTime
- `crates/cairn-evals/src/services/graph_integration.rs:13` — graph projection failures are silently ignored via if let Ok
- `crates/cairn-evals/src/services/graph_integration.rs:26` — graph integration never handles EvalRunCompleted to add outcome edge

---

### cairn-tools (worker-3)

- `crates/cairn-tools/src/permissions.rs:64` — file defines only the PermissionGate trait; PolicyBackedPermissionGate mentioned in docs is not implemented here
- `crates/cairn-tools/src/permissions.rs:65` — PermissionGate::check() receives only InvocationGrants; no access to DeclaredPermissions to validate grants are within declared scope
- `crates/cairn-tools/src/registry.rs:55` — InMemoryPluginRegistry uses lock().unwrap(); mutex poison panics entire process
- `crates/cairn-tools/src/registry.rs:76` — list_all() returns HashMap iteration order; non-deterministic for tests and operator views
- `crates/cairn-tools/src/plugin_host.rs:112` — handshake() never verifies that the response protocol_version matches the expected "1.0"
- `crates/cairn-tools/src/plugin_host.rs:114` — transport or deserialize failures return PluginHostError but leave the managed plugin in Handshaking state; never transitions to Failed
- `crates/cairn-tools/src/plugin_host.rs:160` — health_check() never validates the response body; any JSON response is treated as healthy
- `crates/cairn-tools/src/plugin_host.rs:187` — send_request() accepts the next frame from any plugin; if a different plugin's notification arrives first, it consumes it silently
- `crates/cairn-tools/src/plugin_host.rs:262` — shutdown() returns immediately for terminal states without waiting for the child process to exit
- `crates/cairn-tools/src/plugin_host.rs:274` — shutdown() never waits for a shutdown acknowledgement response from the plugin
- `crates/cairn-tools/src/transport.rs:64` — stderr is piped but never drained; plugin stderr fills its OS buffer and deadlocks the plugin process
- `crates/cairn-tools/src/transport.rs:112` — recv() only deserializes JsonRpcResponse; plugin-to-host notifications (log.emit, progress.update) are silently dropped
- `crates/cairn-tools/src/transport.rs:120` — recv() creates a fresh BufReader on every call; previous read state is discarded, breaking framing for multi-line JSON
- `crates/cairn-tools/src/transport.rs:122` — recv() is an unbounded blocking read with no timeout; a hung plugin blocks the calling thread forever
- `crates/cairn-tools/src/transport.rs:126` — recv() treats any empty line as process exit; valid JSON-RPC traffic with blank separator lines triggers early termination
- `crates/cairn-tools/src/transport.rs:139` — kill() maps every failure to TransportError::Io; cannot distinguish "already dead" from "permission denied"
- `crates/cairn-tools/src/plugin_executor.rs:61` — ConcurrencyTracker uses lock().unwrap(); poison panics
- `crates/cairn-tools/src/plugin_executor.rs:130` — execute_plugin_tool() discards the plugin process after each invocation; no process reuse for the same plugin
- `crates/cairn-tools/src/plugin_executor.rs:172` — execute_with_transport() sends the RPC then immediately calls recv(); no handling of interleaved notifications before the response
- `crates/cairn-tools/src/plugin_executor.rs:197` — execute_with_transport() assumes the first frame from the plugin is the response; notifications interleaved with the response are lost
- `crates/cairn-tools/src/plugin_executor.rs:201` — execute_with_transport() kills the process immediately on error; no graceful shutdown attempt
- `crates/cairn-tools/src/plugin_executor.rs:205` — execute_with_transport() ignores JSON-RPC error responses (id present, error field set); treats them as success
- `crates/cairn-tools/src/plugin_executor.rs:232` — parse_notification() fabricates empty string for missing invocation_id; callers cannot correlate notifications
- `crates/cairn-tools/src/plugin_executor.rs:245` — percent progress is parsed as any u64 and cast to f64; values over 100 are accepted without clamping
- `crates/cairn-tools/src/plugin_bridge.rs:22` — every RPC builder uses uuid::Uuid::new_v4().to_string() for request id; uuid crate may not be in scope
- `crates/cairn-tools/src/plugin_bridge.rs:50` — build_tools_invoke_request() hardcodes execution_class as "supervised_process"; ignores actual execution class from config
- `crates/cairn-tools/src/plugin_bridge.rs:62` — invoke_result_to_outcome() collapses every non-Success status to PermanentFailure; loses RetryableFailure and Timeout distinctions
- `crates/cairn-tools/src/plugin_bridge.rs:129` — build_hooks_post_turn_request() hardcodes empty params; no turn summary or context passed to hook
- `crates/cairn-tools/src/plugin_bridge.rs:150` — build_policy_evaluate_request() hardcodes empty params; no policy context or subject passed
- `crates/cairn-tools/src/invocation.rs:73` — mark_started() panics via expect() on an already-started invocation
- `crates/cairn-tools/src/invocation.rs:85` — mark_finished() panics via expect() on an already-finished invocation
- `crates/cairn-tools/src/pipeline.rs:88` — run_builtin_pipeline() synthesizes started_at_ms from now; if permission check takes time, the recorded start time is wrong
- `crates/cairn-tools/src/pipeline.rs:143` — build_plugin_pipeline_request() derives grants from declared permissions directly; no policy evaluation step
- `crates/cairn-tools/src/execution_class.rs:51` — select_execution_config() overrides explicit SupervisedProcess with Sandboxed in team mode; may break legitimate supervised plugins
- `crates/cairn-tools/src/runtime_service.rs:18` — RuntimeToolRequest redundantly carries both plugin_id and target (which already contains plugin_id); can be inconsistent
- `crates/cairn-tools/src/runtime_service_impl.rs:54` — invoke() routes plugin targets only if plugin_id is present on the request; plugin invocations without explicit plugin_id fall through to builtin path silently
- `crates/cairn-tools/src/runtime_service_impl.rs:117` — permission-denied and held-for-approval outcomes not emitted as events; approval flow is invisible to the event log
- `crates/cairn-tools/src/runtime_service_impl.rs:160` — HeldForApproval is surfaced as a successful outcome to the caller; callers cannot distinguish approval-pending from actual success

---

### cairn-plugin-proto (worker-3)

- `crates/cairn-plugin-proto/src/manifest.rs:8` — PluginManifestWire has no execution-class field; host cannot know at discovery time whether a plugin requires sandboxing
- `crates/cairn-plugin-proto/src/manifest.rs:24` — CapabilityWire uses a free-form type string; no validation against the CapabilityFamily enum
- `crates/cairn-plugin-proto/src/manifest.rs:39` — LimitsWire allows maxConcurrency: 0; should be at least 1
- `crates/cairn-plugin-proto/src/wire.rs:14` — JsonRpcResponse only models success (result field); error responses carry a separate JsonRpcError type but recv() never routes to it
- `crates/cairn-plugin-proto/src/wire.rs:24` — JsonRpcError.id is a required String; JSON-RPC spec allows null id for parse errors, so valid error frames are rejected
- `crates/cairn-plugin-proto/src/wire.rs:107` — InitializeResult uses raw serde_json::Value for capabilities; no typed validation of capability entries
- `crates/cairn-plugin-proto/src/wire.rs:143` — ToolsInvokeResult, SignalsPollResult, ChannelsDeliverResult, HooksPostTurnResult, PolicyEvaluateResult all use raw serde_json::Value payload; no typed contract
- `crates/cairn-plugin-proto/src/wire.rs:372` — PluginNotification::from_raw() returns None for unknown method names silently; new notification types added by plugins are dropped

## Round 3 — cairn-api, cairn-app, integration tests, compiler warnings

### cairn-api (worker-1)

- `crates/cairn-api/src/admin.rs:9` — AdminEndpoints returns raw serde_json::Value; no typed response structs
- `crates/cairn-api/src/admin.rs:10` — list_workspaces(tenant_id) conflicts with RFC 008 workspace-scoped access model
- `crates/cairn-api/src/admin.rs:14` — list_projects has no corresponding preserved route in the compat catalog
- `crates/cairn-api/src/assistant.rs:77` — get_session_messages lacks project scope; messages across all projects could be returned
- `crates/cairn-api/src/bootstrap.rs:62` — BootstrapConfig has one global listen_addr/port; no per-role binding as RFC 011 requires for multi-role deployments
- `crates/cairn-api/src/bootstrap.rs:62` — BootstrapConfig has no fields for RFC 011's TLS, auth, or rate-limiting configuration
- `crates/cairn-api/src/endpoints.rs:17` — shared ListQuery only models limit/offset/status; missing project scope, date range, cursor-based pagination
- `crates/cairn-api/src/endpoints.rs:37` — list_tasks returns ListResponse<TaskRecord>; TaskRecord exposes internal store fields not suitable for API consumers
- `crates/cairn-api/src/endpoints.rs:48` — list_approvals returns ApprovalRecord directly; leaks internal store fields
- `crates/cairn-api/src/endpoints.rs:55` — list_sessions returns SessionRecord instead of a product-shaped session summary
- `crates/cairn-api/src/endpoints.rs:45` — get_task, get_session, list_runs_by_session have no project scope parameter
- `crates/cairn-api/src/evals_api.rs:20` — get_scorecard takes ProjectId instead of PromptAssetId; RFC 004 scorecards are grouped by prompt asset
- `crates/cairn-api/src/evals_api.rs:28` — list_eval_runs and get_eval_run have no project scope
- `crates/cairn-api/src/evals_api.rs:50` — EvalRunSummary::from_eval_run turns None dataset_source into empty string silently
- `crates/cairn-api/src/external_workers.rs:28` — WorkerReportRequest.percent is API-local f64; not validated 0.0–100.0
- `crates/cairn-api/src/feed.rs:71` — mark_read has no project parameter; marks items across all projects
- `crates/cairn-api/src/feed.rs:74` — read_all returns bare u32 count instead of the preserved API's ListResponse shape
- `crates/cairn-api/src/graph_api.rs:13` — GraphEndpoints exposes raw string node IDs with no typed wrapper
- `crates/cairn-api/src/http.rs:165` — preserved route catalog includes /v1/skills but no SkillEndpoints trait exists
- `crates/cairn-api/src/http.rs:205` — preserved route catalog includes /v1/poll/run but no polling endpoint is implemented
- `crates/cairn-api/src/http.rs:215` — RFC 010 operator-route block omits preserved route for prompt version history
- `crates/cairn-api/src/http.rs:261` — route catalog preserves /v1/admin/workspaces with no tenant scope parameter
- `crates/cairn-api/src/memory_api.rs:67` — list reuses generic ListQuery; preserved API expects source-type filter parameter
- `crates/cairn-api/src/memory_api.rs:74` — search returns Vec<MemoryItem> instead of the preserved RetrievalResponse with diagnostics

---

### cairn-app (worker-2)

- `crates/cairn-app/src/main.rs:20` — AppBootstrap::start is a hard-blocking stub; no actual server startup implemented
- `crates/cairn-app/src/main.rs:37` — invalid --mode values silently fall back to Local mode instead of erroring
- `crates/cairn-app/src/main.rs:46` — invalid --port values are silently ignored; port defaults to 8080 with no error message
- `crates/cairn-app/src/main.rs:61` — --db accepts SQLite paths even in self-hosted team mode; RFC 011 says SQLite is not supported for team mode
- `crates/cairn-app/src/main.rs:85` — switching to SelfHostedTeam only changes the bind address; storage backend and encryption key remain unchanged
- `crates/cairn-app/src/main.rs:110` — startup sequence performs no preflight checks (database connectivity, migration status, encryption key availability)
- `crates/cairn-app/src/sse_hooks.rs:15` — SseMemoryProposalHook is not constructed anywhere; dead code
- `crates/cairn-app/src/sse_hooks.rs:29` — collected_frames() unwraps the mutex lock; panic on poison
- `crates/cairn-app/src/sse_hooks.rs:36` — on_proposed() also unwraps the mutex lock; same panic risk
- `crates/cairn-app/tests` — crates/cairn-app/tests/ does not exist; main application binary has zero integration tests

---

### Compiler warnings across workspace (worker-2)

- `crates/cairn-domain/src/policy.rs:102` — unused import: ApprovalMode
- `crates/cairn-domain/src/tenancy.rs:163` — unused import: WorkspaceRole
- `crates/cairn-app/src/sse_hooks.rs:15` — unused struct SseMemoryProposalHook
- `crates/cairn-app/src/sse_hooks.rs:22` — unused field frames on SseMemoryProposalHook
- `crates/cairn-runtime/src/services/route_resolver_impl.rs` — unused imports: RouteAttemptDecision, RouteAttemptRecord, RouteDecisionReason

---

### Integration test gaps (worker-3)

- `crates/cairn-api/tests/compat_catalog_sync.rs:62` — only compares static strings; does not verify actual HTTP routing or handler wiring
- `crates/cairn-api/tests/evals_provenance_wiring.rs:29` — despite "wiring" name, only validates types compile; no actual graph projection verified
- `crates/cairn-api/tests/feed_wiring.rs:32` — instantiates FeedStore in isolation; never tests project-scoped feed isolation
- `crates/cairn-api/tests/http_boundary_alignment.rs:45` — verifies fixture JSON shapes but not live HTTP responses; routing bugs invisible
- `crates/cairn-api/tests/memory_wiring.rs:119` — assert!(results.is_empty() || !results.is_empty()) is a tautology; test always passes
- `crates/cairn-api/tests/migration_report_consistency.rs:31` — only checks that migration report compiles; never executes a migration
- `crates/cairn-api/tests/product_surface_composition.rs:41` — uses type_name::<T>() checks; renaming a type silently breaks the test
- `crates/cairn-api/tests/sse_payload_alignment.rs:63` — validates payload shape but never sends a real SSE event; publisher integration not tested
- `crates/cairn-evals/tests/api_contract_guard.rs:13` — only checks that types compile; no behavioral assertions
- `crates/cairn-memory/tests/bundle_roundtrip.rs:98` — silently defaults malformed bundle entries; serialization errors not surfaced
- `crates/cairn-memory/tests/operator_provenance_read.rs:47` — uses a fake GraphQueryService that always returns empty; provenance logic never actually tested
- `crates/cairn-memory/tests/provenance_integration.rs:55` — uses InMemory store only; does not test Postgres retrieval path
- `crates/cairn-memory/tests/retrieval_reranking_integration.rs:101` — MMR test verifies results are non-empty but never verifies actual diversity reordering
- `crates/cairn-memory/tests/signal_feed_integration.rs:44` — reuses the same signal ID across test calls; dedup behavior never tested
- `crates/cairn-runtime/tests/recovery_integration.rs:106` — checkpoint recovery test does not verify CheckpointRestored event was emitted
- `crates/cairn-runtime/tests/sqlite_integration.rs:283` — only tests that tool invocation service can be constructed; never invokes a tool
- `crates/cairn-runtime/tests/week3_integration.rs:225` — manually wires approval; never uses the runtime approval service
- `crates/cairn-runtime/tests/week4_e2e.rs:122` — replay test never asserts on replayed event content; only checks count
- `crates/cairn-store/tests/cross_backend_parity.rs:43` — pause_reason, resume_trigger, prompt_release_id hardcoded None in all fixtures; new fields never tested for parity
- `crates/cairn-store/tests/cross_backend_parity.rs:694` — checkpoint parity test never passes a data payload; checkpoint data field parity untested
- `crates/cairn-store/tests/cross_backend_parity.rs:810` — only tests Latest disposition; Superseded transition parity never verified
- `crates/cairn-store/tests/cross_backend_parity.rs:993` — rebuild-parity assertions only compare entity counts, not field values

---

## Summary

**Total findings: ~220 across all crates**

### By category

| Category | Count | Examples |
|----------|-------|---------|
| Panics in production | ~25 | Mutex unwrap, SystemTime unwrap, expect() on state transitions |
| Data integrity | ~30 | Silent overwrites, no duplicate guards, ToolInvocationFailed→Completed in SQLite |
| RFC compliance gaps | ~40 | Missing event fields, wrong ownership scopes, prompt release deactivation wrong state |
| Missing implementations | ~35 | PgAdapter not updated, AppBootstrap::start is a stub, no cairn-app tests |
| Logic bugs | ~20 | BFS off-by-one, has_more tautology, MMR lambda unclamped, ApprovalResolved no-op |
| Test quality | ~25 | Tautological assertions, fake services, no field-level parity |
| Type safety | ~20 | Raw Strings where enums exist, unvalidated IDs, scores > 1.0 |
| Missing error variants | ~15 | StoreError missing projection variants, notifications dropped silently |

### Priority order for fixes
1. **Panics** — crash the process; fix immediately
2. **Data integrity** — silent data corruption; fix before any production use
3. **RFC compliance** — required fields missing; fix before any release claim
4. **Logic bugs** — incorrect behavior; fix before feature is considered done
5. **Missing implementations** — functionality stubs; implement
6. **Type safety** — validation gaps; tighten
7. **Test quality** — gaps in coverage; address alongside fixes
