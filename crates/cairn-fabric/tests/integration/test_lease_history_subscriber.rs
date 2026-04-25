//! End-to-end proof that the lease-history subscriber picks up an
//! FF-initiated lease-expiry event and emits the right BridgeEvent.
//!
//! Flow:
//! 1. Build a FabricServices with the subscriber wired in.
//! 2. Submit a task, claim it, start it (task is now `Running` with
//!    an active lease).
//! 3. Force the lease to expire: HSET `lease_expires_at = 0` on
//!    exec_core, then FCALL `ff_mark_lease_expired_if_due` so FF
//!    writes the `expired` XADD frame on the lease_history stream.
//! 4. Wait for the subscriber to pick up the frame and emit
//!    `BridgeEvent::TaskStateChanged { to: RetryableFailed,
//!    failure_class: Some(LeaseExpired) }`.
//! 5. Assert the cairn-store TaskReadModel transitions to
//!    `RetryableFailed` (confirms the event actually flowed through
//!    EventBridge → event_log → projection, not just the bridge
//!    channel).

use std::sync::Arc;

use cairn_domain::{FailureClass, TaskState};
use cairn_fabric::{id_map, CairnWorker, FabricServices};
use cairn_store::projections::{FfLeaseHistoryCursorStore, TaskReadModel};
use flowfabric::core::keys::ExecKeyContext;
use flowfabric::core::partition::execution_partition;

use crate::TestHarness;

#[tokio::test]
async fn subscriber_emits_retryable_failed_on_lease_expiry() {
    // Share the harness's Valkey container + HMAC secret via the same
    // shared `VALKEY_CONTAINER` OnceCell TestHarness uses. We spin
    // our own FabricServices instance on top so we can start it with
    // lease_history wired; the harness defaults to the subscriber-off
    // variant.
    let harness = TestHarness::setup().await;

    // Build a sibling FabricServices with the lease-history subscriber
    // attached to the *same* event_log backing the harness's
    // projections — otherwise the subscriber writes into one log and
    // the test reads from another.
    let event_log_shared: Arc<dyn cairn_store::event_log::EventLog + Send + Sync> =
        harness.event_log.clone();
    let cursor_store: Arc<dyn FfLeaseHistoryCursorStore> = harness.event_log.clone();

    let mut config: cairn_fabric::FabricConfig = (*harness.fabric.runtime.config).clone();
    // Per-test-unique namespace prevents collision if multiple tests
    // spin up their own FabricServices against the same container.
    config.namespace = flowfabric::core::types::Namespace::new(format!(
        "lease_hist_subscriber_test_{}",
        uuid::Uuid::new_v4()
    ));
    let subscriber_fabric =
        FabricServices::start_with_lease_history(config.clone(), event_log_shared, cursor_store)
            .await
            .expect("start_with_lease_history");

    // Submit → claim → start against the SUBSCRIBER's fabric so the
    // Valkey state lives on the partition the subscriber is watching.
    let session_id = harness.unique_session_id();
    let task_id = harness.unique_task_id();
    let project = harness.project.clone();

    subscriber_fabric
        .tasks
        .submit(&project, task_id.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit task");

    let grant = loop {
        let g = subscriber_fabric
            .scheduler
            .claim_for_worker(
                &config.lane_id,
                &config.worker_id,
                &config.worker_instance_id,
                config.grant_ttl_ms,
            )
            .await
            .expect("claim_for_worker");
        if let Some(g) = g {
            break g;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    // Materialize the grant into a real ff_claim_execution call so
    // the exec's lifecycle_phase flips from `runnable` → `active`.
    // `ff_mark_lease_expired_if_due` rejects any non-active exec
    // with ALREADY_SATISFIED / not_active, so the lease_history
    // XADD only fires once we've really claimed.
    let worker = CairnWorker::connect(&config, subscriber_fabric.bridge.clone())
        .await
        .expect("CairnWorker::connect");
    let _claimed = worker
        .claim_from_grant(config.lane_id.clone(), grant)
        .await
        .expect("claim_from_grant");

    // Derive the exec_id from the same mint cairn-fabric used for
    // this task.
    let partition_config = subscriber_fabric.runtime.partition_config;
    let eid =
        id_map::session_task_to_execution_id(&project, &session_id, &task_id, &partition_config);
    let partition = execution_partition(&eid, &partition_config);
    let ctx = ExecKeyContext::new(&partition, &eid);

    // Backdate the lease so ff_mark_lease_expired_if_due's "actually
    // expired" guard passes.
    let _: ferriskey::Value = subscriber_fabric
        .runtime
        .client
        .cmd("HSET")
        .arg(ctx.core())
        .arg("lease_expires_at")
        .arg("0")
        .execute()
        .await
        .expect("HSET lease_expires_at");

    // Fire the expiry FCALL. Signature per lua/lease.lua:
    //   KEYS (4): exec_core, lease_current, lease_expiry_zset, lease_history
    //   ARGV (1): execution_id
    let index = flowfabric::core::keys::IndexKeys::new(&partition);
    let _: ferriskey::Value = subscriber_fabric
        .runtime
        .client
        .fcall(
            "ff_mark_lease_expired_if_due",
            &[
                ctx.core(),
                ctx.lease_current(),
                index.lease_expiry(),
                ctx.lease_history(),
            ],
            &[eid.as_str().to_owned()],
        )
        .await
        .expect("ff_mark_lease_expired_if_due");

    // Poll the TaskReadModel for up to 5s. The subscriber polls the
    // ZSET every 1s; once it hits the new stream it XREADs, emits,
    // the EventBridge consumer runs, projection applies. Total
    // latency budget: ~2s worst-case; 5s gives generous headroom
    // for CI.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let final_state = loop {
        let rec = TaskReadModel::get(harness.event_log.as_ref(), &task_id)
            .await
            .expect("TaskReadModel::get")
            .expect("task not in projection");
        if rec.state == TaskState::RetryableFailed {
            break rec;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timeout waiting for RetryableFailed; last state = {:?}, failure = {:?}",
                rec.state, rec.failure_class
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };

    assert_eq!(final_state.state, TaskState::RetryableFailed);
    assert_eq!(final_state.failure_class, Some(FailureClass::LeaseExpired));

    subscriber_fabric.shutdown().await;
    harness.teardown().await;
}
