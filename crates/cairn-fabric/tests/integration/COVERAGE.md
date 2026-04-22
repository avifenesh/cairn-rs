# cairn-fabric integration test coverage

Audit of `crates/cairn-fabric/tests/integration/` against the 17 functions
declared in `src/fcall/mod.rs::CRITICAL_CONTRACTS`.

**Last refresh: 2026-04-22 — Phase D PR 2 coverage gate.**

- Live test files: 14 (`test_budget.rs`, `test_checkpoint.rs`,
  `test_engine.rs`, `test_event_emission.rs`, `test_heartbeat.rs`,
  `test_instance_tag_filter.rs`, `test_lease_history_subscriber.rs`,
  `test_orchestrator_stream.rs`, `test_run_lifecycle.rs`,
  `test_scanner_filter_perf.rs`, `test_scanner_filter_upstream.rs`,
  `test_session.rs`, `test_suspension.rs`, `test_task_dependencies.rs`).
- FCALL functions exercised by at least one integration test: **15 / 17**.
- FCALL functions with **zero** integration coverage: **2 / 17**
  (`ff_check_admission_and_record`, `ff_create_quota_policy` — both in
  `quota_service`, out of scope for Phase D PR 2).

### Coverage deltas from the original audit

| FCALL | Status | Test file(s) |
|-------|--------|--------------|
| `ff_suspend_execution` | **LIVE** | `test_suspension.rs::test_suspend_and_resume_roundtrip`, `::test_signal_delivery_resumes_waiter`, `::test_enter_approval_after_prior_approval_creates_fresh_waitpoint`, `::task_pause_and_resume_emit_state_changed` |
| `ff_resume_execution` | **LIVE** | `test_suspension.rs::test_suspend_and_resume_roundtrip`, `::task_pause_and_resume_emit_state_changed` |
| `ff_deliver_signal` | **LIVE** | `test_suspension.rs::test_signal_delivery_resumes_waiter`, `::test_signal_delivery_is_idempotent` |
| `ff_renew_lease` | **LIVE** | `test_heartbeat.rs::test_heartbeat_extends_lease_expiry`, `::test_heartbeat_with_stale_epoch_is_rejected` |
| `ff_create_flow` | **LIVE** | `test_session.rs::test_session_create_and_cancel_flow` |
| `ff_cancel_flow` | **LIVE** | `test_session.rs::test_session_create_and_cancel_flow`, `::test_double_archive_is_idempotent` |
| `ff_check_admission_and_record` | **MISSING** | `quota_service` — out of Phase D PR 2 scope |
| `ff_create_quota_policy` | **MISSING** | `quota_service` — out of Phase D PR 2 scope |

The §2 / §4 tables below preserve the original audit for historical
context. Anything marked LIVE above supersedes the §2/§4 "No" entries.

### Original audit (frozen; superseded where annotated above)

- Existing tests: **9** across `test_run_lifecycle.rs` + `test_budget.rs`
- FCALL functions exercised (at least once): **9 / 17 = 53%**
- FCALL functions with **zero** integration coverage: **8 / 17 = 47%**

Static-builder unit tests in `src/fcall/mod.rs` verify KEYS/ARGS counts for all
17 functions. That guarantees the wire shape we send matches the contract, but
it does **not** guarantee the Lua on the server side behaves as we expect.
Live tests are the only thing that closes that gap.

---

## 1. Existing tests

| # | Test (file) | What it exercises (service → fabric op) | FCALL functions hit |
|---|-------------|------------------------------------------|---------------------|
| 1 | `test_create_and_read_run` (`test_run_lifecycle.rs`) | `runs.start` then `runs.get`; asserts ProjectKey round-trip. | `ff_create_execution` |
| 2 | `test_tags_readable` (`test_run_lifecycle.rs`) | `runs.start`; asserts `session_id` persists on the record. Overlaps #1 but tightens the session-id assertion. | `ff_create_execution` |
| 3 | `test_duplicate_start_is_idempotent` (`test_run_lifecycle.rs`) | `runs.start` twice with same `run_id`; asserts same state/session/project. | `ff_create_execution` (idempotency path only) |
| 4 | `test_complete_task` (`test_run_lifecycle.rs`) | `tasks.submit` → `tasks.claim` → `tasks.complete`; asserts final state `Completed`. | `ff_create_execution`, `ff_issue_claim_grant`, `ff_claim_execution`, `ff_complete_execution` |
| 5 | `test_fail_task_terminal` (`test_run_lifecycle.rs`) | `tasks.submit` → `tasks.claim` → `tasks.fail(ExecutionError)`; asserts state is `Queued` (retry) or `Failed` (terminal). | `ff_create_execution`, `ff_issue_claim_grant`, `ff_claim_execution`, `ff_fail_execution` |
| 6 | `test_cancel_task` (`test_run_lifecycle.rs`) | `tasks.submit` → `tasks.claim` → `tasks.cancel`; asserts state `Canceled`. | `ff_create_execution`, `ff_issue_claim_grant`, `ff_claim_execution`, `ff_cancel_execution` |
| 7 | `test_budget_hard_limit` (`test_budget.rs`) | `budgets.create_run_budget` → `record_spend(50)` → `record_spend(60)`; asserts `HardBreach{dimension="tokens"}`. | `ff_create_budget`, `ff_report_usage_and_check` |
| 8 | `test_budget_status_reflects_spend` (`test_budget.rs`) | `create_run_budget` → `record_spend(42)` → `get_budget_status`; asserts usage + hard_limits are readable. | `ff_create_budget`, `ff_report_usage_and_check` (status is plain `HGETALL`, not an FCALL) |
| 9 | `test_budget_release_resets_usage` (`test_budget.rs`) | `create_run_budget` → `record_spend(90)` → `release_budget` → `record_spend(50)` returns `Ok`. | `ff_create_budget`, `ff_report_usage_and_check`, `ff_reset_budget` |

**Aggregate FCALL footprint (covered):**
`ff_create_execution`, `ff_complete_execution`, `ff_fail_execution`,
`ff_cancel_execution`, `ff_issue_claim_grant`, `ff_claim_execution`,
`ff_create_budget`, `ff_report_usage_and_check`, `ff_reset_budget` — 9/17.

---

## 2. FCALL functions with no integration coverage

Ordered by shipping risk.

| FCALL | Risk if it regresses in production | Callsite in cairn-fabric |
|-------|------------------------------------|--------------------------|
| `ff_suspend_execution` | Approvals, human-in-the-loop, timers, signal waits — all broken. Affects every agent pause path (RFC 022). | `run_service::pause` (line 726), `run_service::enter_waiting_approval` (line 955), `task_service::pause` (line 1028). |
| `ff_resume_execution` | Paired with suspend; a bug here silently strands suspended runs. Twin callers in runs and tasks, and also the approval-resolve path. | `run_service::resume` (line 810), `task_service::resume` (line 1090). |
| `ff_deliver_signal` | Signal routing to suspended executions (approval responses, webhook-triggered resumes). Regression = approvals never unblock. | `run_service::resolve_approval` (line 1057). |
| `ff_renew_lease` | Worker heartbeat. Regression = long-running tasks lose their lease mid-execution and get stolen — silent double-execution. | `task_service::heartbeat` (line 561). |
| `ff_check_admission_and_record` | Multi-tenant admission control. Regression = either noisy-neighbor escapes (overspend) or legitimate traffic denied. | `quota_service::check_admission` (line 132), `check_admission_for_run` (line 169). |
| `ff_create_quota_policy` | Quota setup prerequisite. If this regresses, nothing else in the quota path matters. | `quota_service::create_quota_policy` (line 28), `create_tenant_quota`, `create_workspace_quota`, `create_user_quota`. |
| `ff_create_flow` | Session creation. Regression = no session = no runs = platform down. | `session_service::create` (line 115). |
| `ff_cancel_flow` | Session cancel / archive. Lower severity; sessions can leak rather than crash, but multi-tenant cleanup depends on it. | `session_service::archive` (line 200). |

---

## 3. Edge-case gaps in already-covered FCALLs

Even where an FCALL is exercised once, the existing suite never probes the
harder branches.

| Scenario | Currently tested? | Gap |
|----------|-------------------|-----|
| **Lease timeout during claim** — two workers race to claim the same task; the loser gets a typed error, not a silent overwrite. | No | No test calls `claim` from two workers, and no test lets a lease expire mid-work. |
| **Double-complete** — `complete` invoked twice on the same task. Expected: idempotent success or a typed "already terminal" error. | No | `test_complete_task` completes once and stops. Double-call behavior (success? error? panic on FF side?) is unknown against live FF. |
| **Double-cancel / complete-after-cancel** | No | Same gap: terminal transition semantics beyond first call are untested. |
| **Retry backoff observability** — `test_fail_task_terminal` accepts *either* `Queued` or `Failed`. That's a shrug: if the retry path broke and failures went terminal on attempt 1, the test would still pass. | Partial / weak | Need a test that pins `max_retries=2`, asserts `Queued` after 1st failure, then fails again and asserts `Failed`. Should also assert lease is released between retries. |
| **Budget reset cycle** — budget resets over a rolling window (not just one-shot release). `test_budget_release_resets_usage` only exercises the release branch. | No | `ff_reset_budget` with the `resets_zset` key (see `budget_service::release_budget`) implies scheduled resets; need a test that writes a reset into the zset and verifies usage clears at the next spend without explicit release. |
| **Suspend → timeout → escalate** | No | `run_service::pause` / `enter_waiting_approval` set a `TimeoutBehavior::Escalate` path. No integration test triggers timeout-then-escalate. |
| **`already_satisfied` on suspend** — if the waitpoint is already satisfied when the suspend call lands, the FF contract returns `already_satisfied` and the caller must NOT wait. | No | No test creates a pre-satisfied waitpoint, so this branch is only covered by unit-level faith. |
| **Duplicate submit** — same `TaskId` submitted twice; we have `test_duplicate_start_is_idempotent` for runs, nothing for tasks. | No | Tasks have a different ID derivation path (see `task_service::task_to_execution_id`); duplicate submit idempotency is unverified end-to-end. |
| **Duplicate signal delivery** — `ff_deliver_signal` is dedup'd server-side on `signal_dedup_ttl_ms`; we need to send the same signal twice and assert only one resume. | No | `ff_deliver_signal` has zero tests at all. |
| **Signal delivered to unknown waitpoint** | No | Expected: no-op or typed error. Wire semantics unverified. |
| **Heartbeat after lease-steal** — `ff_renew_lease` must reject a heartbeat whose `lease_epoch` has been superseded. | No | `ff_renew_lease` has zero tests. |
| **Admission at hard cap** — quota admits until limit, then denies with the right reason. | No | `ff_check_admission_and_record` has zero tests. |
| **Cross-tenant collision** — two tenants with identical-string IDs should share nothing. `ProjectKey` partitioning claim needs a live check. | No | All tests use the single `test_tenant/test_workspace/test_project` scope. |
| **TestHarness teardown** — `TestHarness::teardown` calls `fabric.shutdown()` but does **not** delete keys. Keys accumulate across runs. The header comment in `test_run_lifecycle.rs` admits this. | Acknowledged, not fixed | Either add `FLUSHDB` in teardown (drastic — breaks `--keep-valkey` debugging) or scope every test under a unique namespace. |

---

## 4. Top 5 tests to add before shipping to prod

Ranked by shipping risk × blast radius. Each bullet is one new test file or
one new `#[tokio::test] #[ignore]` in an existing file.

### **#1 — `test_suspend_and_resume_roundtrip` (new file `test_suspension.rs`)**
**Covers:** `ff_suspend_execution`, `ff_resume_execution`.
**Shape:** submit + claim a task → `tasks.pause(...)` with a waitpoint →
assert state is `Suspended` and suspension record is readable → `tasks.resume(...)` →
assert state is back to `Active` (or `Queued` depending on the waitpoint contract) →
complete.
**Why #1:** Suspension is the spine of the approval / timer / signal path. Two distinct FCALL functions both uncovered; their contract is paired and the bug surface is large (waitpoint keys, timeout behavior enum, suspension index).

### **#2 — `test_signal_delivery_resumes_waiter` (new file `test_suspension.rs`)**
**Covers:** `ff_deliver_signal` + the consumer side of `ff_suspend_execution`.
**Shape:** submit + claim → `enter_waiting_approval(...)` → from a second handle, `resolve_approval(signal_payload)` → assert the run state transitions to `Approved` (or equivalent) and a second `resolve_approval` with the same `signal_id` is a no-op (dedup).
**Why #2:** `ff_deliver_signal` is the ONLY way an external caller unblocks a suspended run. If delivery or dedup regresses, every human-approval flow silently hangs. Also exercises `already_satisfied` if we pre-resolve before enter.

### **#3 — `test_heartbeat_extends_lease_and_rejects_stale_epoch` (add to `test_run_lifecycle.rs`)**
**Covers:** `ff_renew_lease`.
**Shape:** submit + claim (lease 1s) → `tasks.heartbeat(...)` before expiry, assert success → advance time / wait past the original TTL → assert the task is still Active because the renewal landed → as a second assertion, artificially bump the epoch (or `release_lease` + re-claim) and assert a heartbeat with the old epoch returns a typed "lease-lost" error.
**Why #3:** Without heartbeat integrity, long-running agent tasks lose their lease mid-step and get stolen. That's a silent double-execution in prod — hardest class of bug to diagnose.

### **#4 — `test_admission_passes_until_limit_then_denies` (new file `test_quota.rs`)**
**Covers:** `ff_create_quota_policy`, `ff_check_admission_and_record`.
**Shape:** `quotas.create_tenant_quota(...)` with cap=3 → `check_admission` × 3 all return admitted → 4th returns denied with `reason="cap_exceeded"` (or contract-defined value) → advance time past window → 5th admits again.
**Why #4:** Two entire FCALLs covered in one cohesive flow, and this is the only line of defense against noisy-neighbor tenants. If admission regresses, a single tenant can exhaust shared capacity.

### **#5 — `test_session_create_and_cancel_flow` (new file `test_session.rs`)**
**Covers:** `ff_create_flow`, `ff_cancel_flow`.
**Shape:** `sessions.create(...)` → assert session is readable and listed → `sessions.archive(...)` → assert session is no longer listed (or marked archived, per contract) → second `archive` is idempotent.
**Why #5:** Sessions are the root of the entity hierarchy (session → run → task). A create-flow regression is platform-down. A cancel-flow regression leaks tenant data across workspace boundaries. Low complexity, high coverage win.

---

## 5. Suggested sixth-and-beyond (for reference, not prioritized here)

- `test_budget_reset_schedule` — exercise the `resets_zset` branch of
  `ff_reset_budget` (scheduled reset, not manual release).
- `test_double_complete_is_idempotent_or_typed_error` — pin the terminal-transition contract.
- `test_fail_retry_then_terminal` — tighten `test_fail_task_terminal` by pinning the retry/terminal boundary.
- `test_cross_tenant_isolation` — two `ProjectKey`s with identical string
  segments but different tenant IDs must not collide.
- `test_claim_race_between_two_workers` — second claim on an already-leased
  task returns typed error, not silent overwrite.

---

## 6. Harness observations (out of scope for test-writing, but worth a follow-up)

- `TestHarness::teardown` does not purge keys. Re-running the suite against
  the same Valkey accumulates state. The runner script's container-reuse is
  safe on a fresh container; it's NOT safe across re-runs with
  `--keep-valkey`. Either (a) `FLUSHDB` in `setup` before each test or
  (b) prefix every generated ID with a per-run uuid (already done for
  run/task ids — extend to budget_id / session_id).
- All tests use `ProjectKey::new("test_tenant", "test_workspace", "test_project")`.
  Good enough for now; multi-tenant tests (#4, cross-tenant isolation) should
  vary this deliberately.
- `CAIRN_TEST_VALKEY_URL` parsing in `TestHarness::setup` does not fail loudly
  on bad input — `unwrap_or(...)` swallows host/port fallback. If the env var
  is set to something malformed, tests run against localhost silently. Minor,
  but a future test for bad-config error surface would be nice.
