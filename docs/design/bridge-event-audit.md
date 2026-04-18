# Bridge-event completeness audit — cairn-fabric services

**Status:** Complete, 2026-04-18.
**Scope:** Every `pub async fn` on `FabricRunService`, `FabricTaskService`, `FabricSessionService`, `FabricSchedulerService`, `FabricWorkerService`, `FabricBudgetService`, `FabricQuotaService` that mutates FF state (Valkey / FCALL side-effects).
**Sibling work:** FF-side bridge-event gap report at `avifenesh/FlowFabric` `benches/perf-invest/bridge-event-gap-report.md` (PR #22). This audit covers the cairn-method-inward axis (did cairn's own wrapper emit?); the FF-side report covers the FF-initiated-transition axis (did FF emit an XADD that cairn should subscribe to?). Both axes are needed to characterise the full bridge.

## §1 Background

Cairn does not subscribe to any FF stream for lifecycle observation. Its bridge is cairn-internal: every `BridgeEvent::*` emit is from a cairn wrapper method that just called an FCALL. The `EventBridge` consumer appends the mapped `RuntimeEvent` to cairn-store's event log, which feeds sync projections (`SessionReadModel`, `RunRecord`, `TaskRecord`, etc.) and the SSE publisher.

The 12 `BridgeEvent` variants (`crates/cairn-fabric/src/event_bridge.rs:17-87`) map to `RuntimeEvent` variants that have projection consumers in all three store backends (Postgres / SQLite / InMemory). Every variant has a live consumer — no dangling emits.

**Source-of-truth rule:** when an FF mutation has no corresponding projection read model on the cairn side, silence is correct. When a projection DOES derive state from the transition but no emit fires, the read model drifts from FF truth. The audit classifies every mutation path into one of three buckets:

- **EMITS** — wrapper emits a `BridgeEvent` on the success path; projection stays in sync.
- **SILENT-OK** — no projection reads this transition; intentional lean-bridge silence.
- **GAP** — projection derives from this transition but no emit fires; read model drifts.

## §2 Audit table

### §2.1 `FabricRunService` (run_service.rs)

| fn | FCALL | BridgeEvent emitted | Classification | Notes |
|----|-------|---------------------|----------------|-------|
| `start` / `start_with_correlation` | `FF_CREATE_EXECUTION` | `ExecutionCreated` (guarded by `!is_duplicate_result`) | EMITS | Correlation-id threaded onto envelope. |
| `get` | — | — | SILENT-OK | Read, not mutation. |
| `list_by_session` | — | — | SILENT-OK | Read; documented no-op — cairn-store projection answers. |
| `claim` | `ff_issue_claim_grant` + `ff_claim_execution` | none | SILENT-OK | Non-idempotent FF contract; documented at `docs/design/CAIRN-FABRIC-FINALIZED.md` §4.3 (PR #35). Lean-bridge: FF owns `ownership_state`; no cairn projection reader of `RunClaimed`. |
| `complete` | `FF_COMPLETE_EXECUTION` | `ExecutionCompleted` | EMITS | Unconditional on FCALL success. |
| `fail` | `FF_FAIL_EXECUTION` | `ExecutionFailed` (only when terminal per `parse_fail_outcome`) | EMITS | Retry-scheduled fail intentionally silent — non-terminal. |
| `cancel` | `FF_CANCEL_EXECUTION` | `ExecutionCancelled` | EMITS | Unconditional on success. |
| `pause` | `FF_SUSPEND_EXECUTION` | `ExecutionSuspended` (guarded by `!is_already_satisfied`) | EMITS | |
| `resume` | `FF_RESUME_EXECUTION` | `ExecutionResumed` | EMITS | |
| `enter_waiting_approval` | `FF_SUSPEND_EXECUTION` (approval waitpoint) | `ExecutionSuspended` (guarded by `!is_already_satisfied`) | EMITS | |
| `resolve_approval` (approved branch) | `FF_DELIVER_SIGNAL` | `ExecutionResumed` | EMITS | |
| `resolve_approval` (rejected branch) | delegates to `fail(…, ApprovalRejected)` | — | EMITS | Covered by `fail`. |

**Run service verdict:** clean. Every mutation either emits or has documented intentional silence.

### §2.2 `FabricTaskService` (task_service.rs)

| fn | FCALL | BridgeEvent emitted | Classification | Notes |
|----|-------|---------------------|----------------|-------|
| `submit` | `FF_CREATE_EXECUTION` | `TaskCreated` (guarded by `!is_duplicate_result`) | EMITS | |
| `declare_dependency` | — | — | SILENT-OK | Documented error path — FF flow edges own dependencies, not cairn projections. |
| `check_dependencies` | — | — | SILENT-OK | Read. |
| `get` | — | — | SILENT-OK | Read. |
| `claim` | via `issue_grant_and_claim` helper | `TaskLeaseClaimed` | EMITS | `lease_epoch` carries task-record version; no cairn-side lease cache (lean bridge). |
| `heartbeat` | `FF_RENEW_LEASE` | none | SILENT-OK | Lease extension is not a new claim; projection `TaskReadModel` renews `lease_expires_at` via `TaskLeaseHeartbeated` not `TaskLeaseClaimed`. No renewal BridgeEvent exists; heartbeat frequency would swamp the projection. Accepted as lean-bridge silence. |
| `start` | — | — | SILENT-OK | No-op: FF transitions to active on claim. |
| `complete` | `FF_COMPLETE_EXECUTION` | `TaskStateChanged(Completed)` | EMITS | Unconditional; projection idempotent on (task_id, event_id). |
| `fail` | `FF_FAIL_EXECUTION` | `TaskStateChanged(Failed)` (only terminal per `parse_fail_outcome`) | EMITS | Retry-scheduled fail intentionally silent. |
| `cancel` | `FF_CANCEL_EXECUTION` | `TaskStateChanged(Canceled)` | EMITS | Unconditional. |
| `dead_letter` | — | — | SILENT-OK | Read-only passthrough. |
| `list_dead_lettered` | — | — | SILENT-OK | Documented no-op. |
| **`pause`** | `FF_SUSPEND_EXECUTION` | **none** | **GAP (G1)** | FF state transitions to suspended (`TaskState::Paused` or `WaitingApproval`), projection never sees it. See §3.1. |
| **`resume`** | `FF_RESUME_EXECUTION` | **none** | **GAP (G2)** | FF state returns to runnable (`TaskState::Queued`/`Leased`/`Running`), projection never sees it. See §3.1. |
| `list_by_state` | — | — | SILENT-OK | Documented no-op. |
| `list_expired_leases` | — | — | SILENT-OK | FF's `lease_expiry` scanner handles this. |
| `release_lease` | — | — | SILENT-OK | No-op passthrough. |

**Task service verdict:** two confirmed gaps (G1, G2). See §3.1 for mitigation.

### §2.3 `FabricSessionService` (session_service.rs)

| fn | FCALL | BridgeEvent emitted | Classification | Notes |
|----|-------|---------------------|----------------|-------|
| `create` | `FF_CREATE_FLOW` + HSET tags | `SessionCreated` (guarded by `!is_already_satisfied`) | EMITS | |
| `get` | — | — | SILENT-OK | Read. |
| `list` | — | — | SILENT-OK | Documented no-op — cairn-store projection answers. |
| `archive` | `FF_CANCEL_FLOW` + HSET `cairn.archived` | `SessionArchived` | EMITS | `flow_already_terminal` tolerated (session still archives on cairn side). |

**Session service verdict:** clean.

### §2.4 `FabricSchedulerService` (scheduler_service.rs)

| fn | FCALL | BridgeEvent emitted | Classification | Notes |
|----|-------|---------------------|----------------|-------|
| `claim_for_worker` | `ff_issue_claim_grant` (via `Scheduler`) | none | SILENT-OK | Claim-grant is transient pre-claim state; the eventual `task_service::claim` emits `TaskLeaseClaimed`. No cairn projection for grants. |

**Scheduler service verdict:** clean. One row; grants are a lean-bridge-by-design concept.

### §2.5 `FabricWorkerService` (worker_service.rs)

| fn | FCALL / Valkey ops | BridgeEvent emitted | Classification | Notes |
|----|---------------------|---------------------|----------------|-------|
| `register_worker` | HSET worker hash + SADD index + per-cap SADDs | none | SILENT-OK | Worker registry is FF-owned operational state. No cairn-store projection for `WorkerRecord`. |
| `heartbeat_worker` | HSET `last_heartbeat_ms` + PEXPIRE | none | SILENT-OK | Same as above. TTL-driven liveness, no audit need. |
| `mark_worker_dead` | HSET `is_alive=false` | none | SILENT-OK | Same rationale. If cairn ever surfaces worker-status in the UI, this becomes a gap. |
| `claim_next` | via `Scheduler::claim_for_worker` | none | SILENT-OK | Transient grant; eventual claim emits (see §2.4). |

**Worker service verdict:** clean. Workers are FF-owned; no projection to drift.

### §2.6 `FabricBudgetService` (budget_service.rs)

| fn | FCALL | BridgeEvent emitted | Classification | Notes |
|----|-------|---------------------|----------------|-------|
| `create_budget` / `create_run_budget` / `create_tenant_budget` | `FF_CREATE_BUDGET` | none | SILENT-OK | Budget state is FF-owned. No `BudgetRecord` projection in cairn-store. Operators read via `get_budget_status` (FF HGETALL). |
| `release_budget` | `FF_RESET_BUDGET` | none | SILENT-OK | Same. |
| `record_spend` | `FF_REPORT_USAGE_AND_CHECK` | none | SILENT-OK | High-volume op (every tool call); emitting would saturate the bridge. Spend results returned inline to caller. |
| `get_budget_status` | — | — | SILENT-OK | Read. |

**Budget service verdict:** clean. Budgets are FF-owned operational state; cairn does not project. Admin read surface reads FF directly.

### §2.7 `FabricQuotaService` (quota_service.rs)

| fn | FCALL | BridgeEvent emitted | Classification | Notes |
|----|-------|---------------------|----------------|-------|
| `create_quota_policy` / `create_tenant_quota` / `create_workspace_quota` / `create_user_quota` | `FF_CREATE_QUOTA_POLICY` + HSET scope fields | none | SILENT-OK | Quota state is FF-owned. No cairn-store projection. |
| `check_admission` / `check_admission_for_run` | `FF_CHECK_ADMISSION_AND_RECORD` | none | SILENT-OK | High-volume op (every request/admission). Admission decision returned inline. |

**Quota service verdict:** clean. Same rationale as budget service.

### §2.8 `services/claim_common.rs`

Not audited as a separate row. `pub(crate) issue_grant_and_claim` is the helper both `RunService::claim` and `TaskService::claim` delegate to; its callers own the emit (or documented silence). No public mutation surface.

## §3 Findings

### §3.1 Gaps (2)

**G1 — `FabricTaskService::pause` does not emit `BridgeEvent::TaskStateChanged`.**

- **Site:** `crates/cairn-fabric/src/services/task_service.rs:769-939`.
- **FF side:** `FF_SUSPEND_EXECUTION` commits cleanly (Valkey state reflects `public_state=suspended`, `ownership_state=lease_paused`, task reads as `TaskState::Paused` or `WaitingApproval`).
- **cairn side:** no `bridge.emit(...)` call anywhere in `pause`. Projection `TaskReadModel` never transitions to `Paused`; SSE subscribers never observe the transition.
- **User-visible impact:** `POST /v1/tasks/:id/pause` succeeds, `GET /v1/tasks/:id` returns fresh-from-FF so looks right via HTTP, but the cairn-store projection lags until the next terminal emit (complete / fail / cancel). Audit log misses the pause event. SSE clients do not observe pause.
- **Asymmetry:** `FabricRunService::pause` (line 809-817) DOES emit `ExecutionSuspended` via the same FF_SUSPEND_EXECUTION path. Task path was missed.
- **Projection readiness:** `InMemoryStore` already handles `RuntimeEvent::TaskStateChanged` with `to == TaskState::Paused` — clears lease fields, bumps version. Postgres + SQLite mirror this.

**G2 — `FabricTaskService::resume` does not emit `BridgeEvent::TaskStateChanged`.**

- **Site:** `crates/cairn-fabric/src/services/task_service.rs:941-1001`.
- **FF side:** `FF_RESUME_EXECUTION` commits cleanly.
- **cairn side:** same omission. Projection stays at `Paused`; resume is invisible until the next terminal emit.
- **Asymmetry:** `FabricRunService::resume` (line 893-899) DOES emit `ExecutionResumed`.

### §3.2 Mitigation

**No FF dependency.** Both fixes are pure cairn-side additions — the FCALL already succeeds, we just need to emit afterward. Pattern mirrors `FabricRunService::pause` / `resume` exactly:

```rust
// pause — after check_fcall_success on FF_SUSPEND_EXECUTION:
let record = self.read_task_record(project, task_id).await?;
if !is_already_satisfied(&raw) {
    self.bridge
        .emit(BridgeEvent::TaskStateChanged {
            task_id: task_id.clone(),
            project: record.project.clone(),
            to: record.state, // reflects Paused / WaitingApproval from FF read
            failure_class: None,
        })
        .await;
}
Ok(record)

// resume — after check_fcall_success on FF_RESUME_EXECUTION:
let record = self.read_task_record(project, task_id).await?;
self.bridge
    .emit(BridgeEvent::TaskStateChanged {
        task_id: task_id.clone(),
        project: record.project.clone(),
        to: record.state, // reflects Queued / Leased / Running from FF read
        failure_class: None,
    })
    .await;
Ok(record)
```

**Why read `record.state` instead of hard-coding `TaskState::Paused`:** FF's `map_reason_to_blocking` can route `OperatorPause` to either `operator_hold` (→ `Paused`) or `waiting_for_approval` (→ `WaitingApproval`); the authoritative value is what FF actually committed. `read_task_record` HGETALLs `exec_core` fresh, so it reflects the post-FCALL truth. Same pattern as `FabricRunService::pause` which reads `prev_run_state` from exec_core.

**Risk:** low. Projection is idempotent on `(task_id, event_id)`. `is_already_satisfied` guard on pause prevents double-emit when FF reports the suspension was already in place.

**Tests:** existing integration test `crates/cairn-fabric/tests/integration/test_suspension.rs` only asserts against the fabric `tasks.get()` path (HGETALL direct). Fix must add a cairn-store projection assertion — subscribe to the event log (or read `TaskReadModel` after a `bridge.stop()` drain) and verify `TaskStateChanged(Paused)` + `TaskStateChanged(Running)` project through.

### §3.3 Not our axis (referenced)

Two FF-initiated bridge-event gaps exist but are out of scope for this audit. They live on the axis of "FF transitions state without cairn calling anything" and require either FF-side XADD additions or a new cairn-side stream subscriber. Pointers only, per the load-bearing finding in FF's report (`bridge-event-gap-report.md`):

- **§1.1 FF-initiated lease expiry reclaim** (`ff_mark_lease_expired_if_due`) — worker dies, FF scanner reclaims the lease, cairn projection stuck at `Running`. Fix: cairn subscribes to `lease_history_key`. FF already emits; cairn doesn't consume.
- **§1.3 FF runnable-branch timeout** (`ff_expire_execution` @ `lua/execution.lua:1464-1615`) — deadline hits on an unclaimed execution, FF writes terminal state but emits no `lease_history` XADD (active/suspended branches do; runnable branch doesn't). Fix: FF adds symmetric XADD, then cairn's §1.1 subscriber picks it up.

These are tracked upstream and will get their own cairn-rs issues once direction is chosen.

## §4 RFC-011 impact assessment

RFC-011 (FF `avifenesh/FlowFabric#23`, OPEN) reshapes `ExecutionId` from bare UUID to `{fp:N}:<uuid>` hash-tagged form for cross-slot atomicity of `ff_add_execution_to_flow`. Phase 4 is explicitly cairn-side re-alignment: `RunService::start` and `TaskService::submit` migrate to `ExecutionId::for_flow` / `solo` constructors.

**Does RFC-011 change the fix shape for G1 / G2?** No.

- The FCALL contracts for `FF_SUSPEND_EXECUTION` / `FF_RESUME_EXECUTION` don't change.
- The `BridgeEvent` enum is explicitly a non-goal (RFC-011 §11: "Changing the wire shape of cairn's BridgeEvent::* variants" listed as out-of-scope).
- Our emit site (after `check_fcall_success`) survives the RFC-011 migration 1:1.
- Projection consumers of `RuntimeEvent::TaskStateChanged` don't care about the exec-id shape.

**Does RFC-011 change the silent-OK classifications?** No. Budget / quota / worker / scheduler all remain FF-owned with no cairn projection; RFC-011 only changes how exec_ids are routed.

Safe to land G1/G2 fix independently, pre-RFC-011-merge. No FF version bump required.

## §5 Deliverables

1. This document.
2. Cairn-rs issue for G1 + G2: one PR fixes both (~30 LOC + test).
3. Cairn-rs issue tracking the FF-initiated lease-expiry subscriber (§3.3 first bullet).
4. FF issue for the §3.3 second bullet (runnable-branch XADD).
5. Inline doc-comments on `FabricWorkerService`, `FabricBudgetService`, `FabricQuotaService`, and `FabricSchedulerService::claim_for_worker` explaining their intentional lean-bridge silence — so a future reviewer doesn't accidentally add emits.

## §6 References

- `crates/cairn-fabric/src/event_bridge.rs:17-87` — canonical `BridgeEvent` enum.
- `crates/cairn-fabric/src/event_bridge.rs:237-401` — `bridge_event_to_runtime_event` mapping.
- `crates/cairn-store/src/in_memory.rs:350-369` — `TaskStateChanged` projection handler (includes Paused lease-clear).
- `docs/design/CAIRN-FABRIC-FINALIZED.md` §4.3 — `runs.claim` non-idempotency contract (precedent for documented silence).
- `avifenesh/FlowFabric` PR #22 — FF-side bridge-event audit (W2 inventory + W3 gap report).
- `avifenesh/FlowFabric` PR #23 — RFC-011 exec/flow co-location.
