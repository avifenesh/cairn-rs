# FF upstream ask: lease renewal for pull-model drivers

Status: draft — NOT yet filed against FlowFabric.
Owner: cairn-rs (avifenesh).
Target: ff-sdk 0.10.x follow-up.
Related in cairn: F51 (fixed cairn-side in PR fix/f51-lease-reclaim-at-orchestrate-entry).

## Problem

`ff_sdk::FlowFabricWorker::claim` returns a `ClaimedTask` whose
lifetime includes a background tokio task that renews the FF lease at
`lease_ttl_ms / 3`. This is correct for a long-lived worker that owns
the execution for its entire duration (the canonical worker loop).

It is **not** correct for a *pull-model driver*: an HTTP handler that
runs one iteration of work per invocation, returns to the caller, and
resumes on the next HTTP call. cairn's `POST /v1/runs/:id/orchestrate`
endpoint is the motivating example:

* Each call does GATHER → DECIDE → EXECUTE once, then returns.
* Between calls the operator may sit on a tool-call approval screen
  for minutes, or the client may batch calls across a human-paced
  dashboard flow.
* When the handler returns, the `ClaimedTask` is dropped, its renewer
  dies, and FF's lease ticks to its TTL untouched.
* The next call arrives past the TTL. The terminal FCALL on that call
  rejects with `lease_expired`, even though the run is functionally
  healthy and the caller has perfect causality.

## Customer-visible outcome

A dogfood user on 2026-04-26 observed this during an operator-paced
approval flow. Prior to the cairn-side fix, operators had to set
`CAIRN_FABRIC_LEASE_TTL_MS=600_000` in their env to paper over the gap
— a 20× default that bloats the `worker_leases` index and slows FF's
expired-lease scanner sweep.

## cairn-side fix (shipped)

`FabricRunService::renew_lease_if_stale(run_id, min_remaining_ms)`:
snapshot-read the lease, renew in place via `ff_renew_lease` if
stale, fall back to `issue_grant_and_claim` if the lease is already
gone, no-op otherwise. The orchestrate handler calls this after
`ensure_active` on every invocation. Default TTL restored to 30s.

## Proposed upstream change (not filed yet)

One of two shapes:

1. **New `ff_sdk::PullDriver` handle.** A second construction mode
   alongside `ClaimedTask` that:
   * Does not own a background renewer.
   * Exposes `renew_if_stale(min_remaining_ms)` on the handle as a
     single async call backed by `ff_renew_lease`.
   * Callers invoke at the top of each pull-model iteration.
2. **Extend `ClaimedTask` with a `detach_renewer()` method** plus a
   `renew_now(min_remaining_ms)` synchronous tick. Lets the caller
   keep the handle but disable the background task for pull-model
   use.

Option 1 is cleaner (two orthogonal types for two orthogonal access
patterns); option 2 minimises API surface change.

## Concrete cairn use case

`FabricRunService::renew_lease_if_stale` (cairn-fabric/src/services/run_service.rs)
wraps the engine-level `renew_task_lease` directly today because it
only needs to renew, not to own a task handle. If ff-sdk exposed a
pull-mode primitive, cairn would route the engine-level call through
the SDK instead — matching the "cairn stays thin; SDK does the work"
posture cairn targets for every FF integration.

## Non-goals

* Changing the default worker loop. Push-model workers continue to
  use the existing `ClaimedTask` with background renewal.
* Changing FF's lease expiry semantics. `lease_expired` remains the
  correct reject code when a caller genuinely misses the TTL; the
  ask is only about giving pull-mode drivers a clean renewal
  primitive so they don't need to construct one out of engine-level
  primitives.

## Next step

Draft the FF issue under `avifenesh/cairn` FF ask tracker (not filed
from within the cairn-rs F51 PR itself — the fix lands regardless of
FF's response, and the ask is on a separate cadence per the
cairn-first playbook).
