# RFC 020: Durable Recovery and Tool-Call Idempotency

Status: draft (rev 3 — recovery ownership split post-`5fefc76`, 15 gap resolutions, open-question resolutions, invariant #12)
Owner: runtime/durability lead
Depends on: [RFC 002](./002-runtime-event-model.md), [RFC 005](./005-task-session-checkpoint-lifecycle.md), [RFC 011](./011-deployment-shape.md), [RFC 016](./016-sandbox-workspace-primitive.md), [RFC 019](./019-unified-decision-layer.md)

## Resolved Decisions (rev 3)

- **Recovery ownership split.** Post commit `5fefc76` ("cairn-fabric finalization: wire adapter, delete obsolete recovery") the deleted `RecoveryServiceImpl` is rebuilt under a narrower scope: cairn-app owns run-level state recovery (runs, checkpoints, tool-invocation lifecycle, cache, approvals, sandbox↔run binding); FF's 14 background scanners own operational state (leases, timeouts, dependencies, budget/quota). Both contribute to "cairn recovery" as seen by operators, but they run independently across the two process boundaries. See §"Recovery ownership split (rev 3)" below for the ownership matrix.
- **Resume semantics per checkpoint kind.** Explicit table for what the orchestrator does on resume depending on whether the latest checkpoint is Intent, Result, or absent. See §"Resume semantics (rev 3)".
- **Invariant #12 added: storage-transparent durability.** Cairn's durability guarantees are defined against its own DB (Postgres canonical / SQLite dev-test-edge), independent of the engine's backing store. If the engine loses its state, FF's scanners rebuild it; cairn's RecoveryService rebuilds cairn state from the event log. No invariant above requires the engine's storage to survive.
- **Open-question resolutions.** Q1 checkpoint body — full snapshot for v1, diff optimization deferred. Q3 tool normalization — mandatory `normalize_for_cache` trait method with JSON-sorted-keys default + per-tool override + property-test harness. Q5 recovery timeout — wait indefinitely; liveness stays 200, readiness stays 503 with progress. Q7 Postgres unreachable at startup — refuse to start (no degraded mode). Multi-instance (Gap 2) deferred entirely to a future multi-node RFC; v1 is single-instance, no locking code.
- **Storage portability constraint.** Postgres is the v1 production target and SQLite is supported for dev/tests/edge, but the service layer must not lock in Postgres-specific features (no `pg_advisory_lock`, `JSONB` operators, `LISTEN/NOTIFY`, array columns, `tsvector`, etc.). Per-backend DDL in migration directories is the escape hatch; runtime queries stay portable.

## Resolved Decisions (rev 2)

- **Checkpoint timing**: **two checkpoints per orchestrator iteration** — an `intent` checkpoint after the decide phase (before any tool dispatch) and a `result` checkpoint after the execute phase (after tool calls complete). Max durability, max recovery granularity. Body size is mitigated by diff-based serialization (each checkpoint can be a diff against the prior intent checkpoint).
- **Tool retry safety classification**: every tool declares `RetrySafety` in its descriptor with three variants: `IdempotentSafe` (orchestrator retries silently), `DangerousPause` (recovered run pauses in `WaitingApproval` and asks the operator whether the side effect occurred), `AuthorResponsible` (orchestrator retries; tool author handles deduplication via external idempotency keys, upserts, etc.).
- **Startup order**: **parallel where independent, serial where dependent**. The event log replay must complete first (everything depends on it). Then `RepoStore.ensure_all_cloned()`, `SandboxService.recover_all()`, `RecoveryService.recover_all()`, plugin host reconnection, provider pool warmup, and decision cache warmup run with maximum parallelism subject to their declared dependencies. Total cold start = max(parallel branches), not sum.
- **Health endpoints**: **liveness/readiness split**, modeled after Postgres / etcd / Temporal. `GET /health` (liveness) returns 200 once cairn-app is running, even mid-recovery. `GET /health/ready` (readiness) returns 503 with a JSON progress body during recovery, 200 once recovery completes. Real API endpoints return 503 until readiness flips. Wait indefinitely; log progress every 5 seconds; hard fail only on unrecoverable corruption.

## Summary

Cairn-rs already has most of a durable runtime: an append-only event log (RFC 002), checkpoint events on runs and tasks (RFC 005), a `RecoveryService` (`crates/cairn-runtime/src/services/recovery_impl.rs`), and lease/heartbeat semantics. What it does not yet have is a **single coherent story** that says what happens when cairn-app crashes mid-run, confirms that everything an in-flight agent produced is recoverable, and guarantees that tool calls are not double-executed when a run resumes.

This RFC tightens existing machinery into a production-grade durability guarantee. It does three things:

1. **Declares Postgres as the canonical production store.** SQLite is for development, tests, and edge / local-mode deployments. Self-hosted team mode requires Postgres with WAL enabled, matching RFC 011's existing decision.
2. **Specifies the recovery contract** for crashes mid-run: what gets replayed, what gets re-issued, what gets abandoned, and what gets checkpointed so the next boot does not lose work.
3. **Adds tool-call idempotency** via deterministic `ToolCallId`s and a result cache so resumed runs do not double-execute side effects.

Much of this is already built. This RFC is less "design a new system" and more "write down the guarantees, audit where they are not met, and close the gaps."

## Why

### The durability question today is fuzzy

If you ask "what happens when cairn-app crashes during a 30-minute agent run?", the answer is today:

- the event log up to the last appended event survives (via `EventLog::append` flushing)
- the run's state at that point can be reconstructed from the log (via `SyncProjection`)
- but nobody has verified that the `RecoveryService` actually restores in-flight runs on startup, as opposed to just emptying stale leases
- checkpoints exist as a primitive (RFC 005) but are not consistently written at each major step of an agent loop
- tool calls that were in-flight at crash time have no idempotency key, so a resumed run re-issues them — an agent that was mid-PR-merge when the crash happened could try to merge the same PR twice, or open two identical issues

These are real gaps. The fix is to pin down the invariants and close the gaps without changing the event model.

### Why Postgres, not SQLite, for production

RFC 011 already declares this. This RFC restates it because the durability story depends on it:

- Postgres has mature WAL semantics, replication, and point-in-time recovery
- Postgres handles concurrent writers from multiple cairn-app processes (horizontal scaling story that SQLite cannot support)
- Postgres's transaction guarantees around the event log append-and-project pattern are well-understood and well-tested at scale
- SQLite with WAL is fine for local-mode development and can be pushed to edge deployments with one cairn-app instance, but it is not the production target for teams

The user explicitly confirmed this: "if we go with durability, postgres. sqlite for testing maybe, or edge testing. sqlite is not good enough."

### Why tool-call idempotency matters

Every tool call that has a side effect (posts a comment, runs a subprocess, creates a resource) is at risk of being re-executed on resume. Without an idempotency key, resume logic has two bad choices: replay the tool (risking duplicates) or skip every tool after the crash point (risking loss). Neither is right. The right answer is: each tool call has a deterministic ID derived from its position in the run, and the tool result cache records completed calls. On resume, the orchestrator checks the cache before re-issuing any tool call.

## Scope

### In scope for v1

- Explicit durability contract: a numbered list of invariants cairn-rs guarantees for runs under the canonical deployment modes
- Audit of existing `RecoveryService`: does it actually recover in-flight runs, not just stale leases? What's broken or missing?
- New `ToolCallId` derivation rule (deterministic from `run_id + step_number + call_index + tool_name + normalized_args`; `call_index` is a per-step monotonic counter assigned by the orchestrator at dispatch time to distinguish parallel identical calls)
- New tool result cache (`ToolCallResultCache` as a projection over `ToolInvocationCompleted` events), consulted before dispatch in the execute phase
- Alignment with RFC 005's recovery rules: anything contradictory gets called out explicitly with a resolution
- Alignment with RFC 016's sandbox recovery: the sandbox recovery scan and the runtime recovery scan coordinate on startup
- Alignment with RFC 019's decision cache: decisions persist across restarts
- A `RecoverySummary` event emitted once per cairn-app boot with counts: recovered runs, recovered tasks, recovered sandboxes, orphans cleaned, decisions restored
- Documented startup order so operators can reason about it

### Explicitly out of scope for v1

- Multi-node cairn-app deployments (RFC 011 defers this)
- Online backup / PITR (operator uses Postgres's existing tooling)
- Exactly-once delivery of outbound side effects to external systems (the best we can do is idempotency keys plus retry caching; external systems vary in whether they honor them)
- Transactional guarantees across multi-tool call sequences (at-most-once per tool call; at-least-once per run is the best we can promise)
- Cross-tenant recovery coordination
- Disaster recovery for lost Postgres instances (operator's backup responsibility)

## The Durability Contract

Cairn-rs guarantees the following for runs executed in team mode (Postgres):

1. **Event log append is the commit boundary.** A run's state at any moment is exactly the state derivable from the event log. Nothing that is not in the log is part of the state.

2. **`EventLog::append` returns only after the events are durable in Postgres WAL.** This is already the case; this RFC makes it a documented invariant, verified by integration tests.

3. **On cairn-app startup, every run in a non-terminal state is recovered before the readiness endpoint flips to 200.** The HTTP listener opens early for health endpoints only (`/health` liveness, `/health/ready` readiness with progress JSON). All non-health routes return 503 until the full startup dependency graph completes (see §"Startup order").

4. **Recovery is idempotent.** Running `recover_all()` twice on the same event log produces the same final projection state. Recovery actions (lease expirations, run state transitions, workspace reconciliation) emit events; replaying those events produces the same outcome.

5. **Two checkpoints are written per orchestrator iteration.** An **Intent checkpoint** (`kind: Intent`) after the decide phase and before tool dispatch, and a **Result checkpoint** (`kind: Result`) after the execute phase once all tool calls complete. A crash during execute rolls back to the Intent checkpoint; a crash during the next gather rolls back to the Result checkpoint. A resumed run starts from the latest checkpoint, not from the run's original start.

6. **Tool-call results are cached.** Every completed tool call produces a `ToolInvocationCompleted` event that includes a `tool_call_id`. On resume, if the cache contains a result for a given `tool_call_id`, the orchestrator returns the cached result instead of re-invoking the tool.

7. **Sandbox state is reconciled with run state.** On startup, `SandboxService::recover_all()` (RFC 016) runs first (step 4a), then `RecoveryService::recover_all()` (step 4b) — sandbox state must be known before runs resume. A run whose state is `Running` but whose sandbox is missing either re-provisions the sandbox (if the base revision is still available) or transitions the run to `failed` with `reason: sandbox_lost`. Sandboxes preserved due to `AllowlistRevoked` or `BaseRevisionDrift` (sealed RFC 016) cause their runs to transition to `WaitingApproval`.

8. **In-flight approval requests survive.** A run in `WaitingApproval` on crash is still in `WaitingApproval` after recovery. An operator's approval from before the crash (if it reached the event log) is honored.

9. **Decisions survive.** RFC 019's decision cache is a projection over the event log; cached decisions from before the crash are available after recovery without any re-approval.

10. **No events are dropped.** Events written but not yet projected are replayed into projections on startup. The projection state is eventually consistent with the log. The log is authoritative.

Any deviation from these invariants is a bug. Integration tests at the end of this RFC verify them.

## Recovery Contract

This section specifies what happens to each type of in-flight state on cairn-app startup.

### Startup order (parallel-where-independent dependency graph)

```
cairn-app start
  ↓
1. Load config, init logging, connect to store (Postgres or SQLite)
   Open HTTP listener for health endpoints ONLY (/health returns 200
   immediately as liveness; /health/ready returns 503 with progress JSON).
   All non-health routes return 503 until step 6.
  ↓
2. Replay event log into ALL enumerated projections (single serial pass):
   Core: RunProjection, TaskProjection, ApprovalProjection,
         SessionProjection, MailboxProjection
   Knowledge: MemoryIndexProjection (cairn-memory), GraphProjection
         (cairn-graph::event_projector), EvalScorecardProjection (cairn-evals)
   Decision: DecisionCacheProjection (RFC 019; clears stale Pending
         singleflight entries as unrecoverable), ToolCallResultCacheProjection
   Dedup: WebhookDeliveryDedupSet (sealed RFC 017; rebuilt from
         WebhookDeliveryReceived events)
  ↓
3. Parallel recovery branches (run concurrently where independent):
   Branch A: RepoCloneCache::ensure_all_cloned()
             (sealed RFC 016; iterates distinct (tenant, repo_id) from
              project allowlists via ProjectRepoAccessService::list_all())
   Branch B: Plugin host reconnection
             (sealed RFC 015; re-validates descriptors + re-reads enablement
              from event log; process instances are NOT pre-spawned; they
              lazy-spawn on next use. Exception: SignalSource-declaring
              plugins eager-spawn the tenant-default scope at the first
              EnableForProject replay, so webhook ingress has a listener
              before readiness flips.)
   Branch C: Provider pool warmup
             (existing provider_pool_impl; depends only on step 2)
  -- barrier: wait for A, B, C --
  ↓
4. Sequential recovery (depends on step 3 completion):
   4a. SandboxService::recover_all() (sealed RFC 016)
       Includes: allowlist-revoked check (step 2a in sealed RFC 016 recovery),
       overlay-only base-revision-drift check (step 2b in sealed RFC 016),
       provider-specific reattach / remount, orphan cleanup.
       Depends on: Branch A (clone availability) + step 2 (run/task state)
   4b. RecoveryService::recover_all() (this RFC tightens)
       Includes: run recovery matrix, task lease recovery, checkpoint resume.
       Depends on: 4a (sandbox state known before resuming runs)
  ↓
5. Emit RecoverySummary event
  ↓
6. Flip /health/ready to 200; open all HTTP routes for requests
```

**Why this ordering**: sandboxes must be reconciled (4a) before the runtime tries to resume runs (4b), because the runtime needs to know which sandboxes survived, which were preserved due to drift/revocation, and which are missing. The decision cache, tool-result cache, and projection warmup (all in step 2) must complete before any tool call is dispatched, so cache hits work correctly from the first resumed iteration. Repo clones, plugin host, and providers (step 3) are independent of each other and of the recovery services, so they run in parallel. Total cold start = step 2 + max(A, B, C) + 4a + 4b — not the serial sum of all steps.

**`/health/ready` JSON body during recovery** (visible to operators and orchestration systems during startup):

```json
{
  "status": "recovering",
  "step": "4a",
  "branches": {
    "event_log": { "state": "complete", "events_replayed": 15234 },
    "tool_result_cache": { "state": "complete", "entries": 42 },
    "decision_cache": { "state": "complete", "entries": 87, "stale_pending_cleared": 2 },
    "memory": { "state": "complete", "chunks_indexed": 3401 },
    "graph": { "state": "complete", "nodes": 892, "edges": 2104 },
    "evals": { "state": "complete", "scorecards": 14 },
    "repo_store": { "state": "complete", "cloned": 3 },
    "plugin_host": { "state": "complete", "descriptors_validated": 1, "eager_spawns": 1 },
    "providers": { "state": "complete", "warmed": 2 },
    "sandboxes": { "state": "in_progress", "recovered": 4, "preserved": 1, "orphaned": 0 },
    "webhook_dedup": { "state": "complete", "entries": 156 },
    "runs": { "state": "pending" }
  },
  "started_at": 1775759896876,
  "elapsed_ms": 2340
}
```

Readiness is 200 only once ALL branches report `complete`.

### Run recovery matrix

For each run in a non-terminal state at startup, `RecoveryService` applies the following rules (derived from RFC 005's recovery section and tightened):

| Run state at crash | Task state | Sandbox state | Action on recovery |
|---|---|---|---|
| Running | Running (claimed) | Sandbox reattached OK | Resume: emit `RunRecovered`; next orchestrator cycle picks up from latest checkpoint |
| Running | Running | Sandbox missing | Attempt re-provision from base revision; if success, resume; if fail, transition run to `failed` with `reason: sandbox_lost` |
| Running | Running | Sandbox preserved: `AllowlistRevoked` | Sealed RFC 016: project's allowlist no longer includes the repo this sandbox was provisioned against. Sandbox is `Preserved { reason: AllowlistRevoked }`. Run transitions to `WaitingApproval` with synthesized approval asking the operator to re-grant or cancel. Previously-authorized work is NOT retroactively invalidated — the sandbox contents survive for inspection. |
| Running | Running | Sandbox preserved: `BaseRevisionDrift` (overlay only) | Sealed RFC 016: the locked clone's HEAD moved since provisioning (via `RepoCloneCache::refresh()`). Overlay sandbox's upper layer is applied over a moved base. Sandbox is `Preserved { reason: BaseRevisionDrift }`. Run transitions to `WaitingApproval`. Reflink sandboxes are exempt (physically independent). |
| Running | Lease expired | Sandbox reattached | Task lease renewed to a fresh expiry; run resumes |
| WaitingApproval | N/A | N/A | Unchanged; approval is still pending |
| Paused | N/A | N/A | Unchanged |
| WaitingDependency | N/A | N/A | Unchanged; dependency projection is re-evaluated on the next dependency completion event |
| Running but no recent checkpoint | ... | ... | The run's message history is rebuilt from `RunMessageAppended` events up to the latest; no checkpoint shortcut |

### Task recovery matrix

Tasks are leased (RFC 005). On recovery:

- Task with expired lease and no `ToolInvocationCompleted` event after the lease's last heartbeat → requeue (emit `TaskLeaseExpired` + `TaskRequeued`)
- Task with expired lease but with a completed tool invocation after heartbeat → the work was in progress at crash; resume via run-level recovery (do not requeue; the run picks up where it left off)
- Task marked `retryable_failed` → return to `queued` via the existing lease reaper (unchanged)
- Task in terminal state → no action

### Checkpoint recovery rules

RFC 005 specifies checkpoints as immutable per-run records, with at most one marked `latest`. This RFC adds:

- **Two checkpoints per orchestrator iteration** (matching the Resolved Decision at the top of this RFC):
  1. **Intent checkpoint** (`kind: Intent`) — written after the decide phase, before any tool dispatch. Captures the planned tool calls (with their `ToolCallId`s, including the deterministic `call_index` assignments), the current message history, and the assembled context bundle. A crash during the execute phase rolls back to this checkpoint; the tool-call result cache covers any calls that completed before the crash.
  2. **Result checkpoint** (`kind: Result`) — written after the execute phase, once all tool calls for the iteration have completed (or timed out/failed). Captures the completed tool results and the updated message history. A crash during the next iteration's gather phase rolls back to this checkpoint, which already includes the completed results.
- The Intent checkpoint is the **safe rollback point**; the Result checkpoint is the **progress commit point**.
- **Resume point**: a recovered run starts from the latest checkpoint's message history. If the latest checkpoint is an Intent checkpoint (crash during execute), any tool calls in the checkpoint that are in the cache as completed return their cached results; any not in the cache are re-dispatched (subject to `RetrySafety` classification). If the latest checkpoint is a Result checkpoint (crash during gather), the orchestrator proceeds directly to the next gather phase.

**The cost**: checkpoint writes add event log overhead proportional to run length. For most runs this is negligible. For runs with enormous contexts, checkpoint bodies compress well (message history is highly redundant).

## Tool-Call Idempotency

### The problem

On resume, the orchestrator needs to know which tool calls already happened and which did not. Without a stable identifier, every tool call on resume is ambiguous: was it dispatched pre-crash? If yes, did it complete? Did its side effect reach the external system? Re-dispatching a non-idempotent tool is unsafe.

### Deterministic `ToolCallId`

Every tool call gets a deterministic identifier:

```rust
pub struct ToolCallId(String);

impl ToolCallId {
    pub fn derive(
        run_id: &RunId,
        step_number: u32,         // the orchestrator's monotonic step counter
        call_index: u32,          // per-step monotonic counter starting at 0
        tool_name: &str,
        normalized_args: &str,    // the tool's canonical argument normalization
    ) -> Self {
        let input = format!("{run_id}:{step_number}:{call_index}:{tool_name}:{normalized_args}");
        let hash = blake3::hash(input.as_bytes());
        Self(format!("tc_{}", hex::encode(&hash.as_bytes()[..16])))
    }
}
```

The key insight: `ToolCallId` is derived from position in the run, not from wall-clock time or PID. A resumed run computing the same tool call at the same step gets the same `ToolCallId`.

**`call_index`** is assigned by the orchestrator at dispatch time. When an execute phase dispatches multiple tool calls in parallel, each gets a monotonically increasing `call_index` within the step. Two parallel calls to the same tool with the same args get `call_index` 0 and 1 → distinct IDs. The `call_index` assignment order **must be deterministic across original dispatch and recovery replay** — the orchestrator sorts parallel dispatch entries by `(tool_name, normalized_args)` before assigning indices so recovery recomputes the same IDs.

**Normalization is per-tool**, using the `cache_on_fields` allowlist from RFC 019. A tool that mis-normalizes will miss its own cache. Test harness enforces correctness.

### Tool result cache

`ToolCallResultCache` is a projection over `ToolInvocationCompleted` events keyed by `tool_call_id`. On dispatch:

```rust
// In cairn-orchestrator::execute
async fn dispatch_tool(
    &self,
    tool_call_id: ToolCallId,
    tool: &dyn ToolHandler,
    args: &ToolArgs,
    is_recovery: bool,  // true when re-dispatching from a recovered checkpoint
) -> Result<ToolResult, ExecuteError> {
    // 1. Check cache first — covers completed calls from before the crash.
    if let Some(cached) = self.tool_result_cache.get(&tool_call_id).await? {
        emit_event(ToolInvocationCacheHit {
            run_id: ctx.run_id.clone(),
            tool_call_id: tool_call_id.clone(),
            original_completed_at: cached.completed_at,
        }).await?;
        return Ok(cached.result);
    }

    // 2. On recovery with no cache entry — check RetrySafety classification.
    if is_recovery {
        match tool.retry_safety() {
            RetrySafety::IdempotentSafe => {
                // Safe to re-dispatch; the tool has no external side effects
                // or is naturally idempotent. Fall through to fresh dispatch.
            }
            RetrySafety::DangerousPause => {
                // The tool may have partially executed before the crash.
                // Transition the run to WaitingApproval with a synthesized
                // approval request asking the operator whether the side effect
                // already occurred and whether to continue/retry/cancel.
                emit_event(ToolRecoveryPaused {
                    run_id: ctx.run_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool.name().to_owned(),
                    retry_safety: RetrySafety::DangerousPause,
                    paused_at: now_ms(),
                }).await?;
                return Err(ExecuteError::RecoveryPaused {
                    tool_call_id,
                    reason: "DangerousPause tool with no cached result on recovery",
                });
            }
            RetrySafety::AuthorResponsible => {
                // Re-dispatch with the same tool_call_id. The tool author is
                // responsible for using tool_call_id as an external idempotency
                // key (upsert, Idempotency-Key header, etc.). Fall through.
            }
        }
    }

    // 3. Fresh dispatch. ToolContext provides buffer_event() for tools
    //    that emit domain events during invoke() (e.g. IngestEvents from
    //    memory_store, ProjectRepoAllowlistExpanded from cairn.registerRepo).
    let result = tool.invoke(args, ctx).await?;

    // 4. Drain events the tool buffered during invoke(), then append ALL
    //    tool-buffered events + ToolInvocationCompleted in a SINGLE atomic
    //    EventLog::append batch. This is the 11th durability invariant:
    //    either ALL events (tool side-effects + completion marker) are
    //    durable, or NONE are. No partial state where the projection saw
    //    the side-effect but the cache did not.
    let mut batch = ctx.drain_buffered_events();
    batch.push(make_envelope(ToolInvocationCompleted {
        run_id: ctx.run_id.clone(),
        tool_call_id: tool_call_id.clone(),
        tool_name: tool.name().to_owned(),
        args_normalized: args.normalize_for_cache(),
        result: result.clone(),
        completed_at: now_ms(),
    }));
    event_log.append(&batch).await?;

    Ok(result)
}
```

**Durability invariant #11 (batched tool-event append)**: tools that emit events to the event log during `invoke()` (e.g. `IngestEvents` from `memory_store`, `ProjectRepoAllowlistExpanded` from `cairn.registerRepo`) must buffer those events via `ToolContext::buffer_event()` rather than appending them directly. The orchestrator drains the buffer after `tool.invoke()` returns and appends all tool-buffered events + `ToolInvocationCompleted` in a single `EventLog::append` batch. This guarantees that either ALL events (tool side-effects + completion marker) are durable, or NONE are — no partial state where the projection saw the side-effect but the cache did not. This invariant requires:

- `ToolContext` gains `buffer_event()` and `drain_buffered_events()` methods
- `IngestService::submit()` gains a "buffered mode" that writes via `ctx.buffer_event()` instead of directly appending
- `cairn.registerRepo` similarly buffers its `ProjectRepoAllowlistExpanded` event
- Any tool that currently appends events directly during `invoke()` must be audited and converted

### RetrySafety classification

Every tool declares `RetrySafety` in its `ToolHandler` trait (mandatory, no default):

```rust
pub enum RetrySafety {
    /// Safe to retry silently. The tool has no external side effects, or
    /// its side effects are naturally idempotent (reads, queries, pure
    /// computations). On recovery with no cached result, the orchestrator
    /// re-dispatches without asking.
    IdempotentSafe,

    /// Dangerous to retry without human confirmation. The tool has external
    /// side effects that may have partially completed before the crash. On
    /// recovery with no cached result, the run transitions to WaitingApproval
    /// with a synthesized operator question: "This tool may have already
    /// executed. What should we do?" Options: retry, skip, cancel run.
    DangerousPause,

    /// The tool author takes responsibility for external idempotency. The
    /// orchestrator re-dispatches with the same tool_call_id; the tool uses
    /// it as an external idempotency key (HTTP Idempotency-Key header,
    /// upsert-by-document-id, etc.). On recovery with no cached result,
    /// the orchestrator re-dispatches and trusts the tool to dedup.
    AuthorResponsible,
}
```

Per-tool classification (aligned with sealed RFC 018's tool enumeration):

| Classification | Tools |
|---|---|
| **IdempotentSafe** | memory_search, web_fetch, grep_search, file_read, glob_find, json_extract, calculate, graph_query, get_run, get_task, get_approvals, list_runs, search_events, wait_for_task, tool_search, summarize_text, scratch_pad |
| **AuthorResponsible** | memory_store (upsert-by-document_id), update_memory, delete_memory, file_write (sandbox-scoped, filesystem idempotent), create_task (idempotent-if-task-exists), cancel_task |
| **DangerousPause** | shell_exec, http_request, notify_operator, git_operations, resolve_approval, schedule_task, eval_score |

Plugin tools declare `RetrySafety` in their manifest (parallel to `ToolEffect` from RFC 018). The GitHub plugin's tools from sealed RFC 017: read-only tools are `IdempotentSafe`; `github.comment_on_issue` and `github.create_pull_request` are `AuthorResponsible` (GitHub supports idempotency via request dedup); `github.merge_pull_request` is `DangerousPause` (merges are not idempotent and cannot be undone).

### TTL and scope

The tool result cache is scoped to the run. A cache entry exists as long as the run is non-terminal; when the run terminates, the cache entries for that run are retained for audit but no longer participate in dispatch lookups.

Cache entries are never shared across runs. A tool call's cached result applies only to the specific run that dispatched it. This is different from RFC 019's decision cache, which can be scoped at project/workspace/tenant. Tool results are run-local.

### Writing to external idempotency keys

Tools classified `AuthorResponsible` should pass cairn's `tool_call_id` as an external idempotency key when the target API supports it. Example: GitHub accepts an `Idempotency-Key` header; the GitHub plugin can pass `tool_call_id` as that header, so even a duplicate request from cairn is deduped on GitHub's side. `memory_store` uses the document ID for upsert semantics. `file_write` is filesystem-level idempotent within the sandbox.

This is a tool-level concern — the tool author decides whether and how to use the `tool_call_id` as an external key. Cairn does not force it, but the infrastructure provides the ID.

## Postgres as Production Target

### Why the canonical store

Per RFC 011, self-hosted team mode requires Postgres. This RFC restates the consequence: the durability guarantees in this document are defined against Postgres. SQLite can meet most of them but:

- SQLite's WAL is single-writer; concurrent cairn-app instances are not supported
- SQLite's durability on write is fsync-dependent and can be configured loosely
- SQLite cannot support the RFC 011 replication story for production

### SQLite use cases

- **Development**: one operator, one machine, fast iteration, no backup required
- **Tests**: integration tests run with in-memory SQLite for speed
- **Edge deployments**: a single cairn-app instance running in a resource-constrained environment where Postgres is overkill; operator accepts reduced durability and no replication

### Startup behavior per store

- **Postgres**: full recovery as specified above. The HTTP listener opens at step 1 for health endpoints only (`/health` liveness, `/health/ready` readiness with progress JSON); all other routes return 503 until recovery completes at step 6.
- **SQLite with WAL in team mode**: cairn-app **refuses to start** with an explicit error: "Team mode requires Postgres. SQLite is supported only for local/development/edge deployments." No degraded startup, no warning-and-continue — the operator must fix the store configuration.
- **SQLite with WAL in local/edge mode**: same recovery contract as Postgres but single-writer only. Startup logs print a notice that this is not a production configuration.
- **InMemory**: no recovery possible; every restart starts fresh; clearly marked as ephemeral; startup log prints a prominent warning.

## Decision Cache Survival

RFC 019's decision cache is a projection over `DecisionEvent`s. On startup:

1. The event log is replayed (step 2 of startup order)
2. The decision cache projection is rebuilt from the replayed events
3. All decisions that were cached pre-crash and have not expired are available
4. `DecisionCacheWarmup` event is emitted with counts (cached, expired-and-dropped)

Operators see in the audit log that the decision cache survived the crash. Agents resuming their runs will hit the cache correctly and not re-nudge the operator for previously-approved decisions.

## Events

New event variants:

```rust
pub enum RecoveryEvent {
    RunRecovered {
        run_id: RunId,
        recovered_from_checkpoint: Option<CheckpointId>,
        message_history_size: u32,
        recovered_at: u64,
    },
    RunRecoveryFailed {
        run_id: RunId,
        reason: String,       // e.g. "sandbox_lost", "base_revision_unavailable"
        transitioned_to: RunState,  // typically Failed
        failed_at: u64,
    },
    TaskLeaseExpired {
        task_id: TaskId,
        lease_owner: String,
        expired_at: u64,
    },
    TaskRequeued {
        task_id: TaskId,
        requeued_at: u64,
    },
    CheckpointSaved {
        run_id: RunId,
        step_number: u32,
        kind: CheckpointKind,             // Intent | Result
        message_history_size: u32,
        tool_calls_snapshot: Vec<ToolCallId>,  // for Intent: planned calls; for Result: completed calls
        saved_at: u64,
    },
    ToolRecoveryPaused {
        run_id: RunId,
        tool_call_id: ToolCallId,
        tool_name: String,
        retry_safety: RetrySafety,        // always DangerousPause
        paused_at: u64,
    },
    ToolInvocationCacheHit {
        run_id: RunId,
        tool_call_id: ToolCallId,
        original_completed_at: u64,
        hit_at: u64,
    },
    DecisionCacheWarmup {
        cached: u32,
        expired_and_dropped: u32,
        warmed_at: u64,
    },
    RecoverySummary {
        recovered_runs: u32,
        recovered_tasks: u32,
        recovered_sandboxes: u32,
        preserved_sandboxes: u32,       // AllowlistRevoked + BaseRevisionDrift
        orphaned_sandboxes_cleaned: u32,
        decision_cache_entries: u32,
        stale_pending_cleared: u32,     // singleflight Pending entries from RFC 019
        tool_result_cache_entries: u32,
        memory_projection_entries: u32,  // chunks in retrieval index after replay
        graph_nodes_recovered: u32,
        graph_edges_recovered: u32,
        webhook_dedup_entries: u32,      // sealed RFC 017
        boot_id: String,
        startup_ms: u64,
        summary_at: u64,
    },
}
```

## Non-Goals

- multi-node cairn-app deployments
- exactly-once external side effects (cairn provides at-least-once with idempotency hints; external systems decide)
- disaster recovery of lost Postgres instances
- partial checkpoint granularity smaller than a run step
- transactional multi-run atomicity
- cross-region replication
- undo of side effects already reached the external world

## Open Questions

1. **NEEDS DISCUSSION: Checkpoint frequency vs event log bloat.** Writing a full message history snapshot at every step is correct but bloats the event log. Proposal: checkpoint body is a compressed diff against the prior checkpoint, not a full snapshot. Every Nth checkpoint is a full snapshot for faster recovery (trade-off between recovery speed and log size). Confirm.

2. **Resolved**: Two checkpoints per iteration — Intent (after decide, before execute) and Result (after execute). See §"Checkpoint recovery rules" and the `CheckpointKind` field on `CheckpointSaved`. (No further discussion needed; baked into the Resolved Decisions.)

3. **NEEDS DISCUSSION: Tool-call normalization test harness.** Every `ToolHandler` author must declare a normalization function and ship tests that prove semantic equivalence is preserved across serializations. Should this be enforced at the trait level with a mandatory `normalize_for_cache` method? Proposal: yes, mandatory trait method with a default implementation that normalizes JSON (sorted keys, dropped fields named `timestamp`, `request_id`, etc.) and a per-tool override.

4. **Resolved**: Tool recovery behavior is handled by the `RetrySafety` three-tier classification (`IdempotentSafe` / `DangerousPause` / `AuthorResponsible`), NOT by a `retry_on_recovery: bool`. See §"RetrySafety classification". The conflicting `retry_on_recovery` concept is deleted. (No further discussion needed.)

5. **NEEDS DISCUSSION: Recovery timeout.** If recovery takes more than N seconds on startup, should cairn-app open the HTTP server in degraded mode (accepting reads, refusing mutations) or continue waiting? Proposal: continue waiting. Operators should size their store so recovery is fast; if recovery hangs, the operator has bigger problems than HTTP availability.

6. **Resolved**: Startup order is **parallel-where-independent** per the dependency graph in §"Startup order". Event-log replay is serial (everything depends on it); then RepoStore/plugin-host/providers run in parallel; then sandbox recovery; then run recovery. See the dependency graph for the precise barrier structure. (No further discussion needed; matches sealed RFC 016's decision.)

7. **NEEDS DISCUSSION: What happens if Postgres is unreachable on startup?** Proposal: cairn-app refuses to start and emits a clear error. Do not start in degraded / no-store mode by default — it would give operators the illusion of availability while losing all durability.

8. **NEEDS DISCUSSION: Event log compaction.** Over time the log grows. Postgres can handle large logs, but eventually an operator will want to compact. Proposal: event log compaction is a future RFC; v1 ships without it. Operators prune by archiving Postgres tables with standard Postgres tooling.

## Decision

Proceed assuming:

- the durability contract above is the set of invariants cairn-rs guarantees for runs
- Postgres is the canonical production store; SQLite is for development, tests, and edge single-instance deployments
- startup follows the parallel-where-independent dependency graph: event-log replay (serial) → {RepoStore clone, plugin host reconnect, provider warmup} (parallel) → sandbox recovery → run recovery → readiness flip; HTTP listener opens early for health endpoints only
- two checkpoints per iteration: Intent (after decide, before execute) and Result (after execute); `CheckpointSaved` carries `kind: CheckpointKind`
- every tool call has a deterministic `ToolCallId` derived from `(run_id, step_number, call_index, tool_name, normalized_args)` where `call_index` is a per-step monotonic counter for parallel dispatch
- tools declare `RetrySafety` (IdempotentSafe / DangerousPause / AuthorResponsible); recovery behavior branches per classification — no `retry_on_recovery: bool`
- tools that emit domain events during invoke() buffer them via `ToolContext::buffer_event()` and the orchestrator appends all events + ToolInvocationCompleted in a single atomic batch (durability invariant #11)
- a `ToolCallResultCache` projection is consulted before every tool dispatch; cache hits return prior results without re-invoking the tool
- tool call normalization is mandatory and tested per tool
- the decision cache (RFC 019) is replayed from the event log on recovery with no re-approval required
- a single `RecoverySummary` event is emitted per cairn-app boot
- open questions listed above must be resolved before implementation tightens the existing `RecoveryService`

## Integration Tests (Compliance Proof)

1. **Clean recovery**: start cairn-app, create a run, write 20 events; stop cairn-app cleanly; restart; confirm the run is in the same state, projections match, no orphaned resources
2. **Crash-during-execute recovery**: start cairn-app, create a run that calls a slow tool, SIGKILL cairn-app mid-tool-call; restart; confirm the run resumes, the tool is either cached (if the completion event made it to the log) or re-dispatched, no duplicate side effects reported by the tool
3. **Sandbox reattach (overlay or reflink)**: create a run with an overlay sandbox (Linux) or reflink sandbox (macOS), SIGKILL cairn-app; restart; confirm `SandboxService::recover_all()` finds the sandbox, `RecoveryService` reattaches it, the run resumes
3a. **Sandbox preserved: allowlist revoked**: create a run with a `SandboxBase::Repo` sandbox; between crash and restart, revoke the repo from the project's allowlist via `DELETE /v1/projects/:project/repos/:owner/:repo`; restart; confirm `SandboxAllowlistRevoked` emitted and sandbox transitions to `Preserved { reason: AllowlistRevoked }`; run transitions to `WaitingApproval`
3b. **Sandbox preserved: base-revision drift (overlay only)**: create an overlay run, call `RepoCloneCache::refresh()` to move the clone HEAD; SIGKILL cairn-app; restart; confirm `SandboxBaseRevisionDrift` emitted and sandbox transitions to `Preserved { reason: BaseRevisionDrift }`; a reflink sandbox on the same tenant does NOT emit drift (physically independent)
4. **Sandbox lost**: create a run with a sandbox, manually delete the sandbox directory, SIGKILL cairn-app; restart; confirm the run transitions to `failed` with `reason: sandbox_lost`
5. **Task lease expiration during recovery**: create a task with an expired lease at crash time, restart; confirm `TaskLeaseExpired` and `TaskRequeued` events are emitted; the task is available for claim
6. **In-flight approval survives**: create a run in `WaitingApproval`, restart; confirm the approval is still pending; resolve it; confirm the run resumes
7. **Decision cache survives**: cache a decision (RFC 019), restart; confirm the decision is available; a subsequent equivalent request returns via cache hit without re-nudging
8. **Tool call cache hit on resume**: a run executes tool X, `ToolInvocationCompleted` is written, cairn-app crashes, restart; the next orchestrator iteration for the same run returns the cached result for tool X without re-dispatching
9. **Startup order (dependency graph)**: `/health` returns 200 immediately (liveness); `/health/ready` returns 503 with progress JSON during recovery showing per-branch status; all non-health routes return 503. After all branches complete, `/health/ready` flips to 200 and non-health routes open. Test verifies the full readiness JSON body at each stage.
10. **Recovery is idempotent**: run `recover_all()` twice programmatically; confirm the second run produces no new events and no state changes
11. **`RecoverySummary` emitted**: exactly one `RecoverySummary` event is emitted per cairn-app boot with accurate counts
12. **Postgres required for team mode**: attempt to start cairn-app in team mode with a SQLite DB; confirm startup **refuses** with a clear error message (not just a warning)
13a. **DangerousPause recovery**: a run executing `shell_exec` (DangerousPause) crashes with no cached result; on restart, the run transitions to `WaitingApproval` with a synthesized approval asking "did this tool execute?"; `ToolRecoveryPaused` event emitted
13b. **AuthorResponsible recovery**: a run executing `memory_store` (AuthorResponsible) crashes with no cached result; on restart, the tool is re-dispatched with the same `tool_call_id`; the tool's upsert-by-document-id handles the dedup
13c. **Batched append coherence**: kill cairn-app between a `memory_store` invoke and ToolInvocationCompleted; on restart, verify either (a) both the memory chunk AND the cache entry exist (batch was durable) OR (b) neither exists (batch was not durable and the tool re-executes cleanly). NEVER: chunk exists but cache entry does not.
14. **Dual checkpoint**: verify both Intent and Result checkpoints are written per iteration; a crash during execute recovers from the Intent checkpoint; a crash during gather recovers from the Result checkpoint
15. **Checkpoint compression**: a 100-step run has checkpoints whose total body size is substantially less than 100 full snapshots (validates diff-based compression if adopted)

---

# Rev 3 Addendum

This addendum was added in revision 3 (2026-04-21). It preserves the rev 2 body above unchanged and documents decisions + gaps that surfaced while auditing the current implementation state against the draft.

## Why rev 3 exists

Audit of cairn-rs on `origin/main` (commit `ed0acee`) found that most RFC 020 types exist (`ReadinessState`, `ToolCallId`, `ToolCallResultCache`, `RecoveryDispatchDecision`, `CheckpointKind`, `RecoveryEvent` variants, `RecoverySummary`), but the runtime integration is minimal — at audit time, 1 of 15 compliance tests was live (test #9 readiness, shipped as PR #73). Six of the 11 durability invariants were outright broken; three partial.

Separately, commit `5fefc76` deleted `crates/cairn-runtime/src/services/recovery_impl.rs` with the rustdoc "recovery now lives unconditionally in FlowFabric's background scanners." This is correct for FF operational state and incorrect for cairn run-level state. The deletion left a gap this RFC now addresses explicitly.

Four implementation tracks carry rev 3 to compliance. Track 2 (readiness gate) shipped as PR #73; Track 1 (RecoveryService resurrection), Track 3 (tool-call idempotency end-to-end), Track 4 (dual checkpoint + audit events) follow in sequence.

## Recovery ownership split (rev 3)

Rev 2's "Recovery Contract" described a unified `RecoveryService::recover_all()`. Rev 3 refines: cairn's recovery surface is split across two process boundaries. FF process owns operational state and recovers it continuously via background scanners (not just on cairn-app boot). Cairn-app process owns run-level state and recovers it on startup via its own `RecoveryService`.

| Layer | State it owns | Recovery mechanism | Who runs it |
|---|---|---|---|
| **FF operational** | Execution lease liveness, attempt timeouts, execution deadlines, suspension timeouts, pending waitpoint expiry, budget resets, budget reconciliation, quota reconciliation, dependency reconciliation, flow projection, index reconciliation, retention trimming, unblock propagation, delayed promotion | 14 background scanners inside FF (`LeaseExpiryScanner`, `AttemptTimeoutScanner`, `ExecutionDeadlineScanner`, `SuspensionTimeoutScanner`, `PendingWaitpointExpiryScanner`, `BudgetResetScanner`, `BudgetReconciler`, `QuotaReconciler`, `DependencyReconciler`, `FlowProjector`, `IndexReconciler`, `RetentionTrimmer`, `UnblockScanner`, `DelayedPromoter`) | FF process |
| **Cairn run-level** | Run `public_state`, run message history, checkpoint contents, tool invocation lifecycle, tool call result cache, decision cache, approval state, sandbox↔run binding, session↔run binding | `cairn-runtime::RecoveryService` (rebuilt in Track 1 under the narrowed scope) | cairn-app on startup |
| **Bridge (FF→cairn)** | Lease-history stream position per partition, bridge event correlation | `FfLeaseHistoryCursor` projection + `lease_history_subscriber` | cairn-app subscriber |
| **Cairn peripheral** | Sandbox filesystem reconciliation (RFC 016), plugin host reconnection (RFC 015 — lazy-spawn except SignalSource), provider pool warmup, repo clone cache | `SandboxService::recover_all`, `PluginHost::reconnect`, `ProviderPool::warmup`, `RepoCloneCache::ensure_all_cloned` | cairn-app on startup |
| **Infrastructure** | Event log integrity, projection replay, migration state | cairn-store Postgres/SQLite adapters, `ProjectionRebuilder` | cairn-store on startup |

The rebuilt cairn-runtime `RecoveryService` does NOT expire leases, re-time attempts, reconcile dependencies, or reconcile budgets/quotas — those are FF's lane. It DOES enumerate non-terminal runs, apply the §"Run recovery matrix" rules, read latest checkpoint per run, verify sandbox↔run binding, emit `RunRecovered`/`RunRecoveryFailed`, and emit `RecoverySummary` at end.

## Resume semantics (rev 3)

Rev 2's §"Checkpoint recovery rules" is correct but leaves the per-case orchestrator action implicit. Rev 3 tightens:

| Latest checkpoint on resume | Orchestrator action |
|---|---|
| None (crash before first checkpoint) | Rebuild message history from `RunMessageAppended` events. Start next iteration's decide phase from scratch. |
| `Intent` (crash during execute) | Load message history + planned `ToolCallId`s from checkpoint. For each planned call: if cache hit, mark done; if cache miss, apply `recovery_dispatch_decision(is_recovery=true, tool.retry_safety())`. Once all planned calls are resolved, proceed to gather. |
| `Result` (crash during gather or between iterations) | Load message history + completed results from checkpoint. Proceed to next iteration's gather. No tool re-dispatch. |

Track 3 and Track 4 integration tests assert against this table directly.

## Invariant #12 — storage-transparent durability

Cairn's durability guarantees are defined against cairn's own DB (Postgres canonical for team mode; SQLite acceptable for dev/tests/edge single-instance). They are independent of the engine's backing store (FF's choice — Valkey today, potentially others). If Valkey goes down hard and comes back empty, FF's scanners rebuild FF state; cairn's RecoveryService rebuilds cairn state from Postgres; the bridge subscriber resumes from `FfLeaseHistoryCursor`. No previously-stated invariant requires the engine's storage to survive.

This is the consumer-side corollary of FF's "transparent engine over Valkey OR Postgres" direction. Cairn is written as if the engine backend is unknown; it ships with Valkey today but must not assume Valkey is available.

## Storage portability (rev 3)

Postgres is the v1 production target. SQLite is supported for development, tests, and edge single-instance deployments. **The service layer must use only SQL features common to most common SQL databases.** Any Postgres-specific feature requires an explicit design-level decision before use. This prevents shipping a product that only works for Postgres operators.

- Allowed (standard SQL, portable across Postgres/MySQL/SQLite/MSSQL): CRUD, transactions, standard types, `PRIMARY KEY`/`FOREIGN KEY`/`UNIQUE`/`CHECK`, btree indexes, `SELECT ... FOR UPDATE`, JSON stored as `TEXT`.
- Requires explicit decision: `pg_advisory_lock`, `LISTEN`/`NOTIFY`, `JSONB` type and operators (`->>`, `->`, `@>`), array columns, `tsvector`/`tsquery`, `SKIP LOCKED`, `MERGE`, `CREATE EXTENSION`, `WITH RECURSIVE`, generated columns, range types.
- Per-backend DDL in `crates/cairn-store/src/pg/migrations/*.sql` and `crates/cairn-store/src/sqlite/*.sql` is the escape hatch for backend-specific schema choices. Runtime query code must stay portable.

At rev 3 time, a schema-parity check (shipped as PR #76, `#[ignore]`d by default) surfaces known SQLite gaps: ten tables currently exist only on Postgres (`route_policies`, `workspace_members`, `tenants`, `workspaces`, `projects`, `prompt_assets`, `prompt_releases`, `prompt_versions`, `provider_calls`, `route_decisions`). This is acceptable while SQLite is dev-only; becomes a bug if SQLite targets production parity.

## Gap resolutions (rev 3)

Rev 2's §"Open Questions" listed five items marked NEEDS DISCUSSION. Rev 3 resolves four; one remains deferred.

| Open question | Rev 3 resolution |
|---|---|
| Q1 — Checkpoint body size (full vs diff) | Full snapshot per checkpoint for v1. `CheckpointSaved.message_history_size` records serialized length so operators observe cost. Diff optimization reconsidered only if observed cost exceeds a threshold (follow-up Track 4b). |
| Q3 — Tool normalization trait method | Accept proposal. `normalize_for_cache(&self, args: &ToolArgs) -> String` mandatory on `ToolHandler` with a default (JSON keys sorted lexicographically, UTF-8 encoded) and per-tool override. Property test harness at trait level: `normalize(args) == normalize(args)` (idempotent) and parse-round-trip semantic equivalence for a corpus per tool. |
| Q5 — Recovery timeout | Wait indefinitely. Liveness (`/health`) stays 200; readiness (`/health/ready`) stays 503 with progress JSON. No degraded mode. |
| Q7 — Postgres unreachable at startup | Refuse to start with nonzero exit code. systemd/Kubernetes restarts the process; no in-memory fallback. |
| Q8 — Event log compaction | Deferred to a future RFC (rev 2 position unchanged). |

Rev 3 adds the following previously-unenumerated gaps with resolutions:

- **Gap A — Process-boundary event ordering during recovery.** FF scanners and cairn's RecoveryService run independently at startup. A run may appear `Running` to cairn's projection snapshot at the instant FF emits a lease-expiry event that requeues its task. Resolution: `RunRecovered` is advisory. The orchestrator, when it actually picks up the run on next tick, re-reads current state and acts on it. Invariant #8 already permits this ("projections are eventually consistent with the log"). The event name stays `RunRecovered` — documented as advisory in its rustdoc rather than renamed.
- **Gap B — Multi-process cairn-app.** Two instances against the same DB would both try to recover the same runs. Resolution: defer multi-instance entirely to a future multi-node RFC. v1 is single-instance. No locking code in Track 1. This supersedes an earlier draft suggestion to use `pg_advisory_lock`, which would violate the portability rule.
- **Gap C — Tool buffered events ownership.** Invariant #11 requires tool-buffered side-effect events and `ToolInvocationCompleted` to land in one `EventLog::append` call. Today `record_completed` appends `ToolInvocationCompleted` alone, and no caller drains `ctx.buffer_event`. Resolution (Track 3): `record_completed` accepts `additional_events: &[EventEnvelope]`; orchestrator drains buffered events after each `tool.invoke()` and passes them through. Tools currently appending directly during `invoke()` (e.g. `memory_store`, `cairn.registerRepo`) convert to use `ctx.buffer_event()`.
- **Gap D — SIGKILL integration test harness.** Rev 2's 15 compliance tests all require restarting a real cairn-app subprocess against durable state. `LiveHarness` at `crates/cairn-app/tests/support/live_fabric.rs` spawned subprocesses but had no kill/restart. Resolution: shipped in PR #74 — `sigkill()`, `restart()`, `sigkill_and_restart()`, `poll_readiness_until_ready()` helpers, with SQLite-backed variant for DB-survival tests.
- **Gap E — Boot ID.** `RecoverySummary` has a `boot_id` field but rev 2 didn't say how to mint it. Resolution: `BootId(String)` (UUID v7) minted once per process boot, plumbed through `AppState` and every `RecoveryEvent` variant for audit correlation. Visible in readiness progress JSON.
- **Gap F — Runs stuck in `Running` with no progress.** Edge case: a run marked `Running` via `RunStarted` event, then cairn-app crashed before any checkpoint or message append. Resolution: new recovery matrix row — a `Running` run with zero messages and zero checkpoints for more than `RUN_WEDGE_THRESHOLD_MS` (5 min default, tuneable) transitions to `failed` with `reason: crashed_before_first_progress` and emits `RunRecoveryFailed`.
- **Gap G — Approvals submitted during crash.** A run in `WaitingApproval` has the approval resolved in the event log between crash and recovery. Rev 2 invariant #8 ("WaitingApproval unchanged on recovery") needs nuance. Resolution: RecoveryService, for runs in `WaitingApproval`, checks the latest approval-resolution event and processes it if present. This is consistent with invariant #4 (recovery is idempotent) — the resolution already happened in the log.
- **Gap H — In-flight subagent work.** No new work. RFC 005 recovery rules + dependency projection handle this via `WaitingDependency` state. Track 1 integration test verifies it still works.
- **Gap I — Plugin state across recovery.** No new work. RFC 015 specifies descriptors are re-validated + enablement re-read from event log before plugin processes are spawned. Track 3 integration test for an `AuthorResponsible` tool using a plugin verifies.
- **Gap J — Observability during recovery.** Resolution: per-branch tracing spans in the startup graph, per-branch duration metrics, log progress every 5 seconds during recovery. `RecoverySummary` logged at INFO level at end.

## Implementation status at rev 3

- **PR #73** `83abf86` — readiness gate wired (`/health/ready` 503→200 with progress JSON; middleware gates non-health routes). Compliance test #9 live.
- **PR #74** `48e63f6` — `LiveHarness` SIGKILL+restart.
- **PR #76** `688f850` — CI schema-parity check (ignored; concrete SQLite-gap list surfaced).
- **Track 1** — `RecoveryService` resurrection (run-level scope). In flight. Delivers compliance test #1.
- **Track 3** — tool-call idempotency end-to-end (ToolCallId mint, cache consultation, is_recovery plumb-through, batched append, `normalize_for_cache`). Planned. Delivers compliance tests #2, #8, #13a, #13b, #13c.
- **Track 4** — dual checkpoint emission + `RecoverySummary`/`DecisionCacheWarmup`. Planned. Delivers compliance tests #11, #14.

Final target: 14 of 15 compliance tests live (test #15 checkpoint compression skipped by design — rev 3 ships full snapshots, not diffs).

## Non-goals for rev 3

Unchanged from rev 2: multi-node cairn-app deployments, exactly-once external side effects, disaster recovery of lost Postgres, cross-region replication, undo of side effects already reached the external world. Rev 3 adds: multi-instance single-DB correctness (separate future RFC), and event log compaction (deferred). Rev 3 explicitly does NOT target SQLite production parity — that remains a dev-only configuration at v1.
