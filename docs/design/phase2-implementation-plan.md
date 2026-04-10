# Phase 2 Implementation Plan

Status: COMPLETE — all 8 RFCs implemented
Source: 8 sealed RFCs (015-022), 78 review findings resolved
Executed by: 3 parallel agent workstreams (worker-1, worker-2, worker-3)

## Dependency DAG

```
                    ┌─── Phase 0: Shared Foundation ───┐
                    │                                   │
                    │  cairn-domain types                │
                    │  (VisibilityContext,               │
                    │   RepoAccessContext, From impl,    │
                    │   DecisionScopeRef, ResourceDim,   │
                    │   OnExhaustion, CheckpointKind,    │
                    │   RetrySafety, TriggerFire)        │
                    │                                   │
                    │  Tool registry wiring (~30 tools)  │
                    │  (RFC 018 blocking pre-req)        │
                    │                                   │
                    └──────────┬────────────────────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │                    │                    │
    Workstream A         Workstream B         Workstream C
    (Plugin+Signal)      (Sandbox+Recovery)   (Decision+Protocol)
          │                    │                    │
     RFC 015 ────┐        RFC 016 ────┐        RFC 019
     (marketplace)│       (workspace) │        (decision layer)
          │       │            │      │             │
     RFC 017      │       RFC 020     │        RFC 018
     (github)     │       (recovery)  │        (agent loop)
          │       │            │      │             │
     RFC 022 ─────┘            │      │        RFC 021
     (triggers)                │      │        (protocols)
          │                    │      │             │
          └────────────────────┴──────┴─────────────┘
                               │
                          Integration
                          (cross-workstream tests)
```

## Phase 0: Shared Foundation (ALL THREE AGENTS, serial, ~2 days)

Must complete before any workstream begins. One agent executes while others review.

### Task 0.1: cairn-domain types module

Create `crates/cairn-domain/src/contexts.rs`:

```rust
// New types required by sealed RFCs 015, 016, 019, 020
pub struct VisibilityContext { pub project: ProjectKey, pub run_id: Option<RunId>, pub enabled_plugins: HashSet<String>, pub allowlisted_tools: HashMap<String, Option<HashSet<String>>> }
pub struct RepoAccessContext { pub project: ProjectKey }
impl From<&VisibilityContext> for RepoAccessContext { ... }
pub enum DecisionScopeRef { Run { run_id, project }, Project(ProjectKey), Workspace { tenant_id, workspace_id }, Tenant { tenant_id } }
pub enum ResourceDimension { DiskBytes, MemoryBytes, WallClockMs }
pub enum OnExhaustion { Destroy, PauseAwaitOperator, ReportOnly }
pub enum CheckpointKind { Intent, Result }
pub enum RetrySafety { IdempotentSafe, DangerousPause, AuthorResponsible }
pub enum RunMode { Direct, Plan, Execute { plan_run_id: RunId } }
pub enum ToolEffect { Observational, Internal, External }
```

Also add to cairn-domain:
- `TriggerFire` variant to `DecisionKind` enum (if it exists) or declare the enum
- `SignalCaptureOverride { graph_project: Option<bool>, memory_ingest: Option<bool> }`
- Extended `DestroyReason` and `PreservationReason` enums with `ResourceLimitExceeded`, `AwaitingResourceRaise`, `BaseRevisionDrift`, `AllowlistRevoked`

### Task 0.2: Wire ~30 tool handlers into the orchestrate registry

File: `crates/cairn-app/src/lib.rs` (the `build_tool_registry()` function around line 9221)

Currently wired: memory_search, memory_store, web_fetch, shell_exec, notify_operator, tool_search (6 tools)

Wire all handlers from `crates/cairn-tools/src/builtins/`:
- Add ToolEffect classification to each ToolHandler impl
- Add RetrySafety classification to each ToolHandler impl
- Add cache_on_fields to each tool descriptor
- Register in build_tool_registry()

Per RFC 018's sealed enumeration:
- 15 Observational tools
- 8 Internal tools
- 7 External tools

### Task 0.3: ToolContext::buffer_event() mechanism

Per RFC 020 invariant #11 (batched EventLog::append):
- Add `buffer_event()` and `drain_buffered_events()` to ToolContext
- Modify `IngestService::submit()` to support buffered mode
- Modify `dispatch_tool()` to drain buffer + ToolInvocationCompleted in single batch

**Gate**: Phase 0 complete when `cargo test --workspace` passes with new types + wired tools.

---

## Workstream A: Plugin Marketplace + Signals + Triggers (Worker-1)

### A.1 — RFC 015: Plugin Marketplace (~XL, 5-7 days)

New code:
- `PluginDescriptor`, `PluginCategory`, `CredentialSpec` types in cairn-domain
- `MarketplaceService` in cairn-runtime (layer above existing plugin host)
- Marketplace commands/events (PluginListed, PluginInstalled, PluginInstanceReady, PluginEnabledForProject, etc.)
- `VisibilityContext` consumed by prompt builder in cairn-orchestrator
- Per-project `PluginEnablement` with tool_allowlist, signal_allowlist, signal_capture_override
- Signal Knowledge Capture: graph projection (default ON) + memory ingest (opt-in) async off durable event spine
- `POST /v1/plugins/:id/verify` ephemeral credential verification
- Bundled catalog loader (`crates/cairn-plugin-catalog/catalog.toml`)
- HTTP routes: plugin install, credentials, enable/disable per project, catalog listing
- No `POST /connect`, no `PluginConnected` — sealed lifecycle

Dependencies: Phase 0 (VisibilityContext type)
Integration tests: 1-11 from sealed RFC 015

### A.2 — RFC 017: GitHub Reference Plugin (~L, 3-4 days)

**Separate repo** (`avifenesh/cairn-plugin-github`):
- External binary speaking RFC 007 stdio JSON-RPC
- 19 tools (12 read-only, 7 mutating)
- GitHub App credential model (4 credentials: app_id, private_key, webhook_secret tenant-scoped + installation_id project-scoped)
- Webhook intake with tenant-scoped HMAC verification
- Durable webhook dedup via WebhookDeliveryReceived events
- Signal normalization with source_run_id for loop prevention
- memory_ingest hints on SignalSource (issue.opened/labeled/pr.opened)
- `<!-- cairn:run_id=... -->` marker in mutating tools

Dependencies: A.1 (marketplace must exist for install flow)
Integration tests: 1-16 from sealed RFC 017

### A.3 — RFC 022: Triggers (~L, 3-4 days)

New code in cairn-runtime:
- `Trigger` and `RunTemplate` entities + events
- Trigger evaluator (tokio task subscribing to signal router)
- TriggerCondition DSL (Equals/Contains/Exists/Not)
- Variable substitution (`{{path.to.field}}`)
- Durable `TriggerFireLedger` projection (post-routing dedup)
- Durable `TriggerBucketProjection` (rate-limit/budget state)
- Loop prevention via source_run_id → chain depth lookup
- HTTP CRUD: `/v1/projects/:project/triggers` + `/v1/projects/:project/run-templates`
- cairn-graph projection: GraphNode(Trigger) with matched_by/fired edges

Dependencies: A.1 (signal router), A.2 (GitHub signals for testing), RFC 019 from Workstream C (decision layer for TriggerFire)
Integration tests: 1-14 from sealed RFC 022

---

## Workstream B: Sandbox + Recovery (Worker-2)

### B.1 — RFC 016: Sandbox Workspace Primitive (~XL, 5-7 days)

**New crate**: `crates/cairn-workspace/`

New code:
- `RepoCloneCache` (tenant-scoped physical clone layer)
- `ProjectRepoAccessService` (project-scoped access allowlist)
- `RepoStore` facade composing both
- `OverlayProvider` (Linux: OverlayFS mounts)
- `ReflinkProvider` (macOS/Windows: reflink-copy)
- `SandboxService` with state machine (9 states)
- Vendored codex-linux-sandbox pieces (bubblewrap, landlock, seccomp)
- `cairn.registerRepo` built-in tool rewrite (project-scoped, no host path return)
- `SandboxResourceLimitExceeded` + `OnExhaustion` policy enforcement
- Locked-clone immutability invariant + `refresh()` operation
- OverlayProvider drift detection (`SandboxBaseRevisionDrift`)
- Reflink snapshot-semantics validation
- HTTP routes: `/v1/projects/:project/repos` (CRUD with :owner/:repo split)
- Async clone GC sweep with `ActiveSandboxRepoSource` trait injection
- Storage layout with `{tenant_id}` segment
- cairn-graph projection: GraphNode(Sandbox)/GraphNode(RepoBase)

Dependencies: Phase 0 (RepoAccessContext type)
Integration tests: 1-20 from sealed RFC 016

### B.2 — RFC 020: Durable Recovery (~XL, 5-7 days)

Modifications to cairn-runtime + cairn-app:
- Startup dependency graph (parallel-where-independent)
- All projections enumerated in step 2 (core + knowledge + decision + dedup)
- `/health/ready` JSON body with per-branch status
- HTTP listener opens early for health only; non-health 503 until ready
- Dual checkpoint (Intent/Result) per iteration
- `ToolCallId::derive()` with call_index for parallel dispatch
- `ToolCallResultCache` projection
- RetrySafety enforcement in dispatch_tool (IdempotentSafe/DangerousPause/AuthorResponsible)
- `ToolRecoveryPaused` event for DangerousPause on recovery
- `RecoverySummary` event with all counts
- Run recovery matrix including AllowlistRevoked + BaseRevisionDrift preserved paths
- SQLite team-mode refusal
- Batched EventLog::append (invariant #11) integration with ToolContext::buffer_event

Dependencies: B.1 (sandbox recovery), Phase 0 (CheckpointKind, RetrySafety types, buffer_event)
Integration tests: 1-15 from sealed RFC 020

---

## Workstream C: Decision Layer + Agent Loop + Protocols (Worker-3)

### C.1 — RFC 019: Unified Decision Layer (~L, 3-4 days)

New service in cairn-runtime:
- `DecisionService` composing GuardrailService + ApprovalService + BudgetService + VisibilityContext
- `DecisionRequest` / `DecisionKey` / `DecisionScopeRef` types
- Decision cache with singleflight (Miss/Pending/Resolved)
- `pending_timeout_ms` for stale Pending recovery
- `cache_on_fields` allowlist for semantic key derivation
- Policy-rule reference index for selective invalidation
- `DecisionRecorded` as single canonical event (both Allowed + Denied)
- `TriggerFire` decision kind
- HTTP routes: `/v1/decisions` (list, cache, drill-in, invalidate, bulk invalidate, rule-based invalidate)
- `DecisionCacheProjection` rebuilt on startup

Dependencies: Phase 0 (DecisionScopeRef type)
Integration tests: 1-12 from sealed RFC 019

### C.2 — RFC 018: Agent Loop Enhancements (~L, 3-4 days)

Modifications to cairn-orchestrator:
- `RunMode` (Direct/Plan/Execute) metadata on run creation
- `ToolEffect` classification consumed by prompt builder
- Plan mode: only Observational + Internal tools in prompt
- Plan artifact (`<proposed_plan>` block)
- Plan review: approve/reject/revise HTTP routes + `PlanRevisionRequested` event
- `GuardianResolver` integration (spawn sub-run for approval decisions)
- Inline context compaction with `ContextCompacted` event
- `VisibilityContext` consumed at prompt assembly (plugin tools per project)
- `tool_output_token_limit` truncation
- Exploration budget (separate from execute budget, 10% default)

Dependencies: Phase 0 (RunMode, ToolEffect types + wired tools), C.1 (decision layer for guardian)
Integration tests: 1-12 from sealed RFC 018

### C.3 — RFC 021: Control Plane Protocols (~L, 3-4 days)

New code in cairn-app:
- SQ/EQ protocol: `/v1/sqeq/initialize`, `/v1/sqeq/submit`, `/v1/sqeq/events`
- Scope-bound transport sessions (`sqeq_session_id` + `ProjectKey`)
- `correlation_id` threading through submissions + SSE events
- Async error via `submission.error` SSE event
- `include_reasoning` advisory filter based on bearer token permissions
- A2A Agent Card at `GET /.well-known/agent.json`
- A2A task submission at `POST /v1/a2a/tasks`
- OTLP exporter with GenAI semantic conventions (including knowledge-layer events)
- `PluginCapability::GenerationProvider` + dynamic provider registration
- Provider catalog restart rebuild from durable events
- TypeScript type generation via ts-rs

Dependencies: Phase 0, C.1 (decision events for OTLP), B.2 (startup ordering for provider rebuild)
Integration tests: 1-13 from sealed RFC 021

---

## Timeline (estimated)

```
Week 1:  Phase 0 (all agents, serial)
         ├─ Day 1-2: cairn-domain types + tool wiring + buffer_event
         └─ Day 2: verify with cargo test --workspace

Week 2-3: Parallel workstreams begin
         ├─ A: RFC 015 (marketplace)
         ├─ B: RFC 016 (sandbox/workspace)
         └─ C: RFC 019 (decision layer)

Week 3-4: Second wave
         ├─ A: RFC 017 (GitHub plugin, separate repo)
         ├─ B: RFC 020 (durable recovery)
         └─ C: RFC 018 (agent loop)

Week 4-5: Third wave + integration
         ├─ A: RFC 022 (triggers)
         ├─ B: (integration testing, cross-workstream)
         └─ C: RFC 021 (protocols)

Week 5:  Cross-workstream integration testing
         End-to-end dogfood path verification
```

## Cross-Workstream Sync Points

1. **After A.1 + C.1**: triggers (A.3) can begin because both signal routing and decision layer exist
2. **After B.1 + A.1**: the dogfood path can be tested (marketplace + sandbox)
3. **After B.2**: all workstreams depend on recovery being correct before final integration
4. **After A.2 + A.3 + B.1 + C.2**: the full dogfood demo path (issue → trigger → run → sandbox → PR) can run end-to-end

## What Each Agent's Prompt Should Include

### Worker-1 (Plugin + Signals + Triggers)
- Sealed RFCs: 015, 017, 022
- Key crates: cairn-runtime (MarketplaceService), cairn-tools (plugin host extensions), cairn-plugin-catalog, external cairn-plugin-github repo
- Domain: plugin lifecycle, signal routing, signal knowledge capture, trigger evaluation, webhook handling

### Worker-2 (Sandbox + Recovery)
- Sealed RFCs: 016, 020
- Key crates: cairn-workspace (NEW), cairn-runtime (RecoveryService, SandboxService), cairn-app (startup ordering, health endpoints)
- Domain: filesystem isolation, OverlayFS/reflink, crash recovery, checkpoint/resume, idempotency

### Worker-3 (Decision + Agent Loop + Protocols)
- Sealed RFCs: 019, 018, 021
- Key crates: cairn-runtime (DecisionService), cairn-orchestrator (RunMode, Guardian, compaction, visibility), cairn-app (SQ/EQ routes, OTLP, A2A)
- Domain: policy composition, approval resolution, agent loop modes, protocol design, observability
