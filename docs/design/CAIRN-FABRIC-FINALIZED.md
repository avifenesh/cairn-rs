# Cairn ↔ FlowFabric Finalized Architecture

**Status**: finalization round. Captures the wiring, feature gates, operator workflow, and outstanding asks as of FF @a098710 and cairn branch `feat/cairn-fabric-finalization`.

`cairn-fabric` is a **thin bridge**. FlowFabric owns execution truth in Valkey; cairn translates HTTP handler calls into FF FCALLs and projects FF events back into cairn-store for query paths. This doc is the runbook, not a design proposal — behaviour is already shipped.

---

## 1. Architecture

```
  cairn-app handlers
     │  trait: RunService / TaskService / SessionService
     ▼
  ┌──────────────────┐   ┌──────────────────────┐
  │ in-memory impl   │   │ FabricAdapter        │
  │ (dev default,    │   │ (CAIRN_FABRIC_       │
  │  CAIRN_FABRIC_   │   │  ENABLED=1)          │
  │  ENABLED unset)  │   │ resolves project via │
  │                  │   │ store, delegates     │
  │  → InMemoryStore │   │ mutation to Fabric   │
  └───────┬──────────┘   └──────────┬───────────┘
          │                         ▼
          │              ┌────────────────────────┐
          │              │ cairn-fabric services  │
          │              │ {Run,Task,Session,     │
          │              │  Budget,Quota,         │
          │              │  Scheduler} +          │
          │              │ SignalBridge           │
          │              └──────────┬─────────────┘
          │                         │ FCALL ff_*
          │                         ▼
          │              ┌────────────────────────┐
          │  EventBridge │ Valkey + FF library    │
          │  queue       │ exec_core / leases /   │
          │  (FF→store)  │ waitpoints / signals / │
          ▼              │ budgets / flows +      │
  ┌──────────────┐       │ 14 ff-engine scanners  │
  │ cairn-store  │◄──────┤ (see § scanners)       │
  │ EventLog +   │       └────────────────────────┘
  │ projections  │
  │ serve GET /  │
  │ list queries │
  └──────────────┘
```

**Write path**: handler → trait → Fabric adapter → `FabricServices::{runs,tasks,sessions}` → FCALL → Valkey. FF state transitions emit bridge events → `EventBridge` queue → cairn-store append → projections update.

**Read path**: handler → trait → store projection (`RunReadModel::get`, `list_by_session`, etc). Fabric is NOT on the read path — FF doesn't index by cairn's `(tenant, workspace, project)` scope.

**Scanners** (FF-owned, cairn runs none of its own): `DelayedPromoter`, `LeaseExpiryScanner`, `AttemptTimeoutScanner`, `ExecutionDeadlineScanner`, `SuspensionTimeoutScanner`, `PendingWaitpointExpiryScanner`, `BudgetResetScanner`, `BudgetReconciler`, `QuotaReconciler`, `DependencyReconciler`, `FlowProjector`, `IndexReconciler`, `RetentionTrimmer`, `UnblockScanner`. The pre-Fabric `FabricRecoveryStub` was retired when finalization landed.

---

## 2. Feature gates

| Flag / env var | Default | Scope | Effect |
|---|---|---|---|
| `CAIRN_FABRIC_ENABLED` | unset (off) | env var | When set, `AppState` constructs `FabricServices`, replaces `state.runtime.{runs,tasks,sessions}` with the adapter. Unset = in-memory path for dev/local. |
| `CAIRN_FABRIC_WAITPOINT_HMAC_SECRET` | unset (boot fails) | env var | 64-char hex (32-byte) HMAC secret. **Required** when `CAIRN_FABRIC_ENABLED=1` — boot aborts with `FabricError::Config` if unset (no silent degrade). Dev paths that don't want HMAC must use the in-memory runtime by leaving `CAIRN_FABRIC_ENABLED` unset. |
| `CAIRN_FABRIC_WAITPOINT_HMAC_KID` | `k1` when secret set | env var | Kid for the HMAC secret. Must be non-empty and free of `:` (FF field-name delimiter). |
| `CAIRN_FABRIC_WORKER_CAPABILITIES` | empty set | env var | Comma-separated capability tokens. Passed to `ff_scheduler::Scheduler::claim_for_worker`. Empty = matches only executions with no capability requirements. |
| `CAIRN_FABRIC_HOST` / `_PORT` / `_TLS` / `_CLUSTER` | `localhost` / `6379` / off / off | env var | Valkey connection. `_CLUSTER=1` uses cluster mode; all FCALL KEYS on a single `{p:N}` hash tag so this is cluster-safe without extra wiring. |
| `CAIRN_FABRIC_LEASE_TTL_MS` / `_GRANT_TTL_MS` | `30_000` / `5_000` | env var | Timing knobs. |
| `insecure-direct-claim` | off | cargo feature | Forwards to `ff-sdk/insecure-direct-claim`, exposing `CairnWorker::claim_next`. **Test / local dev only** — the direct path skips budget + quota admission that only ff-scheduler enforces. Production must go through `FabricSchedulerService::claim_for_worker`. |
| `fabric-b4-idempotency` | off | cargo feature | Two-phase rollout for FF's B4 idempotency-key ARGV slot on `ff_report_usage_and_check`. Kept off until the FF version we pin ships it. |

---

## 3. Operator runbook

### 3.1 Enabling Fabric

1. Run a Valkey 8+ instance reachable from cairn.
2. Load the FlowFabric Lua library into Valkey (`scripts/run-fabric-integration-tests.sh` does this from a pinned FF checkout; production operators seed from their CI artefact).
3. Generate a 32-byte HMAC secret: `openssl rand -hex 32`.
4. Set env vars before boot. The HMAC secret is **required** when Fabric is enabled — boot fails loud if it's missing:
   ```bash
   export CAIRN_FABRIC_ENABLED=1
   export CAIRN_FABRIC_HOST=valkey.internal
   export CAIRN_FABRIC_PORT=6379
   export CAIRN_FABRIC_WAITPOINT_HMAC_SECRET=<64-hex>
   export CAIRN_FABRIC_WAITPOINT_HMAC_KID=prod-2026-04
   ```
   Omitting `CAIRN_FABRIC_WAITPOINT_HMAC_SECRET` (or supplying an invalid length / non-hex value) aborts `FabricServices::start` with a typed `FabricError::Config` — we do not ship a runtime that would reject every `ff_suspend_execution` with `hmac_secret_not_initialized` at runtime. Dev / CI paths that don't want HMAC must unset `CAIRN_FABRIC_ENABLED` and use the in-memory dev path.
5. Boot cairn-app. You should see:
   ```
   INFO connecting to valkey url=redis://valkey.internal:6379
   INFO seeded waitpoint HMAC secret kid=prod-2026-04 partitions=256
   INFO fabric runtime started
   ```

### 3.2 Worker capability advertisement

Workers that can claim tasks with specific hardware/runtime requirements advertise them via:

```bash
export CAIRN_FABRIC_WORKER_CAPABILITIES=gpu,cuda-12,linux-x86_64
```

FF builds a deterministic sorted CSV from the cairn-side `BTreeSet`. An execution's `required_capabilities` must be a subset of the worker's for `ff_issue_claim_grant` to succeed; otherwise FF blocks the execution and the `UnblockScanner` promotes it when a matching worker registers.

### 3.3 HMAC rotation

Not automated in this round. FF ships `rotate_waitpoint_hmac_secret` in `ff-test` fixtures; cairn will wrap it in a dedicated admin endpoint in a follow-up round. Until then: redeploy with a new `CAIRN_FABRIC_WAITPOINT_HMAC_SECRET`, accept that in-flight suspensions signed by the old key fail `invalid_token` until their waitpoints close. No silent data loss — operator sees the typed error.

### 3.4 Disabling Fabric for debugging

Unset `CAIRN_FABRIC_ENABLED`. AppState falls back to the in-memory `RunServiceImpl` / `TaskServiceImpl` / `SessionServiceImpl`. No data is shared between the two paths — in-memory runs are not visible from the Fabric path and vice versa.

### 3.5 Dev vs production paths

Two backing stacks live side-by-side in the binary; the env var picks which one AppState wires at boot.

| Path | Trigger | Run/Task/Session backing | Execution state lives in | Recovery | Recommended for |
|---|---|---|---|---|---|
| **Production** | `CAIRN_FABRIC_ENABLED=1` | `Fabric{Run,Task,Session}ServiceAdapter` | Valkey + FF Lua library | FF's 14 background scanners (`LeaseExpiryScanner`, `AttemptTimeoutScanner`, etc.) | real teams, production traffic |
| **Dev / CI** | unset / `0` | `RunServiceImpl` / `TaskServiceImpl` / `SessionServiceImpl` | cairn-store event log (in-memory) | none — no scanners, no Valkey | `cargo test`, local `cargo run -p cairn-app`, short-lived CI without a Valkey dependency |

The in-memory impls are **not** duplication of FF — they're the fallback when Fabric is disabled. Without them, running cairn-app without Valkey would be impossible and the test baseline couldn't validate cairn-side logic in isolation. Keep them.

The deleted pieces in finalization were: `FabricRecoveryStub` (cairn-fabric side) and `RecoveryServiceImpl` (cairn-runtime side). Both were passive duplicates of FF's scanners — FF owns recovery whether Fabric is enabled or not, so the cairn-side sweeps were redundant under FF-enabled and useless under FF-disabled (no background worker to drive them).

### 3.5 Common failures

| Symptom | Cause | Fix |
|---|---|---|
| `ff_suspend_execution rejected: hmac_secret_not_initialized` | HMAC secret not seeded on execution partitions | Set `CAIRN_FABRIC_WAITPOINT_HMAC_SECRET` and reboot. |
| `ff_deliver_signal rejected: invalid_token` | Waitpoint token stale (HMAC rotated mid-flight) or client passed empty token | Re-read from `waitpoint_hash.waitpoint_token` via cairn's `read_waitpoint_token` helper. |
| `ff_report_usage_and_check rejected: wrong number of arguments` | `fabric-b4-idempotency` enabled against an FF version without B4 | Turn the cargo feature off and rebuild. |
| `scheduler claim_for_worker: SchedulerError::Config(...)` on capability token | Operator supplied a token with `,` or whitespace | Fix config — cairn validates at boot, FF validates at claim. |
| CairnTask dropped without terminal call (log warn) | Handler forgot to call `complete` / `fail` / `cancel` | FF's `LeaseExpiryScanner` promotes the execution back to eligible when the lease TTL fires; no data loss, just latency. |

---

## 4. Outstanding FF asks

Filed with FF maintainers; tracked for re-wiring when they land.

1. **`pub fn FlowFabricWorker::claim_from_grant(grant: ClaimGrant) -> Result<ClaimedTask, SdkError>`** — required to route cairn's production worker path through `ff-scheduler` without enabling `insecure-direct-claim`. Today `ClaimedTask::new` is `pub(crate)` on ff-sdk, so cairn can obtain a `ClaimGrant` from the scheduler but cannot turn it into a `ClaimedTask` for the stream / renewal / terminal methods.
2. **`pub fn parse_report_usage_result(raw: &Value) -> Result<ReportUsageResult, SdkError>`** — ff-sdk's parser is private, so cairn-fabric re-implements it (`services/budget_service.rs::parse_spend_result`) and keeps it in sync by hand. Exposing the upstream parser eliminates a drift hazard.
3. **B4 idempotency ARGV stabilisation** on `ff_report_usage_and_check`. Cairn already computes a stable dedup key and passes it as `ARGV[2 * dim_count + 3]`; FF accepts it. When FF ships a typed `ReportUsageArgs.dedup_key: Option<String>` on the contract side, cairn switches to the typed path behind `fabric-b4-idempotency` cargo feature.
4. **Scheduler-mediated `claim_resumed_execution`** — same visibility problem as #1 for the resume-after-suspend path. Needs a `pub` entry point so cairn can consume a scheduler grant for a previously-suspended execution.

---

## 5. Known gaps

Accepted limitations as of finalization — each has a follow-up round scoped.

- **`FabricTaskService::declare_dependency` + `check_dependencies` return errors / empty vecs** (`services/task_service.rs:352-370`). The plan is FF flow-edge FCALLs (`ff_stage_dependency_edge`, `ff_apply_dependency_to_child`) wrapped by Fabric. Blocked on Session→Flow DAG work (Phase 3). Handler `add_task_dependency_handler` must stay on the store-event path until then; adapter routes accordingly.
- **`FabricTaskService::list_expired_leases`** returns empty (FF's `LeaseExpiryScanner` handles expiry server-side). Admin handler `expire_task_leases_handler` becomes a no-op under Fabric — delegate to `TaskReadModel::list_expired_leases` in the adapter or mark the endpoint deprecated.
- **`FabricTaskService::list_by_state`** and **`FabricSessionService::list`** return empty — FF doesn't index by cairn's `(tenant, workspace, project)` scope. Adapter delegates to `TaskReadModel` / `SessionReadModel` projections. Not a defect; it's the read-path split.
- **Provider budgets and tenant quotas** (`handlers/providers.rs` + `handlers/admin.rs`) remain on the legacy `BudgetServiceImpl` / `QuotaServiceImpl` over the cairn event log. `FabricBudgetService` and `FabricQuotaService` are per-run admission controls, not tenant-wide ceilings; they ride alongside as new surfaces, not replacements.
- **Approval path has no active live-FF integration test** (three `test_suspension.rs` tests are `#[cfg(any())]`-gated) because `FabricRunService` has no `claim` API — a run's execution never reaches `lifecycle_phase=active`, which `ff_suspend_execution` requires. Worker-1 owns the run-claim landing. Once it lands, re-enable the gated tests.
- **`ActiveTaskRegistry` is a latency-cache for lease context**, not a source of truth — FF owns every field it stores. It also doubles as the gate for terminal-state `TaskStateChanged` emission, which means API-claimed tasks (claimed without routing through `FabricTaskService::claim`) skip the projection update. Flagged MAJOR in the finalization audit; scheduled for removal once the claim-API work stabilises. See audit report for detail.
- **`scripts/smoke-test.sh` HTTP harness assumes a permissive state machine** (the in-memory `RunServiceImpl` allows `pause` without a prior `claim`, etc). Under `CAIRN_FABRIC_ENABLED=1`, FF enforces strict state transitions — a run must be claimed and active before it can be paused/resumed, tasks must be submitted-and-claimed before lease operations. 8 smoke sections (`POST /v1/runs/:id/pause`, `/resume`, `POST /v1/tasks/:id/claim`, `/release-lease`) return HTTP 500 for this reason on the fabric path. These are **NOT runtime defects** — FF's strictness is correct production behaviour. The smoke-test harness needs a short-lived worker-loop fixture (claim-then-operate) to exercise FF semantics correctly. Filed as a separate follow-up PR: **"smoke-test: harness rewrite for Fabric state machine"**.
  - Fabric-off path (`CAIRN_FABRIC_ENABLED` unset): 97/97 pass, 4 skipped.
  - Fabric-on path (`CAIRN_FABRIC_ENABLED=1` + Valkey + FF lua loaded): 89/97 pass, 8 harness gaps, 4 skipped.

---

## 6. Known tech debt (flagged during finalization-round audit)

Severity ordered. Each item has a follow-up round scoped.

### HIGH — Event emission gate bug

*Location*: `crates/cairn-fabric/src/services/task_service.rs:566-578, 667-684, 751-764`.

`FabricTaskService::complete` / `fail` / `cancel` emit `BridgeEvent::TaskStateChanged` only if the `ActiveTaskRegistry` has an entry for the task id (`was_registered = registry.remove_entry(task_id); if was_registered { bridge.emit(...) }`). The registry is populated only when the task is claimed through `FabricTaskService::claim`. Tasks claimed via the `insecure-direct-claim` feature (`CairnWorker::claim_next`) or through any external API caller that bypasses cairn-fabric's claim path never populate the registry — so their terminal transitions silently skip event emission, and the cairn-store `TaskReadModel` diverges from FF's exec_core.

This is a correctness defect, not a style concern. Handlers that read `state.runtime.tasks.get(task_id)` (which goes to the store projection, not FF) get stale state for any task whose lifecycle crossed the registry-less boundary.

Fix: emit `TaskStateChanged` unconditionally after FF confirms the terminal transition. Cairn-store projections are already idempotent on `(task_id, event_id)` — the worst case is a no-op re-write if a prior path already emitted, which is cheap. Remove the `was_registered` gate at all three call sites.

### MEDIUM — `ActiveTaskRegistry` duplicates FF-owned state

*Location*: `crates/cairn-fabric/src/active_tasks.rs` (entire file).

`DashMap<TaskId, {execution_id, lease_id, lease_epoch, attempt_index, Option<ClaimedTask>}>`. Every non-transient field lives authoritatively in FF:
- `execution_id` is deterministic from `id_map::task_to_execution_id(project, task_id)` (pure UUID v5).
- `lease_id`, `lease_epoch`, `attempt_index` all live in `exec_core.current_lease_*` fields on the Valkey side.
- `ClaimedTask` is ff-sdk's wrapper around FF state, not cairn state.

The registry is only a latency-cache — `services/task_service.rs:80` and `:121` already show the fallback: `if let Some(ctx) = registry.get_lease_context(task_id) return Ok(ctx); else HGETALL exec_core and parse`. One round-trip saved per terminal call, at the cost of the HIGH bug above plus a lean-bridge violation.

Removal: ~80 LOC in `active_tasks.rs` + ~40 LOC fallout in `task_service.rs` and `worker_sdk.rs`. Coupled with the HIGH above — fix both in the same follow-up round after `task_service.rs` stabilises (currently hot with claim/stream work).

### MEDIUM — Bridge-event completeness audit

The finalization-round smoke-test caught a single-emit gap: `FabricSessionService::create` did not emit a `BridgeEvent::SessionCreated`, so sessions were never written to the cairn-store projection, and every subsequent handler that read `sessions.get(session_id)` returned `None`. Fixed in commit 4 (see `event_bridge.rs` + `services/session_service.rs`).

The same class of gap could exist for any FF mutation path cairn-fabric exposes. A systematic audit should walk every public method on `FabricRunService` / `FabricTaskService` / `FabricSessionService` / `FabricBudgetService` / `FabricQuotaService` and verify: every mutation that changes FF state that cairn-app reads back from the projection has a corresponding `BridgeEvent` emitted. No cairn-app read-path should depend on a state change that only lives in Valkey.

Bugs of this shape are invisible to unit tests (services write to FF correctly) and to live FF integration tests (each one tests the fabric layer in isolation, not the full handler → adapter → fabric → store-projection chain). The integration-readiness gate is cairn-app smoke — run `CAIRN_FABRIC_ENABLED=1 scripts/smoke-test.sh` on every cairn-fabric mutation-surface change.

### LOW — Smoke-test harness rewrite for Fabric state machine

`scripts/smoke-test.sh` simulates in-memory permissive state transitions (pause without prior claim, etc.). Under `CAIRN_FABRIC_ENABLED=1`, FF enforces strict state transitions, so 8 sections (pause/resume/claim/release-lease on freshly created runs and tasks) return HTTP 500 — the runtime is correct, the test's expectations aren't. Follow-up PR adds a worker-loop fixture that claims before operating so the fabric path gets exercised end-to-end.

### LOW — CairnTask tag micro-cache

*Location*: `crates/cairn-fabric/src/worker_sdk.rs:174-180`.

`CairnTask` caches `run_id`, `session_id`, `project` extracted from FF exec_core tags at claim time. Struct-lifetime only, not persistent. Re-reading tags on every terminal call adds an HGET per call but eliminates a staleness risk if operator-directed reassignment ever enters scope. Keep as-is until that feature arrives.

### LOW — `TaskLeaseClaimed.lease_expires_at_ms` is a snapshot

*Location*: `crates/cairn-fabric/src/event_bridge.rs:56-62`.

The `TaskLeaseClaimed` bridge event payload carries `lease_expires_at_ms`, which is also stored on FF's `exec_core` as `current_lease_expires_at`. This is a moment-in-time snapshot for projection display, not a cached field with invalidation semantics — consumers must not treat the value as tracked (FF can extend / renew the lease after the event is emitted). Documented here so future readers don't mistake it for live state.

## 7. Versioning

- Pinned FF rev: `a09871000574388256b1dd7c910239e992c0d3a6` (in every `crates/cairn-fabric/Cargo.toml` `rev = …` entry).
- Cairn-fabric crate version: `0.1.0` (unpublished, `publish = false`).
- Bumping the FF rev requires the `scripts/run-fabric-integration-tests.sh` `FF_REV` env var to match. The script fails fast on mismatch.
