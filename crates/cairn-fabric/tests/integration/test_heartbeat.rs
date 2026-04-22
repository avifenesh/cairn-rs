// Lease heartbeat integration tests.
//
// Covers the only FCALL in the Phase D PR 2 scope (run_service +
// task_service + session_service + claim_common) that is still
// zero-coverage after the suspension / signal / session tests landed:
//
//   - ff_renew_lease (via tasks.heartbeat)
//
// Heartbeat integrity is the difference between "long-running task stays
// leased by its worker" and "another worker steals the lease mid-step and
// silently double-executes the user's work". That is the hardest class of
// bug to diagnose in production; this file is the regression net before
// Phase D PR 2 refactors the service layer on top of the Engine tag-write
// API.
//
// Runs against the testcontainers-provisioned Valkey shared by the rest of
// the `integration` test binary. No FLUSHDB — every test is scoped under a
// fresh uuid ProjectKey via `TestHarness::setup()` and uuid-suffixed
// task / session ids.

use cairn_domain::lifecycle::TaskState;
use cairn_fabric::fcall::{claim::build_renew_lease, names::FF_RENEW_LEASE};
use ff_core::keys::{ExecKeyContext, IndexKeys};
use ff_core::partition::execution_partition;
use ff_core::types::AttemptIndex;

use crate::TestHarness;

/// Happy path: heartbeat on an active lease extends
/// `lease_expires_at_ms` and leaves the task in an actionable
/// (Running / Leased / Queued) state. Proves `ff_renew_lease` is wired
/// end-to-end through `tasks.heartbeat` against live FF, and that the
/// renewal actually mutates exec_core — not just returns Ok on the
/// service layer while FF silently no-ops.
#[tokio::test]
async fn test_heartbeat_extends_lease_expiry() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    // Short-ish initial lease so the extension is observable against the
    // wall-clock `lease_expires_at_ms` FF writes. 5s is short enough that
    // a +30s extension crosses a clearly distinguishable boundary without
    // risking the original lease expiring mid-test.
    let claimed = h
        .fabric
        .tasks
        .claim(
            &h.project,
            Some(&session_id),
            &task_id,
            "test-worker".into(),
            5_000,
        )
        .await
        .expect("claim failed");

    let initial_expiry = claimed
        .lease_expires_at
        .expect("fresh claim must publish lease_expires_at");
    let initial_epoch = claimed.version;

    // A human-scale wait so the "after heartbeat" expiry is strictly
    // greater than "before heartbeat" — FF writes wall-clock ms and two
    // calls within the same millisecond are not distinguishable.
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let renewed = h
        .fabric
        .tasks
        .heartbeat(
            &h.project,
            Some(&session_id),
            &task_id,
            30_000, // extension much larger than the remaining window
        )
        .await
        .expect("heartbeat on live lease failed");

    let renewed_expiry = renewed
        .lease_expires_at
        .expect("post-heartbeat record must still have lease_expires_at");

    assert!(
        renewed_expiry > initial_expiry,
        "heartbeat must strictly extend lease_expires_at: initial={}, renewed={}",
        initial_expiry,
        renewed_expiry,
    );

    // FF contract (lease.lua lines 8-16): `ff_renew_lease` preserves
    // `lease_id` and `lease_epoch`. Renewal is NOT a steal — the epoch
    // must NOT bump. Pin that here so a future refactor that accidentally
    // re-grants the lease on heartbeat (bumping the epoch) trips this
    // assertion.
    assert_eq!(
        renewed.version, initial_epoch,
        "heartbeat must preserve lease_epoch: initial={}, renewed={}",
        initial_epoch, renewed.version,
    );

    // Task must still be in an actionable state post-heartbeat. Accept
    // any runnable state — delayed_promoter / claim machinery can race
    // the transition between Leased and Running.
    assert!(
        matches!(
            renewed.state,
            TaskState::Queued | TaskState::Leased | TaskState::Running
        ),
        "post-heartbeat task must be runnable, got {:?}",
        renewed.state,
    );

    h.teardown().await;
}

/// Stale-epoch rejection: `ff_renew_lease` must return `stale_lease`
/// when called with a `lease_epoch` that is not the current one on
/// exec_core (FF lease.lua:81-82).
///
/// The public `tasks.heartbeat(...)` API reads the current epoch fresh
/// via `resolve_active_lease` before every call, so there is no way to
/// observe stale-epoch rejection through the public surface without a
/// race. We therefore invoke the FCALL directly via
/// `FabricRuntime::fcall` with a literal wrong epoch string. This is
/// still a behavior assertion — we assert the returned error mentions
/// `stale_lease` (the contract), not a specific wire shape.
///
/// Phase D PR 2 will reshape the stack above `fcall`; the contract
/// ("wrong epoch → stale_lease error") must survive.
#[tokio::test]
async fn test_heartbeat_with_stale_epoch_is_rejected() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    let claimed = h
        .fabric
        .tasks
        .claim(
            &h.project,
            Some(&session_id),
            &task_id,
            "test-worker".into(),
            30_000,
        )
        .await
        .expect("claim failed");

    // Sanity: a legitimate heartbeat succeeds. If this trips, the later
    // stale-epoch assertion is meaningless (we couldn't renew even with
    // the right epoch).
    h.fabric
        .tasks
        .heartbeat(&h.project, Some(&session_id), &task_id, 30_000)
        .await
        .expect("baseline heartbeat with fresh epoch failed");

    // Derive the exec_core + index keys the same way task_service does
    // (id_map::session_task_to_execution_id mirrors the service's private
    // task_to_execution_id when a session is present).
    let eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_id,
        h.partition_config(),
    );
    let partition = execution_partition(&eid, h.partition_config());
    let ctx = ExecKeyContext::new(&partition, &eid);
    let idx = IndexKeys::new(&partition);

    // Read the actual attempt_id + lease_id so the FCALL is
    // well-formed everywhere EXCEPT the epoch. If we also fabricated
    // those, FF would reject on `lease_id_mismatch` before reaching the
    // epoch check, and we wouldn't be proving what we claim to be proving.
    let attempt_id: String = {
        let v: Option<String> = h
            .fabric
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_id")
            .await
            .expect("HGET current_attempt_id failed");
        v.unwrap_or_default()
    };
    let lease_id: String = {
        let v: Option<String> = h
            .fabric
            .runtime
            .client
            .hget(&ctx.core(), "current_lease_id")
            .await
            .expect("HGET current_lease_id failed");
        v.unwrap_or_default()
    };
    assert!(
        !lease_id.is_empty(),
        "post-claim exec_core must publish current_lease_id; got empty",
    );

    // Forge an epoch the service would never send. FF's current epoch is
    // `claimed.version`; pick a value that definitely is not current.
    let bogus_epoch = claimed.version.saturating_add(9_999_999).to_string();

    let (keys, args) = build_renew_lease(
        &ctx,
        &idx,
        &eid,
        AttemptIndex::new(0),
        &attempt_id,
        &lease_id,
        &bogus_epoch,
        30_000,
    );
    let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // The wire call itself returns Ok(Value::Array([b"err", b"stale_lease", ...])) — Lua-level
    // errors are encoded in the returned Value, not as a Valkey-level error.
    // `check_fcall_success` is what turns those into a FabricError; we use it
    // here to mirror the service-layer error path and assert the message
    // surfaces `stale_lease`.
    let raw = h
        .fabric
        .runtime
        .fcall(FF_RENEW_LEASE, &key_refs, &arg_refs)
        .await
        .expect("fcall transport must succeed; stale_lease is a Lua-level err value");

    let check = cairn_fabric::helpers::check_fcall_success(&raw, FF_RENEW_LEASE);
    let err = check.expect_err("stale-epoch heartbeat must surface as FabricError, not Ok");
    let msg = err.to_string();
    assert!(
        msg.contains("stale_lease"),
        "stale-epoch error must mention `stale_lease` (FF lease.lua contract); got {msg:?}",
    );

    // Post-condition: the bogus-epoch attempt must not have corrupted the
    // real lease. A follow-up legitimate heartbeat must still succeed.
    h.fabric
        .tasks
        .heartbeat(&h.project, Some(&session_id), &task_id, 30_000)
        .await
        .expect("post-rejection heartbeat with real epoch must still succeed");

    h.teardown().await;
}
