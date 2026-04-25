# FlowFabric 0.3.4 → 0.9.0 Migration Plan

Status: **Active — FF 0.9.0 published 2026-04-25. CG in flight.**
Last updated: 2026-04-25
Author: planning aggregated from 4 research agents (breaking changes, PG backend, boot path, encapsulation)
Related: FF#277–#283 (cairn-first upstream asks — all CLOSED in FF 0.9.0)

## 0.9.0 addendum (2026-04-25)

FF 0.9.0 shipped with all 7 cairn-first asks landed:

- **FF#277 capability discovery** — `EngineBackend::capabilities()` + typed `Capabilities`/`Supports` bitset
- **FF#278 LeaseSummary full fields** — `lease_id`, `attempt_index`, `last_heartbeat_at` added
- **FF#279 flowfabric umbrella crate** — `flowfabric = "0.9"` on crates.io, re-exports via features `valkey` (default) / `postgres` / `engine` / `scheduler-internals` / `script-internals`
- **FF#280 seed_waitpoint_hmac_secret trait** — cairn's raw `HSET` in `boot.rs:281-321` goes away
- **FF#281 prepare() trait** — cairn's `ensure_library` retry loop in `boot.rs:78-104` goes away
- **FF#282 subscribe_lease_history trait** — cairn's raw `XREAD` in `lease_history_subscriber.rs` goes away (the single biggest mechanical win, ~-250 LOC + drops ferriskey direct dep)
- **FF#283 ff-sdk::ClaimGrant/ClaimPolicy re-export** — cairn drops `ff-scheduler` direct dep

The original plan's CG/CH/CI/CJ sequencing holds. CG is now "bump to 0.9.0" (not 0.8.2) but everything else — scope items, risks, non-goals — stays.

ferriskey 0.9.0 exists on crates.io; cairn drops its direct pin once FF#282 is adopted.

## TL;DR

FF 0.3.4 → 0.8.1 is **one mechanical bump PR (~150 LOC)** followed by a series of deletion PRs (**~-1,500 LOC net**) as we stop owning local abstractions that FF now provides. PG backend ships as an opt-in cargo feature (off by default) once FF 0.8.2 lands.

Spirit: cairn stops duplicating the FF surface. We expose what users need; we stop wrapping what FF already exposes well.

## Background

Cairn currently pins `ff-* = "0.3"` across 6 crates plus `ferriskey` as a direct dep. Between 0.3.4 and 0.8.1:

- **RFC-012** finished: `EngineBackend` trait finalized (~32 methods)
- **0.4.0** reshaped `WorkerConfig { host, port, tls, cluster, ... }` → `WorkerConfig { backend: BackendConfig, ... }`
- **0.4.0** landed typed `EngineError` (unblocks cairn Phase E)
- **0.5.0** moved `suspend` to a typed trait method with `SuspendArgs` / `SuspendOutcome`
- **0.5.0** added cursor-paginated `list_suspended` / `list_executions` / `list_flows` / `list_lanes`
- **0.6.0** shipped RFC-014 (composite resume conditions), RFC-015 (stream durability modes), RFC-016 (edge-dependency policies)
- **0.8.0** reshaped `ServerConfig` again (flat Valkey fields → `BackendKind` + nested configs), added `ClaimPolicy::new`, added Handle codec v2 (`BackendTag`)
- **0.8.0** published `ff-backend-postgres` as a sibling crate (partial-parity, 23/50 stubs)
- **0.8.1** is a publish-order fix only

Four research reports documented the deltas. This plan sequences the consumption.

## Principles (from user direction)

1. **Less code, not more.** Every PR deletes cairn-side wrapper code where possible.
2. **Don't abstract above what users need.** cairn's public API (HTTP + UI) stays stable; internal cairn ↔ FF coupling changes.
3. **User-choice backend, not forced collapse.** Operators pick Valkey (fast) or Postgres (single-DB) at deploy time.
4. **Stay first-user-of-FF.** Consume the features we asked for; file follow-ups (FF#277–#283) for gaps.

## Sequencing

```
PR CG  (0.8.2 bump, mechanical)       ──┐
                                         ├── unblocks → PR CH (collapse + encapsulate)
FF 0.8.2 ships ────────────────────────┘
                                         ├── unblocks → PR CJ (PG backend opt-in)
PR CH ────────────────────────────────────┘
                                         └── unblocks → PR CI (feature adoption — RFC-014/015/016)
```

CH, CJ, CI can ship in any order after CG + FF 0.8.2 land. CH first is our preference (purely additive-by-subtraction), CI last (requires brainstorm on approval-surface rebuild).

---

## PR CG — Mechanical bump (blocking, ~150 LOC)

**Goal:** cairn compiles + passes all tests against FF 0.8.2 with zero behavior change.

**Blocked on:** FF 0.8.2 release on crates.io (0.8.1 is a broken publish, 0.8.2 is the bug-fix).

### Scope

1. **Cargo.toml bumps.** 6 pins `"0.3"` → `"0.8"`. Add `ff-backend-postgres = { version = "0.8", optional = true }` under `[dependencies]`. New feature `fabric-postgres = ["dep:ff-backend-postgres"]`, default stays `["fabric-valkey"]`.

2. **`WorkerConfig` reshape.** 0.4.0 collapsed flat Valkey fields into `backend: BackendConfig`. Cairn `crates/cairn-fabric/src/worker_sdk.rs` (471 LOC) uses the flat shape. Migration:
   ```rust
   // old
   WorkerConfig { host, port, tls, cluster, ... }
   // new
   WorkerConfig { backend: BackendConfig::Valkey(ValkeyBackendConfig { host, port, tls, cluster, ... }), ... }
   ```
   ~50 LOC changed.

3. **`ServerConfig` reshape** (0.8.0). Same pattern at cairn's `state.rs:1284-1295` boot site. ~20 LOC.

4. **`BackendError` adoption** (0.4.0). Replace `FabricError::Valkey(String)` → `FabricError::Backend(#[from] BackendError)`. Kind-string remapping: `"io_error"` → `"transport"`, etc. ~30 LOC in `crates/cairn-fabric/src/error.rs`.

5. **Typed `SuspendArgs`** (0.5.0). Cairn's suspend path at `crates/cairn-fabric/src/services/task_service.rs` + `run_service.rs` builds args as JSON today. Switch to typed builder. ~30 LOC.

6. **`ClaimPolicy::new` signature** (0.8.0). `immediate()` removed; switch to `ClaimPolicy::new(worker_id, worker_instance_id, lease_ttl_ms, max_wait)`. One call site in `crates/cairn-fabric/src/services/scheduler_service.rs`. ~5 LOC.

7. **Handle codec v2**. `handle_codec::Handle.opaque` now carries a `BackendTag` magic byte. Cairn persists handles in the event log; old handles must decode via compat path (v2 decoder recognizes v1 handles). No cairn code change expected — FF owns the compat. **Verification step:** assert existing `lease_history` + `waitpoint_token` bytes from PG test fixtures still round-trip.

8. **No behavior changes.** No new endpoints, no new events, no UI work. Pure port.

### Tests

- `cargo test --workspace` must pass with no changes to test expectations.
- Integration test: boot cairn with a v1-minted handle (e.g. restored from a pre-bump event log snapshot), verify it still resolves.
- Chaos-append 120-run baseline must stay green.

### Risks

- **Dual `ServerConfig` reshapes in one cycle (0.4 + 0.8)** means the bump has to re-learn both. Mitigated by the fact that we're skipping 0.4.0 → 0.7.x directly to 0.8.2.
- **`BackendError` kind-string remapping** could silently reclassify errors cairn observes. Mitigated by the test suite plus a manual audit of `fabric_err_to_runtime` in `crates/cairn-app/src/fabric_adapter.rs:96`.
- **FF 0.8.2 may ship new breakages** beyond the 0.8.1 surface. Mitigated by waiting for the publish + reviewing its changelog before starting CG.

### Not in CG

- Typed `EngineError` adoption in `FabricError::Script`: deferred to CH to keep this PR purely mechanical.
- Deleting cairn's `Engine` trait / `valkey_impl`: CH.
- Collapsing the 26 service-layer leaks: CH.
- PG backend wiring: CJ.
- RFC-014/015/016 feature adoption: CI.

---

## PR CH — Encapsulation collapse (2–3 sub-PRs, ~-1,500 LOC net)

**Goal:** stop duplicating what FF now provides. Delete cairn's local abstractions where upstream supersedes.

**Blocked on:** CG merged.

### CH-1 — Typed `EngineError` + delete error-helpers (~-150 LOC, ~300 LOC changed)

**Unblocks Phase E** from the old decoupling roadmap.

1. Add `FabricError::Engine(#[from] EngineError)`.
2. Migrate every `FabricError::Script(ScriptError)` classifier through `EngineError::class()` (Contention / Conflict / State / NotFound / Validation / Transport / Bug / Contextual).
3. Delete `helpers::{check_fcall_success, fcall_error_code, is_claim_contention}` at `crates/cairn-fabric/src/helpers.rs` (~120 LOC).
4. Delete `fabric_err_to_runtime` string-matching at `crates/cairn-app/src/fabric_adapter.rs:96`; replace with `match EngineError::{...}`.

### CH-2 — Migrate 26 service-layer leaks to `EngineBackend` trait calls (~-400 LOC changed across 9 files)

For each service file at `crates/cairn-fabric/src/services/`:

| File | Old imports to drop | New trait calls |
|---|---|---|
| `budget_service.rs` | `ff_core::keys::*`, `ff_core::partition::*` | `engine.create_budget`, `engine.report_usage`, `engine.reset_budget`, `engine.get_budget_status` |
| `quota_service.rs` | keys + partition | `engine.create_quota_policy` |
| `rotation_service.rs` | keys + partition + `ferriskey::Client` HSET | `engine.rotate_waitpoint_hmac_secret_all` |
| `run_service.rs` | keys + partition + ARGV prep | `engine.create_execution`, `engine.cancel_execution`, `engine.change_priority`, `engine.replay_execution` |
| `session_service.rs` | keys + partition | `engine.create_flow`, `engine.cancel_flow`, existing `set_flow_tag(s)` |
| `task_service.rs` | keys + partition + ARGV prep | `engine.renew`, `engine.progress`, `engine.append_frame`, `engine.complete`, `engine.fail`, `engine.revoke_lease` |
| `worker_service.rs` | `ff_scheduler::claim::Scheduler` | `engine.claim_for_worker` |
| `scheduler_service.rs` | `ff_scheduler::claim::{Scheduler, ClaimGrant}` | `engine.claim_for_worker` + (once FF#283 ships) `ff_sdk::ClaimGrant` |
| `claim_common.rs` | partition + ARGV | `engine.claim_resumed_execution` |

Side effects: ~24 HGET pre-reads in these services disappear (ARGV-prep was reading state before FCALL; trait methods encapsulate that). **Obsoletes the Phase D `read_fcall_context` helper plan** — skip directly to trait-call migration.

### CH-3 — Delete cairn's `Engine` trait + `valkey_impl`, drop `ferriskey` + `ff-scheduler` direct deps (~-700 LOC)

1. Cairn's `crates/cairn-fabric/src/engine/mod.rs` defines a local `Engine` trait (the adapter layer from Engine Decoupling Phase A+B+C). Post-CH-2, this is a thin passthrough over `ff_core::engine_backend::EngineBackend`. Delete the wrapper; consume the FF trait directly.
2. `valkey_impl.rs` (841 LOC) collapses to a newtype (~50 LOC) or disappears entirely if no cairn-specific adapter logic remains.
3. Drop `ferriskey = "..."` from `crates/cairn-fabric/Cargo.toml` **if** FF#282 (subscribe_lease_history) has shipped. Otherwise keep for lease_history_subscriber.rs.
4. Drop `ff-scheduler = "0.3"` from `crates/cairn-fabric/Cargo.toml` **if** FF#283 (ClaimGrant re-export) has shipped. Otherwise keep.

### CH total estimate

| Sub-PR | LOC delta |
|---|---|
| CH-1 (typed EngineError) | -150 |
| CH-2 (service migration) | -400 |
| CH-3 (trait collapse + dep drop) | -700 — dependent on FF#282 + FF#283 |
| CH-4 (gated by FF#278) | -130 (LeaseSummary wrapper) |
| CH-5 (gated by FF#280 + #281) | -80 (boot-path deletions) |
| **Total** | **-1,460 LOC net** if all upstream asks ship |

CH-1 and CH-2 can ship regardless of FF#277–#283. CH-3/4/5 are gated on upstream action.

---

## PR CJ — PG backend as opt-in operator choice (2 sub-PRs, ~870 LOC added)

**Goal:** operators can pick `--fabric postgres://...` at deploy time. Valkey stays default.

**Blocked on:** CG merged, FF 0.8.2 stable, FF#277 (capability discovery) ideally shipped.

### CJ-1 — Config plumbing + boot reshape (~450 LOC)

1. **`FabricConfig::from_url(url: &str)`** — new constructor. URL scheme auto-detect: `redis://` | `rediss://` | `valkey://` → Valkey; `postgres://` | `postgresql://` → Postgres. Legacy env vars `CAIRN_FABRIC_HOST/PORT/TLS/CLUSTER` stay supported with deprecation warning for one minor.
2. **CLI flag** `--fabric <URL>` in `crates/cairn-app/src/bootstrap.rs`, mirroring `--db` conventions. Do NOT merge with `DATABASE_URL` — cairn-store and cairn-fabric are separate domains (operator may run PG-store + Valkey-fabric for fast hot path + relational projections).
3. **Boot branching** in `crates/cairn-fabric/src/boot.rs`:
   - Backend enum wrapper: `enum FabricBackend { Valkey(ValkeyBackend), Postgres(PostgresBackend) }`. Only needed if FF 0.8.2 `EngineBackend` isn't `dyn`-safe; if it is, use `Arc<dyn EngineBackend>`.
   - `ensure_library` wrapped in `backend.prepare()` (gated on FF#281 — otherwise Valkey-only with conditional skip for PG).
   - `seed_waitpoint_hmac_secret_if_configured` wrapped in `backend.seed_waitpoint_hmac_secret()` (gated on FF#280).
4. **Cargo features** finalized: `fabric-valkey` (default), `fabric-postgres` (opt-in but pre-wired in the binary).
5. **Capability check at boot** (gated on FF#277): log `backend.capabilities()`, warn if operator-required surfaces (cancel/replay/revoke in team mode) are stubs on the selected backend.

### CJ-2 — Dual-backend parity tests + UI capability exposure (~420 LOC)

1. **PG testcontainer harness** at `crates/cairn-fabric/src/test_harness.rs`, mirroring `valkey_endpoint()`. ~80 LOC.
2. **LiveHarness parameterization** at `crates/cairn-app/tests/support/live_fabric.rs` — add `setup_with_fabric_backend(FabricBackendChoice)`. ~120 LOC.
3. **Parity tests duplicated** for critical invariants: RFC 020 durability, chaos-append, scanner filter instance_tag. ~150 LOC.
4. **`GET /v1/fabric/backend` endpoint** exposing `Capabilities` JSON for the UI.
5. **UI greyrender** unsupported operator actions per `Capabilities.supports.*`: Cancel/Replay/Revoke buttons show tooltip "not supported on Postgres backend (limited feature set)".
6. **Docs**: `docs/backend-choice.md` documenting operator tradeoffs + `docs/design/rfcs/RFC-XXX-backend-selection.md`.

### CJ risks

- **23 PG stubs at 0.8.1** bite operator-control UX. Documented as "limited feature set"; capability discovery at boot prevents boot-time surprises.
- **~3-10x perf delta** on hot paths (lease renewal, claim-task, stream tail) per PG backend report. We publish the bench matrix so operators size their own deployments.
- **Single Pg pool shared with cairn-store** is supported by `PostgresBackend::from_pool` — but cairn dual-write still writes cairn-events + FF state in separate transactions. Operational collapse, not atomicity.

---

## PR CI — New feature adoption (3 sub-PRs, scoped separately)

**Goal:** consume features we didn't ask for but that let cairn delete code.

**Blocked on:** CH-2 shipped (trait-call posture).

### CI-1 — RFC-014 composite resume → rebuild `ToolCallApprovalService` (~-400 LOC estimated)

**Changes the architecture story.** FF 0.6.0 landed `Count { DistinctWaitpoints | DistinctSignals | DistinctSources }` + `AllOf` as first-class resume conditions. This is effectively a built-in N-of-M approvals primitive.

Cairn's current `ToolCallApprovalService` (landed via BP-3/5/6, ~400+ LOC) mints waitpoints + tracks approvals in a cairn-local projection. With composite resume, the "N approvers must resolve" logic moves upstream: cairn mints one waitpoint with `Count { DistinctSources, n }` condition; FF evaluates the count; cairn's service collapses to a thin minter.

**Requires brainstorm before implementation** — the current approval data model is load-bearing across BP-2 (projections) and BP-6 (HTTP `/amend`). Port incrementally or rebuild cleanly?

### CI-2 — RFC-016 edge-dependency policies → agent DAGs (~-100 LOC)

`AnyOf`, `Quorum`, `OnSatisfied::{CancelRemaining, LetRun}` let cairn's agent DAGs express "wait for any of N parallel reviewers" natively instead of hand-rolling in `run_service.rs`. Sibling-cancel + crash-recovery reconciler come for free.

### CI-3 — RFC-015 `read_summary` into F29 observability UI (~-50 LOC)

`engine.read_summary()` with JSON Merge Patch stream mode. F29's RunDetailPage telemetry panel can use it to render progressive agent output instead of walking frames. Aligns naturally with F29-CE (observability UI, currently awaiting F29-CD merge).

---

## Decisions locked (autonomous, per user direction)

These were clearly bounded; logged here for review:

1. **Config shape for CJ**: `--fabric <URL>` + `CAIRN_FABRIC_URL` env + scheme auto-detect. Not `--fabric-backend valkey|postgres`. Not merged with `DATABASE_URL`.
2. **Default features**: `fabric-valkey` default; `fabric-postgres` opt-in but pre-wired in cairn-app's `default = [...]`. Matches cairn-store's postgres+sqlite-both-on convention.
3. **Stuck-run threshold** (F29-CD): configurable via `/v1/settings/defaults/system/system/stuck_run_threshold_ms` rather than hardcoded.
4. **Cost wire format** (F29-CD): `cost_micros` on wire, UI formats to USD.
5. **Ordering of CH sub-PRs**: CH-1 (error typing) first, CH-2 (services) second, CH-3+ gated on upstream asks. Ship what we can.

## Decisions deferred to user

1. **CI-1 (ToolCallApprovalService rebuild)** — port or rebuild? Needs brainstorm.
2. **Timing of CJ vs CH** — finish CH entirely before CJ, or interleave? CH-1/CH-2 are low-risk so interleaving is fine; CH-3 requires FF#282+#283 so naturally trails CJ.
3. **UI "limited feature set" warning** for PG mode — how loud? Red banner vs quiet tooltip.
4. **PG backend in default cargo features** — pre-wired (CJ-1 plan) or truly opt-in (`cargo build --features fabric-postgres`)?

## Upstream asks open (FF#277–#283)

| # | Title | Unblocks in this plan |
|---|---|---|
| 277 | Capability discovery | CJ-1 boot check, CJ-2 UI greyrender |
| 278 | LeaseSummary full fields | CH-4 (~-130 LOC) |
| 279 | `flowfabric` umbrella crate | Cargo.toml pin reduction 7→1 |
| 280 | `seed_waitpoint_hmac_secret` trait | CJ-1 PG adoption + CH-5 (~-45 LOC) |
| 281 | Backend-agnostic `prepare()` | CJ-1 PG adoption + CH-5 (~-35 LOC) |
| 282 | `subscribe_lease_history` trait | CH-3 ferriskey drop + (~-250 LOC) |
| 283 | `ff-sdk::ClaimGrant` re-export | CH-3 `ff-scheduler` drop |

## Timeline estimate

- **CG**: 1 day after FF 0.8.2 ships.
- **CH-1**: 1 day (independent of upstream).
- **CH-2**: 2-3 days (9 service files).
- **CH-3/4/5**: gated on FF#277–#283 + #282 most critical.
- **CJ**: 3-4 days (FF 0.8.2 stable + FF#277 shipped).
- **CI-1**: 1 week (design brainstorm + incremental port).
- **CI-2, CI-3**: 1-2 days each.

Parallelizable where independent. Total sprint spend: 2-3 weeks end-to-end if upstream moves in parallel.
