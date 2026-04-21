//! Integration tests for the `Engine` trait against a live Valkey.
//!
//! Exercises the four public trait methods (`describe_execution`,
//! `describe_flow`, `describe_edge`, `list_incoming_edges`) against
//! freshly-submitted entities to verify the typed snapshots cairn
//! builds off them match what services need.
//!
//! These tests are the regression net for PR A+B — if FF renames a
//! hash field or drops a hash key, either these tests fail or they
//! surface the silent drift (blanks / zeros where real values should
//! be).

use cairn_domain::FailureClass;
use cairn_fabric::engine::EdgeState;

use crate::TestHarness;

#[tokio::test]
async fn describe_execution_on_fresh_task_returns_typed_snapshot() {
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
        .expect("submit task");

    let eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_id,
        h.partition_config(),
    );

    let snap = h
        .fabric
        .engine
        .describe_execution(&eid)
        .await
        .expect("describe_execution")
        .expect("freshly-submitted task must have an execution snapshot");

    assert_eq!(snap.execution_id, eid);
    // Freshly-submitted task: eligible, no blockers, no lease.
    assert!(!snap.public_state.is_empty(), "public_state populated");
    assert!(
        snap.current_lease.is_none(),
        "no active lease on fresh submit"
    );
    // Tags carry cairn scope stamped at submit time.
    assert_eq!(
        snap.tags.get("cairn.task_id").map(String::as_str),
        Some(task_id.as_str()),
        "cairn.task_id tag round-trips",
    );
    assert_eq!(
        snap.tags.get("cairn.session_id").map(String::as_str),
        Some(session_id.as_str()),
    );

    h.teardown().await;
}

#[tokio::test]
async fn describe_execution_on_missing_execution_returns_none() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let absent_task = h.unique_task_id();

    // Mint an ExecutionId for a task that was never submitted — the
    // derivation is deterministic, so the id is valid but the Valkey
    // key doesn't exist.
    let eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &absent_task,
        h.partition_config(),
    );

    let result = h
        .fabric
        .engine
        .describe_execution(&eid)
        .await
        .expect("describe_execution");

    assert!(
        result.is_none(),
        "absent execution must yield Ok(None), got {result:?}"
    );

    h.teardown().await;
}

#[tokio::test]
async fn describe_flow_on_fresh_session_returns_typed_snapshot() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();

    h.fabric
        .sessions
        .create(&h.project, session_id.clone())
        .await
        .expect("session create");

    let fid = cairn_fabric::id_map::session_to_flow_id(&h.project, &session_id);
    let snap = h
        .fabric
        .engine
        .describe_flow(&fid)
        .await
        .expect("describe_flow")
        .expect("freshly-created session must have a flow snapshot");

    assert_eq!(snap.flow_id, fid);
    assert_eq!(snap.kind, "cairn_session");
    assert!(
        !snap.public_flow_state.is_empty(),
        "public_flow_state populated (typically \"open\")"
    );
    // graph_revision starts at 0 on a fresh flow per FF's ff_create_flow Lua.
    assert_eq!(snap.graph_revision, 0);
    // Tags populated by sessions.create (cairn.project + cairn.session_id
    // are HSET post-create today — will move to engine.set_flow_tag in
    // Phase C).
    assert_eq!(
        snap.tags.get("cairn.session_id").map(String::as_str),
        Some(session_id.as_str()),
    );

    h.teardown().await;
}

#[tokio::test]
async fn describe_flow_on_missing_flow_returns_none() {
    let h = TestHarness::setup().await;
    let absent_session = h.unique_session_id();
    let fid = cairn_fabric::id_map::session_to_flow_id(&h.project, &absent_session);

    let result = h
        .fabric
        .engine
        .describe_flow(&fid)
        .await
        .expect("describe_flow");
    assert!(result.is_none());

    h.teardown().await;
}

#[tokio::test]
async fn describe_edge_and_list_incoming_edges_on_staged_dependency() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();
    let task_b = h.unique_task_id();

    // Submit two session-bound tasks and declare B depends on A.
    h.fabric
        .tasks
        .submit(&h.project, task_a.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit A");
    h.fabric
        .tasks
        .submit(&h.project, task_b.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit B");
    h.fabric
        .tasks
        .declare_dependency(
            &h.project,
            &session_id,
            &task_b,
            &task_a,
            cairn_domain::DependencyKind::SuccessOnly,
            Some("engine-test-ref"),
        )
        .await
        .expect("declare_dependency");

    let fid = cairn_fabric::id_map::session_to_flow_id(&h.project, &session_id);
    let task_a_eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_a,
        h.partition_config(),
    );
    let task_b_eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_b,
        h.partition_config(),
    );
    let edge_id = cairn_fabric::id_map::dependency_edge_id(&fid, &task_a_eid, &task_b_eid);

    // describe_edge — the edge hash on the flow side.
    let edge = h
        .fabric
        .engine
        .describe_edge(&fid, &edge_id)
        .await
        .expect("describe_edge")
        .expect("staged edge must be readable");
    assert_eq!(edge.edge_id, edge_id);
    assert_eq!(edge.flow_id, fid);
    assert_eq!(edge.upstream_execution_id, task_a_eid);
    assert_eq!(edge.downstream_execution_id, task_b_eid);
    assert_eq!(edge.kind, "success_only");
    assert_eq!(edge.data_passing_ref.as_deref(), Some("engine-test-ref"));

    // list_incoming_edges on task_b — should include exactly one
    // unsatisfied edge pointing at task_a.
    let incoming = h
        .fabric
        .engine
        .list_incoming_edges(&task_b_eid)
        .await
        .expect("list_incoming_edges");
    assert_eq!(incoming.len(), 1);
    let blocker = &incoming[0];
    assert_eq!(blocker.upstream_execution_id, task_a_eid);
    assert_eq!(blocker.state, EdgeState::Unsatisfied);
    assert_eq!(blocker.data_passing_ref.as_deref(), Some("engine-test-ref"));

    // list_incoming_edges on task_a — no blockers.
    let a_incoming = h
        .fabric
        .engine
        .list_incoming_edges(&task_a_eid)
        .await
        .expect("list_incoming_edges on A");
    assert!(a_incoming.is_empty());

    h.teardown().await;
}

#[tokio::test]
async fn describe_execution_after_claim_surfaces_lease_summary() {
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
        .expect("submit");
    h.fabric
        .tasks
        .claim(
            &h.project,
            Some(&session_id),
            &task_id,
            "engine-test-worker".into(),
            30_000,
        )
        .await
        .expect("claim");

    let eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_id,
        h.partition_config(),
    );
    let snap = h
        .fabric
        .engine
        .describe_execution(&eid)
        .await
        .expect("describe_execution")
        .expect("snapshot present");

    let lease = snap.current_lease.expect("claimed execution has a lease");
    assert!(lease.epoch.0 >= 1, "epoch monotonic ≥ 1 after claim");
    assert!(lease.expires_at.0 > 0, "expires_at populated after claim");
    assert!(!lease.owner.is_empty(), "owner populated");
    // Sanity: clean up so the shared Valkey doesn't keep the leased
    // state around (other tests see it via different ProjectKeys so
    // no contamination, but the fail path clears it anyway).
    let _ = h
        .fabric
        .tasks
        .fail(
            &h.project,
            Some(&session_id),
            &task_id,
            FailureClass::ExecutionError,
        )
        .await;

    h.teardown().await;
}
