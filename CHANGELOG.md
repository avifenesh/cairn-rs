# Changelog

All notable changes to cairn-rs are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Fixed

- **Cross-instance event leak in `LeaseHistorySubscriber`.** Two
  cairn-app instances sharing a Valkey previously saw each other's
  lease-expiry / lease-reclaim frames in their own `/v1/events` stream
  — `ff:idx:{fp:N}:lease_expiry` ZSETs are partition-global, not
  cairn-scoped, so the subscriber enumerated every cairn instance's
  leased executions on each partition and dispatched foreign frames
  into the local event log. Now every cairn execution carries a
  `cairn.instance_id` tag at create time and the subscriber drops any
  frame whose tag doesn't match `FabricConfig::worker_instance_id`.
  Fixes the `test_rfc020_recovery::clean_crash_recovery_restores_non_terminal_runs`
  flake (task #185) and the production cross-tenant leak. Docs:
  `docs/operations/cross-instance-isolation.md`. (#106)

- **`RoutePolicy.enabled` now plumbed through PG + SQLite projections.**
  The field was accepted on the wire and persisted in the event log, but
  both projection writers silently dropped it, so `GET /v1/providers/policies`
  always returned `enabled = true`. Adds the column to both backends and
  backfills existing rows. (#108)

- **`POST /v1/decisions/evaluate` added to `http_routes.tsv`.** Route
  handler existed and was reachable in production, but the compatibility
  catalogue did not list it, so the drift check could not detect
  regressions. Gap surfaced by #192. (#105)

- **Session projection read-after-write race closed for RFC 020 test
  #11.** `RecoverySummary` could be emitted before the session projection
  saw the preceding terminal transition, causing the compliance test to
  observe a non-terminal run during recovery enumeration. Recovery now
  reads from the authoritative projection head. (#100)

- **Per-harness sandbox base dir isolated.** Multiple `LiveHarness`
  instances in the same test binary previously shared the same sandbox
  base, producing flaky `SandboxBaseRevisionDrift` emissions when tests
  ran in parallel. Each harness now derives its own base path. (#99)

### Added

- **9-table SQLite port (option B parity).** Ports `tenants`,
  `workspaces`, `projects`, `workspace_members`, `prompt_assets`,
  `prompt_versions`, `prompt_releases`, `route_decisions`, and
  `provider_calls` to the SQLite backend so team-mode deployments on
  single-node hardware can run without Postgres. Schema-parity check
  (`cargo test -p cairn-store --test schema_parity`) now passes for
  these tables. (#102)

- **`route_policies` ported to SQLite.** Completes option B parity
  for the 10-table block tracked by the schema-parity check. (#104)

- **Prompt schema hardened symmetrically in PG + SQLite.** New PG
  migration `V023` and parallel SQLite DDL tighten `prompt_assets` /
  `prompt_releases` FK and NOT-NULL constraints so both backends reject
  the same invalid inputs. (#103)

- **CI `--tests` allow-list extended to close silent coverage gap.**
  Integration tests in crates not previously in the allow-list (notably
  `cairn-api`) were never executed in CI. Audited the workspace for
  `tests/` directories and added every crate with non-empty integration
  coverage. Contributors adding a new `tests/` directory must extend
  the allow-list in the same PR. (#107)

- **`CAIRN_BACKFILL_INSTANCE_TAG=1`** — one-shot boot-time backfill
  that stamps `cairn.instance_id` onto every pre-existing exec-tag
  hash in Valkey that lacks it but carries `cairn.project`. Needed
  only for operators doing an in-place binary swap with `Running` /
  `WaitingApproval` executions that predate the filter; default off
  on fresh deploys. Idempotent across boots — a second pass is a
  no-op.

#### Durability — RFC 020 Tracks 1–4

- **Track 2 readiness gate** — `GET /health/ready` returns `503` with a
  per-branch progress JSON while recovery is in flight, and flips to
  `200` once every branch reports `complete`. Liveness (`/health`) stays
  `200` throughout so orchestrators can keep the process running across
  long replays. Shape documented in `docs/operations/rfc020-recovery.md`.
  (#73)
- **Track 1 RecoveryService** — startup pass enumerates non-terminal runs,
  applies the RFC 020 recovery matrix, and emits `RecoveryAttempted` /
  `RecoveryCompleted` events before readiness flips to `200`. Closes
  durability invariants 3 (non-terminal runs recovered before readiness)
  and 4 (recovery is idempotent). (#75)
- **Track 3 tool-call idempotency** — deterministic `ToolCallId` derivation,
  `ToolCallResultCache` projection consulted on every dispatch,
  `RetrySafety` three-tier enforcement (`IdempotentSafe` /
  `AuthorResponsible` / `DangerousPause`), and batched tool-event append
  (atomic `ToolInvocationRequested` + `ToolInvocationCompleted`). Closes
  invariants 6 (tool results cached) and 11 (batched append). (#82)
- **Track 4 dual checkpoint per iteration** — `Intent` checkpoint before
  tool dispatch and `Result` checkpoint after, plus `RecoverySummary`
  emitted once per boot and `DecisionCacheWarmup` event at startup. Closes
  invariant 5 (two checkpoints per iteration). (#84)

#### Sandbox recovery tripwires

- **`SandboxLost` emission** on recovery when the sandbox directory is
  missing on disk. Un-ignores RFC 020 compliance test #4. (#83)
- **`SandboxAllowlistRevoked` emission** on recovery when a sandbox's
  origin repo has been dropped from the project access allowlist. Un-ignores
  compliance test #3a. (#86)
- **`SandboxBaseRevisionDrift` emission** on recovery when an overlay
  sandbox's upper-layer base-revision has drifted from the clone cache's
  current `HEAD`. Un-ignores compliance test #3b. (#89)
- **Sandbox reattach test hook** — debug-gated `CAIRN_TEST_SEED_*`
  environment hook exercises the overlay reattach path end-to-end against
  a real sandbox fixture. Un-ignores compliance test #3. (#88)
- **RFC 020 sandbox recovery compliance tests #3 / #3a / #3b / #4**
  landed initially as tripwire `#[ignore]`d tests; each subsequent
  emission PR flips one of them live. (#80)

#### Decision cache durability

- **Decision cache persistence via event log + startup replay** — cached
  decisions survive a restart without re-approval. Closes invariant 9
  (decisions survive) and un-ignores compliance test #7. (#85)

#### Test infrastructure

- **LiveHarness SIGKILL + restart** — `sigkill()`, `restart()`, and
  `sigkill_and_restart()` helpers plus `setup_with_sqlite()` for
  durable-state-across-restart integration tests. Required fixture for
  every RFC 020 Track-3/4 compliance test. (#74)
- **Schema parity check between Postgres and SQLite** — new
  `cargo test -p cairn-store --test schema_parity` enumerates
  `CREATE TABLE` statements from both backends and asserts the table
  sets match. Currently ignored with 10 Postgres-only tables surfaced;
  will become a fail-on-merge gate when the gap closes. (#76)
- **RFC 020 compliance tests #7 (decision cache) and #12 (Postgres-only
  team mode)** as independent integration tests against a live
  cairn-app subprocess. (#77)
- **recovery_e2e migration to LiveHarness — PR 1 of 3**: promotes tests
  #6 (in-flight approval) and #11 (RecoverySummary emitted) from mocked
  unit tests to real SIGKILL-and-restart integration tests; deletes three
  unit-mocked tests whose contracts are now covered by the live suite.
  (#81)
- **recovery_e2e migration — PR 2 of 3**: deletes four additional
  Track-3-duplicated mocked tests whose coverage now lives in the Track 3
  LiveHarness suite. (#87)
- **Provider contract test against real OpenRouter** — live-provider
  chat-completion contract test against an OpenRouter free-tier model,
  gated on `OPENROUTER_API_KEY` so CI without the key skips cleanly. (#90)
- **OpenRouter fixture refresh** — refreshes the recorded fixture against
  the real API and swaps to a stable free-tier model so the offline path
  stays accurate. (#91)
- **Real-LLM soak test ladder against OpenRouter MiniMax** — 5-minute
  (#92), 30-minute (#98), and 1-hour (#101) live-provider soaks against
  the cairn-app subprocess, asserting no lease expiry / event-log drift /
  checkpoint divergence under sustained traffic. All three are gated on
  `OPENROUTER_API_KEY` and skip cleanly in CI without the key.
- **Chaos resilience suite** — SIGSTOP/SIGCONT, failed-append, and
  rapid-restart scenarios exercising cairn-app's durability guarantees
  under adverse conditions. (#95)
- **Reasoning-model response-shape contract test** — asserts that
  providers returning `content: null` with `finish_reason: length`
  (the reasoning-model truncation shape) are surfaced to the orchestrator
  as a typed error rather than an empty-string fallback. (#96)
- **recovery_e2e migration — PR 3 of 3 (post-Track-4 cleanup).**
  Deletes the final batch of Track-4-duplicated mocked tests whose
  coverage now lives in the LiveHarness Track-4 suite. (#97)

#### Operator documentation

- **`docs/operations/rfc020-recovery.md`** — operator-facing guide to
  readiness endpoints, startup sequence, store requirements, durability
  of state across crashes, and runbook entries for recovery situations.
  Summarises RFC 020; RFC is source of truth. (#78)
- **RFC 020 rev 3** — recovery ownership split (FF-owned operational
  state vs. cairn-owned run-level state), 15 gap resolutions, and the
  new durability invariant #12 (storage-transparent durability). (#79)

#### Pre-RFC-020 additions

- **Task dependency declaration now accepts `dependency_kind` and
  `data_passing_ref`.** `POST /v1/tasks/{id}/dependencies` surfaces
  both fields from FF 0.2's flow-edge FCALLs:
  - `dependency_kind` is an enum (today only `success_only`; unknown
    strings return 422 at the JSON extractor).
  - `data_passing_ref` is an opaque caller-supplied string stored on
    the FF edge and forwarded to the downstream task after upstream
    resolution. Cairn never dereferences it; downstream consumers are
    responsible for interpreting the value. Validated at the handler
    (length ≤ 256 bytes, charset `[A-Za-z0-9._:/-]`, empty string
    treated as absent). See `SECURITY.md` for the opaque-string
    contract.

  Existing callers that omit the fields get the previous defaults.
  `GET /v1/tasks/{id}/dependencies` now returns both fields on each
  blocker record.

### Changed

- **Dependency `edge_id` is now deterministic** (UUID-v5 over
  `flow_id || upstream_eid || downstream_eid`) instead of random. The
  replay path (`dependency_already_exists`) can read the staged edge
  directly via `HGETALL fctx.edge(edge_id)` and compare
  `(dependency_kind, data_passing_ref)` against the caller's values:
  identical replay is idempotent 201; a different kind or ref now
  returns **409 `dependency_conflict`** carrying both existing and
  requested values (previously returned 201 and silently kept the
  original). This also makes `BridgeEvent::TaskDependencyAdded`'s
  `edge_id` stable across caller retries, fixing a latent correlation
  gap for consumers of the bridge event stream.
- `TaskDependency` / `TaskDependencyRecord` now carry `dependency_kind`
  and `data_passing_ref` fields. Backward-compatible via
  `#[serde(default)]` so prior event-log records deserialise.
- **Fix**: `GET /v1/tasks/{id}/dependencies` now respects the admin
  token bypass; previously an admin-token call hit an open-coded
  tenant check that always returned 404 regardless of `is_admin`.
  Aligns with `load_task_visible_to_tenant` used by every other
  task-mutation endpoint.

- **FlowFabric bumped to 0.2**: `ff-core`, `ff-sdk`, `ff-engine`, `ff-scheduler`,
  `ff-script`, and `ferriskey` all move from `"0.1"` to `"0.2"`. FF 0.2 is
  behavior-compatible for claim / submit / complete paths — the 32
  cairn-fabric integration tests pass unchanged. The semver break is
  `ScriptError` gaining `#[non_exhaustive]`; cairn never matches
  exhaustively so no source change was required. `ferriskey::Value::BulkString`
  switched its inner type from `Vec<u8>` to `bytes::Bytes`; test fixtures
  in the new rotation service use `.to_vec().into()` accordingly.

- **RFC-011 Phase 2 closure**: per-session runs and tasks co-locate on the
  session's FlowId partition (`{fp:N}` hash tag). Runs are session-bound at
  the `RunService` trait; tasks remain `Option<&SessionId>` at `TaskService`
  to accommodate A2A protocol submissions (which have no session concept).
  The fabric adapter resolves session from the projection on every mutation:
  `TaskRecord.session_id` OR `TaskRecord.parent_run_id → RunRecord.session_id`.
  HTTP handlers no longer redundantly resolve session before calling
  `TaskService` — the adapter is the single source of truth. One exception:
  `create_task_handler` still resolves `parent_task_id → RunRecord.session_id`
  because neither the adapter nor the `TaskCreated` projection writer walks
  that edge, and leaving it out would silently route sub-sub-tasks to the
  solo partition.

### Added

- **`POST /v1/admin/rotate-waitpoint-hmac`** — admin-only endpoint that
  rotates the waitpoint HMAC signing kid across every execution
  partition without a restart. Delegates to FF 0.2's
  `ff_rotate_waitpoint_hmac_secret` FCALL. Request body:
  `{ new_kid, new_secret_hex, grace_ms? }`. Response body:
  `{ rotated, noop, failed[], new_kid }`. Idempotent on the same
  `(new_kid, new_secret_hex)` — replays report `noop` per partition.
  `grace_ms` (default 60_000) is the window in which the previously
  installed kid stays accepted for verification so in-flight
  waitpoints don't fail mid-rotation. Status mapping: 200 on any
  success, 400 on unanimous input-validation failure across all
  partitions (`invalid_kid`, `invalid_secret_hex`, `invalid_grace_ms`,
  `rotation_conflict`), 500 on whole-fleet transport failure, 503 when
  the fabric runtime is absent. See SECURITY.md → "Waitpoint HMAC
  secret rotation" for operator guidance. Closes #114.

- **`debug-endpoints` Cargo feature on `cairn-app`** (OFF by default).
  Enables `GET /v1/admin/debug/partition?kind=<run|task>&id=<id>` for
  RFC-011 co-location diagnostics. **SECURITY: this feature is intended
  for CI/development only.** Production release builds MUST be compiled
  without it. Turning it on adds FF-internal `ExecutionId` and Valkey
  partition-index disclosure (admin-gated) to the HTTP surface —
  information not otherwise reachable except through direct Valkey
  access. See `SECURITY.md` § "Debug endpoints feature" for the full
  threat model.

### Removed (breaking)

- **`in-memory-runtime` cargo feature deleted.** The feature existed as
  an "event-log-only courtesy backing" for `RunService` / `TaskService`
  / `SessionService` when Valkey wasn't available — local tinkering, CI
  escape hatch, some tests. Post the PR #66 FF dependency migration,
  Fabric is authoritative for all runs/tasks/sessions and the in-memory
  impls carried no correctness guarantees; keeping them meant every new
  event shape had to be taught to two runtimes or silently skipped on
  the in-memory side, and ~60 tests asserted behavior that might or
  might not match Fabric without re-testing against live Valkey.

  What goes:
  - `InMemoryServices::{new, with_store, with_fabric}` + `Default` impl
    + the three impl files `{run,task,session}_impl.rs`. The single
    factory `InMemoryServices::with_store_and_core(store, runs, tasks,
    sessions)` is now the only path.
  - 18 gated runtime tests + the orchestrator_e2e test — their
    coverage either already lives in `crates/cairn-store/tests/`
    (projection replay, sqlite adapter) or migrates to Fabric
    integration (see Added below).
  - 4 gated app mutation test files (bootstrap_smoke, e2e_lifecycle,
    full_workspace_suite, provider_lifecycle_e2e) + 19 mutating
    tests inside bootstrap_server.rs.
  - `#[cfg(test)]` modules across 5 tools builtins, quota_impl,
    signal_router_impl, execute_impl, lib.rs, main.rs, telemetry_routes,
    trigger_routes, repo_routes — all of which constructed
    `InMemoryServices::new()` to drive handler tests.
  - 3 feature-gated CI jobs (check feature arm, clippy feature arm,
    integration-tests). CI now runs a single-arm check/clippy/test
    plus the existing fabric-integration job.

  Upgrade path: production builds never enabled the feature, so there
  is no migration. Tests that were gated on `in-memory-runtime` are
  either deleted or ride the new `FakeFabric` read-only fixture under
  `crates/cairn-app/tests/support/`.

### Added

- **`AppState::new_with_runtime` + `AppBootstrap::router_with_injected_runtime`**
  — public constructors that build cairn-app's HTTP surface around a
  caller-provided `InMemoryServices`. Integration-test entry point used
  by the new `FakeFabric` read-only fixture.
- **`AppBootstrap::serve_prebuilt_router`** — serves a pre-built router
  on a listener, bypassing the `Self::router(config)` call inside
  `serve_with_listener` that constructs live Fabric from env.
- **`crates/cairn-app/tests/support/fake_fabric.rs`** — read-only
  stand-in for the production `Fabric{Run,Task,Session}ServiceAdapter`
  trio. Forwards every read method (`get`/`list_by_session`/…) to the
  projection store; returns `RuntimeError::Internal` on every
  mutation. Lets cairn-app handler tests boot `AppState` without a
  live Valkey while keeping the Fabric mutation surface honest.

### Changed

- **Task dependencies migrated to FF flow edges.** `declare_dependency`
  now issues `ff_stage_dependency_edge` + `ff_apply_dependency_to_child`
  on FF's flow partition instead of maintaining a cairn-side
  projection. `check_dependencies` reads live state via
  `ff_evaluate_flow_eligibility` + per-edge HGET on the child's dep
  hash. FF is the single source of truth; the cairn-side
  `TaskDependencyReadModel` trait is deleted.
  - **Breaking behavior (pre-public, no users)**: a failed or
    cancelled prerequisite now auto-skips its dependents
    (`TaskState::Failed` + `FailureClass::DependencyFailed`).
    Previously the dependent would stay `WaitingDependency`
    indefinitely. FF dispatches the skip via the completion listener
    (~RTT × depth) with a reconciler fallback at 15 s intervals.
  - **Breaking**: task dependencies now require both tasks to be in
    the same session. Cross-session and session-less-task declares
    return `Validation` before any FCALL. This matches FF's flow-
    membership contract; cross-flow edges are not representable.
  - **Scope**: edges use FF defaults (`dependency_kind=success_only`,
    `satisfaction_condition=all_required`). `data_passing_ref`
    (auto-copy upstream result to child payload) is not exposed yet
    — follow-up.
  - **Audit**: `RuntimeEvent::TaskDependencyAdded` is still appended
    to the EventLog on each declare, but no cairn projection reads
    it. Callers reconstructing "which deps resolved when" join
    against each prerequisite's `TaskStateChanged(Completed)`.
  - **Engine config**: `FabricRuntime::start` enables
    `CompletionListenerConfig` on the embedded `ff-engine`. Adds a
    third Valkey connection per runtime (main + lease-history tap +
    completion listener); the dedicated RESP3 client SUBSCRIBEs to
    `ff:dag:completions` and dispatches `ff_resolve_dependency`
    FCALLs per terminal transition.

- **RFC-011 Phase 3: `TaskCreated.session_id` / `TaskRecord.session_id`**
  —
  the task → session binding is now persisted directly on the event and
  projection row instead of being derived at resolve-time from
  `parent_run_id → RunRecord.session_id`. This removes a read-model
  round-trip from every task mutation hot path (claim, start, complete,
  heartbeat, release, cancel, fail) and closes the last window where a
  projection-lag parent-run lookup could silently degrade a
  session-scoped task to the solo ExecutionId mint path (wrong Valkey
  hash slot → unexplained Fabric 404).
  - **Schema**: `V021__add_task_session_id.sql` adds a nullable
    `tasks.session_id` column + partial index. Both Postgres and SQLite
    backends use `COALESCE` at insert time to pull the parent run's
    session when the event predates Phase 3 — no data backfill required
    for existing deployments.
  - **Event compat**: `TaskCreated.session_id` is
    `#[serde(default, skip_serializing_if = "Option::is_none")]`, so
    replaying pre-Phase-3 event streams still deserializes. The
    projection's COALESCE fallback handles the `None`-on-event case
    at replay time.
  - **Resolvers**: `resolve_session_for_task_record`,
    `load_task_with_session_for_tenant`, and
    `resolve_task_project_and_session` (fabric adapter) prefer
    `task.session_id` and only walk the parent run when it is `None`.
    The legacy fallback still propagates 500/404 from the Phase-2 fix.

### Added

- **`POST /v1/runs/:id/claim`** — activates a run's FlowFabric execution lease
  so downstream FCALLs (`enter_waiting_approval`, `pause`, signal delivery)
  accept it. NOT idempotent on the Fabric path: re-claiming an already-active
  run fails at FF's grant gate with `execution_not_eligible`. A second claim
  after a suspend/resume cycle dispatches through `ff_claim_resumed_execution`
  and is legitimate.

### Changed

<!--
  Note on "phase-2" nomenclature: "RFC-011 phase-2" refers specifically
  to the *second* mechanical-sweep slice of the FlowFabric co-location
  migration (RFC-011), and is unrelated to `docs/design/phase2-implementation-plan.md`,
  which tracks the separate RFC 015-022 batch. The two "phase 2" labels
  share a number by coincidence only.
-->
- **RFC-011 phase-2 session-scoped execution IDs** — `ExecutionId` for runs
  and tasks now derives from `session_id + run_id/task_id` via UUID-v5
  (`session_run_to_execution_id` / `session_task_to_execution_id`), replacing
  the previous `run_id`/`task_id`-only mints. All runs and tasks within the
  same session now co-locate on the session's `FlowId` Valkey partition,
  satisfying RFC-011's `{fp:N}:<uuid>` hash-tag invariant. **Breaking change,
  flag-day cutover:** any existing execution records in Valkey mint under
  the old scheme and will be unreachable post-upgrade. **Operator action
  required:** drain all in-flight runs and flush the FF Valkey namespace
  before deploying. Trait signatures on `RunService` / `TaskService` now
  thread `session_id` through all mutation methods (`claim`, `complete`,
  `fail`, `cancel`, `pause`, `resume`, `heartbeat`); `TaskService::submit`
  gains a trailing `session_id: Option<&SessionId>` parameter. `BridgeEvent::TaskCreated`
  gains `session_id: SessionId`. HTTP handlers resolve `session_id` from
  the store projection (task → parent run → session) on each call; no new
  round-trips in steady state (the HGETALL already carries the tag).

  **Migration procedure:**

  1. Stop accepting new runs (set the gateway to 503 or drain at the LB).
  2. Wait for in-flight runs to reach a terminal state (`Completed`,
     `Failed`, `Cancelled`). Monitor via `GET /v1/runs?state=running`.
  3. Flush the FF Valkey namespace: `redis-cli -n <db> FLUSHDB` against
     the Fabric Valkey instance. The event log (Postgres/SQLite) is
     authoritative and untouched — only the FF execution-state cache is
     invalidated.
  4. Deploy the new binary.
  5. Resume traffic.

  **Rollback:** revert the binary. The old scheme's execution IDs are
  deterministic from `run_id` alone, so a post-rollback Valkey is still
  reachable from the old code path. Any new runs created *after* the
  upgrade will have execution IDs derived from `session_id + run_id` and
  will be dead-lettered by the old binary — these must be re-issued.

  **Caveat:** historical events in the event log reference pre-upgrade
  `ExecutionId` values. Replay against a fresh Valkey will not find them;
  this is expected. Event-log semantics (durability, causality) are
  unaffected — only ephemeral FF state is scoped to the new mint.

- **RFC-011 phase-1 mechanical sweep** — FF rev bump `a098710` → `1b19dd10`
  (RFC-011 exec/flow hash-slot co-location, phases 1-3). Consumer-side
  adoptions in cairn-fabric:
  - `num_execution_partitions` renamed to `num_flow_partitions`; default
    raised 64 → 256. **Operator action required** if `FF_EXEC_PARTITIONS`
    is set: rename env var to `FF_FLOW_PARTITIONS` before deploying, or
    accept the new default of 256.
  - `ExecutionId` construction migrated to deterministic mint helpers
    (`deterministic_solo` / `for_flow`). The `::new()`, `::from_uuid()`,
    and `Default` constructors are removed upstream.
  - Parallel `parse_spend_result` deleted from `budget_service.rs`;
    replaced with `ff_sdk::task::parse_report_usage_result` (FF #16 closed).
  - Hardcoded `format!("ff:usagededup:…")` sites replaced with
    `ff_core::keys::usage_dedup_key` helper.
  - API-boundary validation added: run/session/project IDs now reject
    control characters at the HTTP handler layer.
  - `FabricError` detail stripping: 500 responses no longer leak Valkey
    key names or Lua error internals.

- **`TaskFrameSink` orchestrator integration** (#30) — orchestrator logs
  tool calls, tool results, LLM responses, and checkpoints through a
  non-consuming sink on the active `CairnTask`, removing the need to thread
  a separate `FrameSink` handle alongside the task. Lease-health gate aborts
  the loop before irreversible side effects when FF reports 3 consecutive
  renewal misses. Checkpoint-snapshot serialize failures degrade to a WARN
  log instead of aborting the step.

### Removed

- **`ActiveTaskRegistry`** (#29) — retired in favour of FlowFabric-owned lease
  state. `CairnTask` now carries the underlying `ClaimedTask` directly; the
  cairn-side registry was a cache of state FF already holds atomically, and
  kept drifting out of sync under lease expiry. Event-emission gate in the
  orchestrator now reads lease health through `TaskFrameSink::is_lease_healthy`
  (the worker-sdk accessor) rather than a cairn-local flag.

---

## [0.1.0] — 2026-04-05

First complete, test-verified milestone. The core control-plane infrastructure
is implemented and RFC-compliant across all ten specified contracts.

### Added

#### Runtime and domain

- **Event-sourced runtime** — 111 `RuntimeEvent` variants covering sessions, runs,
  tasks, approvals, checkpoints, provider calls, credentials, channels, evals,
  signals, knowledge, and commercial events. Every state change is an append;
  no in-place mutation.
- **RFC 002 event-log contract** — append-only log with monotonically ordered
  `EventPosition`, causation-ID idempotency, cursor-based replay, and a
  72-hour SSE replay window. `find_by_causation_id` prevents duplicate command
  application across retries.
- **RFC 005 approval blocking** — `ApprovalRequested` gates run/task progression.
  Pending approvals surface in the operator inbox; `ApprovalResolved` unblocks
  the run atomically and increments the approval record version.
- **RFC 006 prompt release lifecycle** — `draft → active` state machine with
  `PromptReleaseCreated` / `PromptReleaseTransitioned` events; per-asset
  scorecard aggregation across releases.
- **RFC 007 provider health** — `ProviderConnectionRegistered`,
  `ProviderHealthChecked`, `ProviderMarkedDegraded`, `ProviderRecovered` events
  drive the health read model; consecutive failure tracking and per-tenant
  isolation.
- **RFC 008 multi-tenant isolation** — all read-model queries are scoped to
  `ProjectKey` (tenant + workspace + project); cross-tenant data does not
  appear in any listing.
- **RFC 009 provider routing and cost** — `FallbackChainResolver` with
  capability checking; `RouteDecisionRecord` persisted with `fallback_used`
  flag; per-run and per-session cost accumulation in USD micros; derived
  `RunCostUpdated` / `SessionCostUpdated` events emitted into the log.
- **RFC 013 eval rubrics and bundles** — rubric scoring (ExactMatch, Contains,
  Similarity, Plugin); baseline comparison with 5 % regression tolerance;
  `BundleEnvelope` import/export with `PromptLibraryBundle` and
  `CuratedKnowledgePackBundle` discriminators.
- **RFC 014 commercial feature gating** — `ProductTier` (LocalEval,
  TeamSelfHosted, EnterpriseSelfHosted), `Entitlement` categories,
  `DefaultFeatureGate` with fail-closed unknown-feature semantics,
  `EntitlementOverrideSet` events for operator-applied overrides.
- **Durability class contract** — `EntityDurabilityClass::FullHistory` for
  Session/Run/Task (full replay required); `CurrentStatePlusAudit` for all
  other entities. Defined in `cairn-domain` so domain tests can reason about
  durability without depending on the store crate.

#### Storage backends

- **`InMemoryStore`** — full `EventLog` + 51 read-model trait implementations;
  synchronous `apply_projection` within the same lock as `append`; broadcast
  channel for SSE live delivery; `subscribe()` for real-time event fan-out.
- **`PgEventLog`** — durable Postgres append-only event log; events stored in
  `event_log` table with JSON payload; `find_by_causation_id` scans for
  idempotency.
- **`PgAdapter`** — Postgres read models for Session, Run, Task, Approval,
  Checkpoint, Mailbox, ToolInvocation (7 of 51; remainder tracked as gap list
  for follow-on work).
- **`PgSyncProjection`** — synchronous projection applier runs within the same
  Postgres transaction as the append; all new `RuntimeEvent` variants have
  no-op arms.
- **`PgMigrationRunner`** — 17 embedded SQL migrations (V001–V017); applied
  atomically within a transaction on first boot; migration history recorded in
  `_cairn_migrations`.

#### HTTP server (`cairn-app`)

- **16 routes** wired with axum 0.7:
  - `GET /health` — liveness probe (auth-exempt)
  - `GET /v1/stream` — SSE event stream with `Last-Event-ID` replay (auth-exempt)
  - `GET /v1/status` — runtime + store health; Postgres health check when configured
  - `GET /v1/dashboard` — active runs, tasks, pending approvals, system health
  - `GET /v1/runs` + `GET /v1/runs/:id` — run listing and lookup
  - `GET /v1/sessions` — active session listing
  - `GET /v1/approvals/pending` + `POST /v1/approvals/:id/resolve` — approval inbox and resolution
  - `GET /v1/prompts/assets` + `GET /v1/prompts/releases` — prompt asset and release listing
  - `GET /v1/costs` — aggregate cost summary (calls, tokens, USD micros)
  - `GET /v1/providers` — provider binding listing
  - `GET /v1/events` — cursor-based event log replay
  - `POST /v1/events/append` — idempotent event append with causation-ID guard
  - `GET /v1/db/status` — Postgres connectivity and migration state
- **Bearer token auth middleware** (RFC 008) — all `/v1/*` routes except `/v1/stream`
  require `Authorization: Bearer <token>`; `ServiceTokenRegistry` supports
  multiple concurrent tokens.
- **SSE protocol** — `connected` event on open; replay up to 1 000 events after
  `Last-Event-ID`; 15-second keepalive comments; SSE `id:` field carries log
  position for resume.
- **Postgres wiring** — `--db postgres://...` flag creates a `PgPool`, runs
  pending migrations, and enables dual-write: events appended to Postgres
  (durability) and InMemory (read models + SSE broadcast). `GET /v1/events`
  served from Postgres log when configured.
- **CLI flags** — `--mode`, `--port`, `--addr`, `--db`, `--encryption-key-env`.
  Team mode binds `0.0.0.0` and requires `CAIRN_ADMIN_TOKEN`.

#### Knowledge pipeline (`cairn-memory`)

- **Ingest pipeline** — `IngestPipeline<S, C>` with `ParagraphChunker`;
  normalization for PlainText, Markdown, Html; chunk deduplication by
  content hash; no-op `NoOpEmbeddingProvider` for tests.
- **Retrieval scoring** — lexical relevance, freshness decay (`e^(-age/decay_days)`),
  staleness penalty (linear beyond threshold), source credibility, corroboration,
  graph proximity from `InMemoryGraphStore` neighbor overlap.
- **`InMemoryRetrieval`** — `with_graph()` now actually wires the graph store
  and computes proximity; `explain_result()` returns a `ResultExplanation` with
  all scoring dimensions and a human-readable summary.
- **Source quality diagnostics** — `InMemoryDiagnostics` tracks chunk counts,
  retrieval hits, average relevance per source; `index_status()` aggregates
  across all sources for a project.
- **Bundle import/export** — `InMemoryImportService` validates `KnowledgeDocument`
  artifacts, deduplicates by content hash, infers `ImportOutcome` (Create/Skip).
  `InMemoryExportService` bundles documents with origin scope and provenance metadata.

#### Eval system (`cairn-evals`)

- **`EvalRunService`** — in-memory eval run lifecycle: Pending → Running →
  Completed/Failed; `complete_run()` stores `EvalMetrics`;
  `build_scorecard()` aggregates across releases per asset;
  `set_dataset_id()` links a dataset to a run post-creation.
- **`EvalBaselineServiceImpl`** — `set_baseline()`, `compare_to_baseline()`;
  regression detection with ±5 % tolerance band; `fallback_used` flag on locked
  baselines; `select_baseline()` prefers locked over most-recent.
- **`EvalRubricServiceImpl`** — rubric scoring across ExactMatch, Contains,
  Similarity, Plugin dimensions; `score_against_rubric()` requires a dataset
  link; `PluginRubricScorer` trait for custom scoring backends.
- **`BanditServiceImpl`** (GAP-013) — `EpsilonGreedy` and `UCB1` selection
  strategies; `record_reward()` updates `pulls` and `reward_sum`; `with_fixed_rng()`
  for deterministic testing; `list_by_tenant()` for per-tenant experiment views.
- **Provider binding cost stats** — `ProviderBindingCostStatsReadModel`
  implemented with real event-log scan (replaces the stub that returned `None`);
  `list_by_tenant()` groups by `provider_binding_id` via raw event scan.

#### Docs

- **`docs/api-reference.md`** — 769-line operator API reference: all 16 routes,
  request/response shapes, curl examples, auth guide, error codes, server
  configuration, route summary table.
- **`docs/deployment.md`** — Docker Compose, Postgres setup, environment
  variables, team/local mode, TLS, production hardening.

### Architecture

- **12 Rust crates** — `cairn-domain`, `cairn-store`, `cairn-runtime`,
  `cairn-api`, `cairn-app`, `cairn-memory`, `cairn-graph`, `cairn-evals`,
  `cairn-tools`, `cairn-signal`, `cairn-channels`, `cairn-plugin-proto`.
  No circular dependencies.
- **Event log + synchronous projections** — the same `apply_projection` logic
  drives both InMemory and Postgres backends; there is no dual-implementation
  drift. Appends within a transaction guarantee projection consistency.
- **RFC 002–014 compliance** — ten RFC contracts verified by executable
  integration tests. `rfc_compliance_summary.rs` in `cairn-store/tests/`
  contains one focused test per RFC verifying the single most critical MUST
  requirement against the real store backend.

### Test suite

| Category | Count | Failures |
|----------|-------|----------|
| Lib tests (all crates except cairn-app) | 796 | 0 |
| Integration tests (new this session) | ~230 | 0 |
| Previously-broken tests (fixed) | 33 | 0 |
| **Total** | **~1 059** | **0** |

**40+ integration test files** across cairn-store (15 files), cairn-runtime (3),
cairn-memory (8), cairn-evals (3), cairn-api (1), cairn-domain (3).

Notable integration suites:
- `rfc_compliance_summary.rs` — one test per RFC (6 tests)
- `entity_scoped_reads.rs` — RFC 002 entity-scoped event pagination
- `idempotency.rs` — causation-ID idempotency contract (7 tests)
- `event_log_compaction.rs` — 50-event scale proof with cursor pagination
- `approval_blocking.rs` — RFC 005 approval gate lifecycle
- `provider_routing_e2e.rs` — RFC 009 fallback chain with FallbackChainResolver
- `cost_aggregation_accuracy.rs` — per-call micros precision, zero-cost isolation
- `durability_classes.rs` — RFC 002 entity durability contract
- `product_tier_gating.rs` — RFC 014 commercial gating across all three tiers

### Fixed

- **9 pre-existing integration test failures** across cairn-evals
  (`baseline_flow`, `dataset_flow`, `rubric_flow`), cairn-runtime
  (`binding_cost_stats`), and cairn-memory (`ingest_retrieval_pipeline`,
  `entity_extraction`, `explain_result`, `graph_proximity`,
  `provenance_tracking`). Root causes: wrong-crate `EvalSubjectKind` imports,
  extra argument to `create_run`, missing `IngestRequest` fields added in
  later RFCs, stub `ProviderBindingCostStatsReadModel` returning `None`,
  missing `explain_result()` method on `InMemoryRetrieval`, missing graph
  proximity implementation.
- **`DashboardOverview` initializers** in `cairn-api/src/overview.rs` — four
  internal test constructors updated to include the six new RFC 010
  observability fields added during the GAP implementation phase.
- **`PgSyncProjection` non-exhaustive patterns** — `ApprovalPolicyCreated` and
  `PromptRolloutStarted` were missing no-op arms; added to resolve the
  `--features postgres` compile error.

---

*This changelog was generated at the close of the implementation session.*
*Session date: 2026-04-05. Workspace: cairn-rs.*
