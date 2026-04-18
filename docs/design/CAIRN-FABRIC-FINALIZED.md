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

The runtime path is selected at **compile time** by cargo features on
`cairn-app`. There is no longer a runtime env var toggle — this was removed
in the PR #27 consolidation because "Fabric off" production builds are
unsupported (FF is the only correctness-guaranteed path).

| Flag / env var | Default | Scope | Effect |
|---|---|---|---|
| `in-memory-runtime` | OFF | cargo feature on `cairn-app` (forwards to `cairn-runtime`, `cairn-orchestrator`, `cairn-tools`) | When OFF (default / production), `AppState::new` constructs `FabricServices` and installs `Fabric{Run,Task,Session}ServiceAdapter` on `state.runtime.{runs,tasks,sessions}`. When ON, the event-log-only `RunServiceImpl` / `TaskServiceImpl` / `SessionServiceImpl` are compiled in and wired instead — no Valkey, no scanners, no correctness guarantees. Local tinkering and unit-test harnesses only. |
| `CAIRN_FABRIC_WAITPOINT_HMAC_SECRET` | unset (boot fails) | env var | 64-char hex (32-byte) HMAC secret. **Required** in the default (Fabric) build — boot aborts with `FabricError::Config` if unset (no silent degrade). Dev paths that don't want HMAC must build with `--features in-memory-runtime` so Fabric boot is skipped. |
| `CAIRN_FABRIC_WAITPOINT_HMAC_KID` | `k1` when secret set | env var | Kid for the HMAC secret. Must be non-empty and free of `:` (FF field-name delimiter). |
| `CAIRN_FABRIC_WORKER_CAPABILITIES` | empty set | env var | Comma-separated capability tokens. Passed to `ff_scheduler::Scheduler::claim_for_worker`. Empty = matches only executions with no capability requirements. |
| `CAIRN_FABRIC_HOST` / `_PORT` / `_TLS` / `_CLUSTER` | `localhost` / `6379` / off / off | env var | Valkey connection. `_CLUSTER=1` uses cluster mode; all FCALL KEYS on a single `{p:N}` hash tag so this is cluster-safe without extra wiring. |
| `CAIRN_FABRIC_LEASE_TTL_MS` / `_GRANT_TTL_MS` | `30_000` / `5_000` | env var | Timing knobs. |
| `insecure-direct-claim` | off | cargo feature | Forwards to `ff-sdk/insecure-direct-claim`, exposing `CairnWorker::claim_next`. **Test / local dev only** — the direct path skips budget + quota admission that only ff-scheduler enforces. Production must go through `FabricSchedulerService::claim_for_worker`. |

---

## 3. Operator runbook

### 3.1 Enabling Fabric

Fabric is the default — `cargo build -p cairn-app` produces a binary
that requires Valkey at boot. To get a Fabric-backed deployment up:

1. Run a Valkey 8+ instance reachable from cairn.
2. Load the FlowFabric Lua library into Valkey (`scripts/run-fabric-integration-tests.sh` does this from a pinned FF checkout; production operators seed from their CI artefact).
3. Generate a 32-byte HMAC secret: `openssl rand -hex 32`.
4. Set env vars before boot. The HMAC secret is **required** — boot fails loud if it's missing:
   ```bash
   export CAIRN_FABRIC_HOST=valkey.internal
   export CAIRN_FABRIC_PORT=6379
   export CAIRN_FABRIC_WAITPOINT_HMAC_SECRET=<64-hex>
   export CAIRN_FABRIC_WAITPOINT_HMAC_KID=prod-2026-04
   ```
   Omitting `CAIRN_FABRIC_WAITPOINT_HMAC_SECRET` (or supplying an invalid length / non-hex value) aborts `FabricServices::start` with a typed `FabricError::Config` — we do not ship a runtime that would reject every `ff_suspend_execution` with `hmac_secret_not_initialized` at runtime. Dev / CI paths that don't want HMAC must build with `--features in-memory-runtime` so Fabric boot is skipped entirely.
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

Rebuild with `cargo build -p cairn-app --features in-memory-runtime`.
`AppState::new` then skips `FabricServices::start` entirely and wires
the event-log-only `RunServiceImpl` / `TaskServiceImpl` /
`SessionServiceImpl` onto `state.runtime.{runs,tasks,sessions}`. No
data is shared between the two builds — in-memory runs are not visible
from the Fabric build and vice versa.

This is a **local / test harness escape hatch**, not a supported
production mode. The in-memory impls do not participate in FF's
scanner lifecycle, so any state that requires recovery (leases, budgets,
delayed promotion) will not work.

### 3.5 Dev vs production paths

Two backing stacks live in the codebase, gated by the
`in-memory-runtime` cargo feature — `AppState::new` wires the
corresponding impl at compile time.

| Path | Trigger | Run/Task/Session backing | Execution state lives in | Recovery | Recommended for |
|---|---|---|---|---|---|
| **Production** | default build (`in-memory-runtime` OFF) | `Fabric{Run,Task,Session}ServiceAdapter` | Valkey + FF Lua library | FF's 14 background scanners (`LeaseExpiryScanner`, `AttemptTimeoutScanner`, etc.) | real teams, production traffic |
| **Dev / CI** | `--features in-memory-runtime` | `RunServiceImpl` / `TaskServiceImpl` / `SessionServiceImpl` | cairn-store event log (in-memory) | none — no scanners, no Valkey | `cargo test`, local `cargo run -p cairn-app --features in-memory-runtime`, short-lived CI without a Valkey dependency |

The in-memory impls are **courtesy backings for tinkerers**, not peers
of FF. They exist so that unit tests and local exploration can run
without standing up Valkey; they do not attempt to replicate FF's
correctness semantics (strict state transitions, lease recovery,
budget scanners).

The deleted pieces in finalization were: `FabricRecoveryStub` (cairn-fabric side) and `RecoveryServiceImpl` (cairn-runtime side). Both were passive duplicates of FF's scanners — FF owns recovery on both paths, so the cairn-side sweeps were redundant under the Fabric build and useless under the in-memory build (no background worker to drive them).

### 3.6 Common failures

| Symptom | Cause | Fix |
|---|---|---|
| `ff_suspend_execution rejected: hmac_secret_not_initialized` | HMAC secret not seeded on execution partitions | Set `CAIRN_FABRIC_WAITPOINT_HMAC_SECRET` and reboot. |
| `ff_deliver_signal rejected: invalid_token` | Waitpoint token stale (HMAC rotated mid-flight) or client passed empty token | Re-read from `waitpoint_hash.waitpoint_token` via cairn's `read_waitpoint_token` helper. |
| `scheduler claim_for_worker: SchedulerError::Config(...)` on capability token | Operator supplied a token with `,` or whitespace | Fix config — cairn validates at boot, FF validates at claim. |
| CairnTask dropped without terminal call (log warn) | Handler forgot to call `complete` / `fail` / `cancel` | FF's `LeaseExpiryScanner` promotes the execution back to eligible when the lease TTL fires; no data loss, just latency. |

---

## 4. Orchestrator integration

The GATHER → DECIDE → EXECUTE loop in `cairn-orchestrator` threads FF
primitives through a narrow, **non-consuming** handle — `TaskFrameSink`
(`crates/cairn-orchestrator/src/task_sink.rs`). This keeps the loop
independent of FF's lease-lifetime contract (where `complete` /
`fail` / `suspend_for_approval` consume `self`) while still populating
FF's attempt-scoped stream for audit + cross-process resumption.

### 4.1 `TaskFrameSink` trait

Five methods, all non-consuming:

| Method | Called from | Purpose |
|---|---|---|
| `log_tool_call(name, args)` | `loop_runner.rs` §5a (pre-execute), §4 (approval gate) | Intent frame — survives a mid-dispatch restart |
| `log_tool_result(name, output, success, duration_ms)` | `loop_runner.rs` §5b (post-execute, per result) | Outcome frame — balances the intent frame |
| `log_llm_response(model, tokens_in, tokens_out, latency_ms)` | `loop_runner.rs` §3b' (post-decide) | Cost + latency audit off `DecideOutput` fields |
| `save_checkpoint(context_json)` | `loop_runner.rs` §6b (after `CheckpointHook::save`) | Matches PR #27's `restore_frames()` read half |
| `is_lease_healthy()` | `loop_runner.rs` §1b (per-iteration, before gather) | Bail before committing any expensive side effect |

Default impl: `NoOpTaskSink` — all writes succeed instantly, lease
always reports healthy. Used when the caller has no live
`CairnTask`; the `EventLog` bridge events continue to drive cairn-store
projections + SSE telemetry unchanged.

Production impl: blanket `impl TaskFrameSink for cairn_fabric::CairnTask`
— delegates to the existing `StreamWriter` (ff-sdk
`ff_append_frame`-backed). Errors are mapped to
`OrchestratorError::Execute`; the loop runner logs and continues so a
frame append that fails (network blip, Valkey hiccup) does not fail a
run.

### 4.2 Stream frames are ADDITIVE

Cairn-store bridge events (`RuntimeEvent::ToolInvoked`,
`RuntimeEvent::CheckpointRecorded`, etc.) are **not replaced**. Stream
frames write to FF's attempt-scoped Valkey stream; bridge events write
to cairn-store's event log via `EventBridge`. Two independent channels:

```
cairn-orchestrator                         FF / Valkey
      │                                          │
      ├── OrchestratorEventEmitter ──→ cairn-store.projections (SSE, read paths)
      │
      └── TaskFrameSink (optional)   ──→ ff:exec:{p:N}:attempt:N:stream (restore_frames)
```

If the FF stream write fails, the run continues using the bridge-event
projections. If the bridge write fails, the stream-frame replay is
still intact. The two paths intersect only at authoritative FF state
(exec_core, flow_core, etc.) — not via shared cairn state.

### 4.3 Approval suspension handoff

When the loop returns `LoopTermination::WaitingApproval { approval_id }`,
the caller (HTTP handler) decides which suspension primitive to fire:

- **Service-level path**: call `state.runtime.runs.enter_waiting_approval(&run_id)`.
  This is the canonical path today — it works whether the caller holds
  a `CairnTask` or not. `RunService` under the Fabric adapter routes to
  `FabricRunService::enter_waiting_approval` which runs
  `ff_suspend_execution`.
- **Task-level path** (preferred when the caller holds a live
  `CairnTask`): call `cairn_task.suspend_for_approval(&approval_id, None)`.
  This consumes the `CairnTask` and the task stays suspended until
  `ff_deliver_signal` fires on the matching approval waitpoint. Use
  this when the run was claim-active via the orchestrator's own lease
  (the loop was already running inside the task's execution context).

The loop never holds a `CairnTask` across iterations (the consuming
self contract would require consuming it mid-iteration, which the
state machine doesn't allow). So the decision of which primitive to
fire is a CALLER concern, not an orchestrator concern. The orchestrator
returns the `approval_id` and exits; the caller knows which path it's on.

### 4.4 Lease-health gate semantics

`task_sink.is_lease_healthy()` polls ff-sdk's 3-consecutive-renewal
threshold. On false, the loop returns
`LoopTermination::Failed { reason: "lease unhealthy" }` BEFORE any
expensive side effect (next iteration's gather / decide / execute).
This is:

- **Correct**: FF rejects every fcall on an unhealthy lease with
  `stale_lease` anyway; bailing early saves work and avoids partial
  state.
- **Cheap**: polling is a struct-field read (ff-sdk tracks the
  renewal counter internally). No network round-trip.
- **Caller-observable**: the handler sees `Failed { reason: "lease
  unhealthy" }` and can call `cairn_task.fail_with_retry(...)` to let
  FF's retry policy decide whether to reschedule or terminal-fail.

### 4.5 Checkpoint-frame failure semantics (design nuance)

Stream-frame writes are intentionally best-effort — the loop runner
logs and continues on every frame-append failure, including
`save_checkpoint`. This matches the existing policy for
`CheckpointHook::save()` (pre-PR #29 behavior) and keeps both channels
consistent.

A lost checkpoint frame means cross-process resumption via
`restore_frames()` silently misses that iteration's state. In
practice this is masked by:

1. The cairn-side `CheckpointHook::save()` call (still running in
   parallel), which persists the checkpoint to cairn-store's event
   log via the injected hook.
2. FF's own lease renewal — a lost frame on a healthy lease means
   the next iteration's checkpoint catches up; a lost frame on an
   unhealthy lease triggers the lease-health gate (§4.4) which
   terminates the loop cleanly.

If we later want checkpoint-frame failure to be fatal (e.g. a
recovery-first mode where cross-process resume is load-bearing), the
trait can gain a `save_checkpoint_fatal -> Result<(), Fatal>` variant
at the sink layer; the loop-runner check becomes strict. That's a
future design call — today's contract is **advisory**, matching the
cairn-side hook.

### 4.6 Dep graph

`cairn-orchestrator` now depends directly on `cairn-fabric`
(`[dependencies]`). The blanket `TaskFrameSink` impl for `CairnTask`
lives in cairn-orchestrator so cairn-fabric stays unaware of the
orchestrator trait; no cycle. The orchestrator → stream integration
test (`crates/cairn-fabric/tests/integration/test_orchestrator_stream.rs`)
adds cairn-orchestrator as a `[dev-dependencies]` of cairn-fabric to
name the trait from the test; this is a dev-only cycle that Cargo
isolates from the library graph.

---

## 5. Outstanding FF asks

Filed with FF maintainers; tracked for re-wiring when they land.

1. **`pub fn FlowFabricWorker::claim_from_grant(grant: ClaimGrant) -> Result<ClaimedTask, SdkError>`** — required to route cairn's production worker path through `ff-scheduler` without enabling `insecure-direct-claim`. Today `ClaimedTask::new` is `pub(crate)` on ff-sdk, so cairn can obtain a `ClaimGrant` from the scheduler but cannot turn it into a `ClaimedTask` for the stream / renewal / terminal methods.
2. **`pub fn parse_report_usage_result(raw: &Value) -> Result<ReportUsageResult, SdkError>`** — ff-sdk's parser is private, so cairn-fabric re-implements it (`services/budget_service.rs::parse_spend_result`) and keeps it in sync by hand. Exposing the upstream parser eliminates a drift hazard.
3. **Scheduler-mediated `claim_resumed_execution`** — same visibility problem as #1 for the resume-after-suspend path. Needs a `pub` entry point so cairn can consume a scheduler grant for a previously-suspended execution.

---

## 6. Known gaps

Accepted limitations as of finalization — each has a follow-up round scoped.

- **`FabricTaskService::declare_dependency` + `check_dependencies` return errors / empty vecs** (`services/task_service.rs:352-370`). The plan is FF flow-edge FCALLs (`ff_stage_dependency_edge`, `ff_apply_dependency_to_child`) wrapped by Fabric. Blocked on Session→Flow DAG work (Phase 3). Handler `add_task_dependency_handler` must stay on the store-event path until then; adapter routes accordingly.
- **`FabricTaskService::list_expired_leases`** returns empty (FF's `LeaseExpiryScanner` handles expiry server-side). The admin handler `expire_task_leases_handler` is delegated via the adapter to the cairn-store projection — see `crates/cairn-app/src/fabric_adapter.rs:556-565` for the `TaskReadModel::list_expired_leases` fallback path.
- **`FabricTaskService::list_by_state`** and **`FabricSessionService::list`** return empty — FF doesn't index by cairn's `(tenant, workspace, project)` scope. Adapter delegates to `TaskReadModel` / `SessionReadModel` projections. Not a defect; it's the read-path split.
- **Provider budgets and tenant quotas** (`handlers/providers.rs` + `handlers/admin.rs`) remain on the legacy `BudgetServiceImpl` / `QuotaServiceImpl` over the cairn event log. `FabricBudgetService` and `FabricQuotaService` are per-run admission controls, not tenant-wide ceilings; they ride alongside as new surfaces, not replacements.
- **`scripts/smoke-test.sh` HTTP harness historically assumed a permissive state machine** (the in-memory `RunServiceImpl` allows `pause` without a prior `claim`, etc). In the default Fabric build, FF enforces strict state transitions — a run must be claimed and active before it can be paused/resumed, tasks must be submitted-and-claimed before lease operations. 8 smoke sections (`POST /v1/runs/:id/pause`, `/resume`, `POST /v1/tasks/:id/claim`, `/release-lease`) returned HTTP 500 on the fabric path. These are **NOT runtime defects** — FF's strictness is correct production behaviour. PR #27 lands a short-lived worker-loop fixture (claim-then-operate) in `crates/cairn-app/src/bin/smoke_worker.rs` that exercises FF semantics correctly.
  - In-memory path (`--features in-memory-runtime`): 97/97 pass, 4 skipped.
  - Fabric path (default build + Valkey + FF lua loaded): 89/97 pass on the
    pre-PR#27 script (8 harness gaps); the PR #27 worker-loop fixture
    closes those gaps.

---

## 7. Known tech debt (flagged during finalization-round audit)

Severity ordered. Each item has a follow-up round scoped.

### ✅ CLOSED: HIGH — Event emission gate bug

*Status*: fixed on `feat/task-emission-gate-fix`.

Was: `FabricTaskService::complete` / `fail` / `cancel` gated `BridgeEvent::TaskStateChanged` emission on `ActiveTaskRegistry` membership, so tasks claimed via any path other than `FabricTaskService::claim` (external API callers, `CairnWorker::claim_next` under `insecure-direct-claim`, cross-process claim/complete) silently skipped projection emission and drifted the cairn-store `TaskReadModel` from FF's exec_core.

Now: emission is unconditional after FF confirms the terminal transition. Projections are idempotent on `(task_id, event_id)`, so a redundant emit from a parallel path is a no-op re-write. Regression guards land in `crates/cairn-fabric/tests/integration/test_event_emission.rs` — four tests exercising the happy-path baseline plus complete/fail/cancel via a fresh `FabricTaskService` (simulating cross-process claim + terminal).

### ✅ CLOSED: MEDIUM — `ActiveTaskRegistry` duplicates FF-owned state

*Status*: deleted on `feat/task-emission-gate-fix`.

Was: `DashMap<TaskId, {execution_id, lease_id, lease_epoch, attempt_index, Option<ClaimedTask>}>` — a cairn-side latency cache for fields FF already owned, and the mechanism the emission-gate bug rode through.

Now: entire `active_tasks.rs` module removed. `FabricTaskService::resolve_active_lease` + `resolve_lease_or_placeholder` always HGETALL `exec_core`; `CairnTask` carries its `ClaimedTask` directly (no registry intermediary). `FabricServices` no longer exposes a `pub registry` field. Net deletion ≈ 250 LOC.

### MEDIUM — Bridge-event completeness audit

The finalization-round smoke-test caught a single-emit gap: `FabricSessionService::create` did not emit a `BridgeEvent::SessionCreated`, so sessions were never written to the cairn-store projection, and every subsequent handler that read `sessions.get(session_id)` returned `None`. Fixed in commit 4 (see `event_bridge.rs` + `services/session_service.rs`).

The same class of gap could exist for any FF mutation path cairn-fabric exposes. A systematic audit should walk every public method on `FabricRunService` / `FabricTaskService` / `FabricSessionService` / `FabricBudgetService` / `FabricQuotaService` and verify: every mutation that changes FF state that cairn-app reads back from the projection has a corresponding `BridgeEvent` emitted. No cairn-app read-path should depend on a state change that only lives in Valkey.

Bugs of this shape are invisible to unit tests (services write to FF correctly) and to live FF integration tests (each one tests the fabric layer in isolation, not the full handler → adapter → fabric → store-projection chain). The integration-readiness gate is cairn-app smoke — run `scripts/smoke-test.sh` (default build) on every cairn-fabric mutation-surface change.

### LOW — Smoke-test harness rewrite for Fabric state machine

`scripts/smoke-test.sh` historically simulated in-memory permissive state transitions (pause without prior claim, etc.). In the default Fabric build, FF enforces strict state transitions, so 8 sections (pause/resume/claim/release-lease on freshly created runs and tasks) returned HTTP 500 — the runtime is correct, the test's expectations weren't. PR #27 replaces those sections with a worker-loop fixture (`crates/cairn-app/src/bin/smoke_worker.rs`) that claims before operating so the fabric path gets exercised end-to-end.

### LOW — CairnTask tag micro-cache

*Location*: `crates/cairn-fabric/src/worker_sdk.rs:174-180`.

`CairnTask` caches `run_id`, `session_id`, `project` extracted from FF exec_core tags at claim time. Struct-lifetime only, not persistent. Re-reading tags on every terminal call adds an HGET per call but eliminates a staleness risk if operator-directed reassignment ever enters scope. Keep as-is until that feature arrives.

### LOW — `TaskLeaseClaimed.lease_expires_at_ms` is a snapshot

*Location*: `crates/cairn-fabric/src/event_bridge.rs:56-62`.

The `TaskLeaseClaimed` bridge event payload carries `lease_expires_at_ms`, which is also stored on FF's `exec_core` as `current_lease_expires_at`. This is a moment-in-time snapshot for projection display, not a cached field with invalidation semantics — consumers must not treat the value as tracked (FF can extend / renew the lease after the event is emitted). Documented here so future readers don't mistake it for live state.

### LOW — `list_child_runs` silently truncates at 10k events

*Location*: `crates/cairn-app/src/fabric_adapter.rs` (`FabricRunServiceAdapter::list_child_runs`) and `crates/cairn-runtime/src/services/run_impl.rs` (`RunServiceImpl::list_child_runs`).

Both implementations scan up to 10,000 `EventLog` events and truncate silently if the parent run's `RunCreated` is older than that window. At high event rates this could miss child runs. `RunReadModel` has no `list_by_parent_run` projection index today, so the scan is the only option. Pre-existing across both paths — not introduced by finalization — flagged for a future round that adds the projection index (then both impls delegate to a single indexed read with no truncation).

## 8. Versioning

- Pinned FF rev: `a09871000574388256b1dd7c910239e992c0d3a6` (in every `crates/cairn-fabric/Cargo.toml` `rev = …` entry).
- Cairn-fabric crate version: `0.1.0` (unpublished, `publish = false`).
- Bumping the FF rev requires the `scripts/run-fabric-integration-tests.sh` `FF_REV` env var to match. The script fails fast on mismatch.
