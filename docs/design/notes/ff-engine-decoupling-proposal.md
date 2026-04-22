# FF Engine decoupling — phase roadmap

Ship plan for moving cairn off direct FlowFabric internals. One trait
(`crates/cairn-fabric/src/engine/mod.rs::Engine`), one impl
(`ValkeyEngine`), and a shrinking list of "not in scope (yet)" items
that shrinks by one phase per PR.

## Phase A — trait skeleton & `ValkeyEngine` (shipped, PR #72)

- `Engine` trait with `describe_execution`, `describe_flow`,
  `describe_edge`, `list_incoming_edges`.
- `ValkeyEngine` as the one impl — holds `ferriskey::Client`, does
  the `HGETALL` / `SMEMBERS` work, parses into typed snapshot structs.
- Every cairn-side read of FF state flipped to go through the trait.

## Phase B — targeted read (`get_execution_tag`) (shipped, PR #72)

- Added alongside Phase A to avoid the N+1 amplification that a
  full `describe_execution` would cause in loops like
  `check_dependencies`'s per-blocker resolve.

## Phase C — tag writes (shipped, this PR)

Goal: cairn services stop calling `ferriskey::Client::hset` on
FF-owned hashes.

Trait surface (3 methods):

| Method | Target hash | Use site |
|--------|------------|----------|
| `set_flow_tag(&FlowId, key, value)` | `fctx.core()` | `FabricSessionService::archive` |
| `set_flow_tags(&FlowId, &BTreeMap)` | `fctx.core()` | `FabricSessionService::create` (bulk — `cairn.project` + `cairn.session_id` in one round-trip) |
| `set_execution_tag(&ExecutionId, key, value)` | `ExecKeyContext::tags()` | (no caller in this phase; provided for Phase D/E symmetry + tested end-to-end) |

Invariants:

- **Namespace guard**: key must match `^[a-z][a-z0-9_]*\.`. Prevents
  collision with FF's own hash fields (which have no `.`). Rejected
  as `FabricError::Validation` at the trait boundary.
- **All-or-nothing bulk**: `set_flow_tags` validates every key before
  issuing the wire call; a partial write is never visible.
- **Empty map is a no-op**: `set_flow_tags` returns `Ok(())` without
  issuing a wire call.

### Scope exception — `instance_tag_backfill`

The one-shot backfill in `crates/cairn-fabric/src/instance_tag_backfill.rs`
keeps its direct `HSET cairn.instance_id` and is NOT routed through
the trait. Reason: it operates on raw `ff:exec:*:tags` scan-key
strings, not typed `ExecutionId`s. A trait method that took a raw
key would re-expose the Valkey key layout we're trying to hide.
Parsing the UUID back out of the scan key just to hand it to a trait
that re-derives the same key is pointless ceremony. The backfill is
a finite-lifetime migration utility gated on
`CAIRN_BACKFILL_INSTANCE_TAG=1` and will be removed once the
pre-filter fleet is fully aged out.

### Why no clippy `disallowed-methods` lint

A workspace-wide `disallowed-methods` entry on `ferriskey::Client::hset`
would flag cairn-owned keyspaces too (`worker_service`,
`quota_service`, `boot::seed_waitpoint_hmac_secret_if_configured`).
Those aren't layering violations — cairn owns those keys end-to-end.
The lint would either produce noise or require 3-4 `#[allow(...)]`
escape hatches, neither of which is a win. The module-docs in
`engine/mod.rs` pledge enforcement via code review. A future tight
lint is tracked as a follow-up in
`~/.claude/projects/-home-ubuntu/memory/project_ff_decoupling_followups.md`.

## Phase D — control-plane FCALLs + FCALL ARGV pre-reads (split)

The dispatch grew once Phase C shipped: control-plane FCALLs (budget,
quota, rotation, worker) plus run/task/session lifecycle FCALLs plus
the ~12 `hget ctx.core()` ARGV pre-reads combined to ~5-8k LOC in a
single PR — unreviewable, and at odds with cairn's "merge before next
PR" rule. Split along the natural fault line:

### Phase D PR 1 — FCALL control plane + worker registry (shipped, this PR)

A new `ControlPlaneBackend` trait absorbs the FCALL-shaped,
self-contained ops: budget create / spend / release / status, quota
create / admission check, waitpoint HMAC rotation. One
`Arc<ValkeyEngine>` implements both `Engine` and
`ControlPlaneBackend`, with the control-plane impl living in
`engine/valkey_control_plane_impl.rs`. Worker registry (register /
heartbeat / mark-dead) folds into the existing `Engine` trait instead
of `ControlPlaneBackend` — the ops are HSET / SADD / PEXPIRE-shaped,
same flavour as Phase C's tag writes, and the fold avoids a spurious
third trait.

Cairn-native mirror types on the trait boundary
(`BudgetSpendOutcome`, `QuotaAdmission`, `BudgetStatusSnapshot`,
`RotationOutcome`, `RotationFailure`, `WorkerRegistration`) replace
the FF wire enums (`ff_core::contracts::ReportUsageResult`, etc.) in
service signatures. Service-level type aliases preserved so
downstream imports keep working.

Scheduler deferred: `ff_core::contracts::ClaimGrant` is a wire type
that flows cairn → `ff-sdk::FlowFabricWorker::claim_from_grant`;
mirroring it cairn-side adds a conversion hop without actually
decoupling anything. Worker-SDK adjacency also places it with the
lifecycle-tangled PR 2 group.

Closes 12 `ff_core::{keys,partition,contracts}` + `ff_sdk::task`
leak lines in:
  - `services/budget_service.rs`
  - `services/quota_service.rs`
  - `services/rotation_service.rs`
  - `services/worker_service.rs`

### Phase D PR 2a — run / session / claim lifecycle (shipped)

Extends `ControlPlaneBackend` with 11 run/session/claim FCALL methods
(`create_run_execution`, `complete_run_execution`, `fail_run_execution`,
`cancel_run_execution`, `suspend_run_execution`, `resume_run_execution`,
`deliver_approval_signal`, `create_flow`, `cancel_flow`,
`issue_grant_and_claim`). Added cairn-native mirrors:
`ExecutionCreated`, `FailExecutionOutcome`, `FlowCancelOutcome`,
`ClaimGrantOutcome`, plus the typed request structs
(`CreateRunExecutionInput`, `SuspendRunInput`, `ExecutionLeaseContext`,
etc.). One concrete `ValkeyEngine` now backs three traits (`Engine`
for reads/tag-writes, `ControlPlaneBackend` for all FCALLs).

Services migrated:
  - `services/run_service.rs` (8 lifecycle methods — start /
    complete / fail / cancel / pause / resume /
    enter_waiting_approval / resolve_approval / claim). Pre-read
    path routes through `engine.describe_execution`, eliminating
    every `hget ctx.core(), "current_attempt_id"` call in this
    service. The `SuspendRunInput.resume_condition_json` +
    `resume_policy_json` JSON assembly stays service-side — only
    the FCALL dispatch moves to the backend.
  - `services/session_service.rs` (`create` + `archive`).
  - `services/claim_common.rs` (now a ~50-line shim over
    `ControlPlaneBackend::issue_grant_and_claim`; the dispatch to
    `ff_claim_resumed_execution` for attempt-interrupted executions
    lives in the backend impl where it belongs).

Audit: `git grep -nE '^use ff_core::(keys|partition)::' crates/cairn-fabric/src/services/{run_service,session_service,claim_common}.rs`
returns zero hits.

### Phase D PR 2b — task lifecycle (deferred)

`services/task_service.rs` (11 lifecycle methods) still imports
`ff_core::keys::{ExecKeyContext, IndexKeys}` + `ff_core::partition::
execution_partition`. The service hasn't migrated yet because it
carries behaviour that deserves its own scope audit:
  - `declare_dependency` retry loop (5 attempts on
    `stale_graph_revision`, with `stage_dependency_edge` +
    `apply_dependency_to_child` fan-out).
  - `check_dependencies` envelope walk across
    `list_incoming_edges` + per-edge `evaluate_flow_eligibility`.
  - `submit` / `complete` / `fail` / `cancel` / `claim` / `renew` /
    `heartbeat` lifecycle ops that share ARGV pre-read patterns
    with run_service but operate on flow-scoped edge state.

PR 2b will apply the same trait-delegation pattern, adding ~10
more methods to `ControlPlaneBackend` (or, if the audit shows it,
carving a `TaskDependencyBackend` sibling for the edge-shaped
ops). `claim_common::issue_grant_and_claim` is already PR-2a-ready
— task_service only needs to pass `&self.control_plane` there
(already done in 2a).

### Phase D exception — `scheduler_service.rs`

`FabricSchedulerService` continues to import `ff_scheduler::claim::
{Scheduler, ClaimGrant}` directly. `ClaimGrant` is a wire-contract
type shared with ff-sdk workers — mirroring it cairn-side adds a
conversion hop without real hiding because both cairn AND ff-sdk
workers must agree on the layout. The exception is documented at
the top of the file. Phase E / F may revisit if cairn eventually
runs its own scheduling loop against `ControlPlaneBackend`
directly.

## Phase E — typed error model (not started)

FCALL errors today arrive as `ferriskey::Value` envelopes parsed by
`helpers::check_fcall_success` et al. Phase E introduces a typed
`EngineError` enum returned by the trait, absorbing the parse step
into the engine boundary. Aligns cairn with RFC-012 Stage 1a, which
introduced `ff_core::EngineError` on the FF side.

## Phase F — swap-in upstream `describe_*` primitives

When FlowFabric#58 ships, `ValkeyEngine` shrinks to ~30 lines of
delegation; the typed snapshot structs become re-exports from the
`ff` umbrella crate. No caller-visible change.
