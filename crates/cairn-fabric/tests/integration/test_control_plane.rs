//! Integration tests for the [`ControlPlaneBackend`] trait and the
//! [`Engine`]'s worker methods (Phase D PR 1).
//!
//! Exercises the FCALL-shaped control-plane trait (budget / quota /
//! rotation) plus the new worker-registry methods against a live
//! Valkey. These tests are the regression net for the Phase D PR 1
//! split — if `FabricBudgetService`'s delegation to the trait stops
//! preserving outcome variants, or the Engine's worker HSET layout
//! drifts, these fail.
//!
//! Note: rotation is exercised against the shared Valkey with the
//! SAME `(kid, secret)` every TestHarness instance seeds (see
//! `integration.rs`'s HMAC footgun banner). The test therefore
//! relies on the `noop` path for its assertion — we verify the
//! FCALL fan-out round-trips without changing state.
//!
//! [`ControlPlaneBackend`]: cairn_fabric::engine::ControlPlaneBackend
//! [`Engine`]: cairn_fabric::engine::Engine

use std::collections::HashMap;

use cairn_fabric::engine::{
    AddExecutionToFlowInput, ApplyDependencyToChildInput, BudgetSpendOutcome, CancelFlowInput,
    CreateFlowInput, EligibilityResult, FlowCancelOutcome, QuotaAdmission,
    StageDependencyEdgeInput, StageDependencyOutcome, SubmitTaskInput,
};
use flowfabric::core::types::{
    BudgetId, EdgeId, ExecutionId, FlowId, LaneId, Namespace, WorkerId, WorkerInstanceId,
};

use crate::TestHarness;

fn test_eid(h: &TestHarness, seed: &str) -> ExecutionId {
    let uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, seed.as_bytes());
    ExecutionId::deterministic_solo(&LaneId::new("test"), h.partition_config(), uuid)
}

// ── Budget via ControlPlaneBackend ──────────────────────────────────────

#[tokio::test]
async fn control_plane_budget_create_spend_release_roundtrip() {
    let h = TestHarness::setup().await;
    let run_id = cairn_domain::RunId::new(format!("cp_budget_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 200, 1_000_000, 50)
        .await
        .expect("create_run_budget");

    let eid = test_eid(&h, "cp_budget_spend");
    let first = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid, &[("tokens", 50)])
        .await
        .expect("record_spend");
    assert_eq!(first, BudgetSpendOutcome::Ok, "first spend must land fresh");

    // Second identical spend (same execution, same deltas) — dedup must fire.
    let second = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid, &[("tokens", 50)])
        .await
        .expect("record_spend second");
    assert_eq!(
        second,
        BudgetSpendOutcome::AlreadyApplied,
        "dedup must fire on identical inputs"
    );

    // Status reflects the ONE applied spend.
    let status = h
        .fabric
        .budgets
        .get_budget_status(&budget_id)
        .await
        .expect("get_budget_status");
    assert_eq!(*status.usage.get("tokens").unwrap_or(&0), 50);

    // Release resets usage.
    h.fabric
        .budgets
        .release_budget(&budget_id)
        .await
        .expect("release_budget");
    let post_status = h
        .fabric
        .budgets
        .get_budget_status(&budget_id)
        .await
        .expect("get_budget_status post-release");
    // After reset, FF clears the usage hash — field absent is equivalent
    // to 0 from the admin-read perspective.
    assert_eq!(*post_status.usage.get("tokens").unwrap_or(&0), 0);

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_budget_get_status_missing_returns_not_found() {
    let h = TestHarness::setup().await;
    let missing = BudgetId::new();
    let err = h
        .fabric
        .budgets
        .get_budget_status(&missing)
        .await
        .expect_err("absent budget must error");
    match err {
        cairn_fabric::FabricError::NotFound { entity, .. } => assert_eq!(entity, "budget"),
        other => panic!("expected NotFound budget, got {other:?}"),
    }

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_budget_hard_breach_preserves_dimension() {
    let h = TestHarness::setup().await;
    let run_id = cairn_domain::RunId::new(format!("cp_hard_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 100, 1_000_000, 10)
        .await
        .expect("create_run_budget");

    // First spend below limit.
    let eid_a = test_eid(&h, "cp_hard_a");
    let _ = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid_a, &[("tokens", 60)])
        .await
        .expect("first spend");

    // Second spend pushes over the hard limit of 100.
    let eid_b = test_eid(&h, "cp_hard_b");
    let breach = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid_b, &[("tokens", 50)])
        .await
        .expect("breach spend");
    assert!(
        matches!(
            breach,
            BudgetSpendOutcome::HardBreach { ref dimension, .. } if dimension == "tokens"
        ),
        "expected HardBreach on tokens, got {breach:?}"
    );

    h.teardown().await;
}

// ── Quota via ControlPlaneBackend ───────────────────────────────────────

#[tokio::test]
async fn control_plane_quota_admission_admits_under_limit() {
    let h = TestHarness::setup().await;

    let qid = h
        .fabric
        .quotas
        .create_quota_policy("run", "cp_quota", 60, 100, 10)
        .await
        .expect("create_quota_policy");

    let eid = test_eid(&h, "cp_quota_admit");
    let outcome = h
        .fabric
        .quotas
        .check_admission(&qid, &eid, 60, 100, 10)
        .await
        .expect("check_admission");
    assert_eq!(outcome, QuotaAdmission::Admitted);

    // Second check with the SAME execution_id must dedup to
    // AlreadyAdmitted.
    let repeat = h
        .fabric
        .quotas
        .check_admission(&qid, &eid, 60, 100, 10)
        .await
        .expect("check_admission repeat");
    assert_eq!(repeat, QuotaAdmission::AlreadyAdmitted);

    h.teardown().await;
}

// ── Rotation via ControlPlaneBackend ────────────────────────────────────

#[tokio::test]
async fn control_plane_rotation_noop_on_seeded_secret() {
    // The harness seeds a deterministic (kid, secret) on every
    // partition at startup. Re-submitting the SAME pair must return
    // `noop` for every partition — no real rotation happens. This is
    // the only rotation test that's safe against the shared Valkey;
    // see integration.rs for the HMAC-ROTATION footgun banner.
    let h = TestHarness::setup().await;
    let num_partitions = h.partition_config().num_flow_partitions;

    let outcome = h
        .fabric
        .rotation
        .rotate_waitpoint_hmac(
            "cairn-test-k1",
            "00000000000000000000000000000000000000000000000000000000000000aa",
            60_000,
        )
        .await;

    assert_eq!(outcome.new_kid, "cairn-test-k1");
    assert!(
        outcome.failed.is_empty(),
        "no partition must fail the replay, got {:?}",
        outcome.failed
    );
    // Rotated may be 0 or num_partitions depending on whether another
    // test in this suite ran a first-time seed — the fabric boot path
    // already did the initial rotate on harness setup, so the typical
    // case is all `noop`. Either way the count of (rotated + noop)
    // must equal the partition count.
    assert_eq!(
        outcome.rotated as u32 + outcome.noop as u32,
        num_partitions as u32,
        "every partition must be accounted for"
    );

    h.teardown().await;
}

// ── Worker registry via Engine ──────────────────────────────────────────

// ── Run / session lifecycle via ControlPlaneBackend (Phase D PR 2a) ─────
//
// The fabric-service-level tests in test_run_lifecycle.rs and
// test_session.rs exercise the same trait through the service shim;
// these tests hit the trait directly so a regression in the trait
// impl's FCALL dispatch is caught without the service layer masking
// it.

#[tokio::test]
async fn control_plane_create_and_cancel_flow_roundtrip() {
    let h = TestHarness::setup().await;
    let flow_id = FlowId::from_uuid(uuid::Uuid::new_v4());
    let namespace = Namespace::new("cp_flow_ns");

    h.fabric
        .control_plane
        .create_flow(CreateFlowInput {
            flow_id: flow_id.clone(),
            flow_kind: "cairn_session".to_owned(),
            namespace: namespace.clone(),
        })
        .await
        .expect("create_flow");

    // Idempotent re-create must NOT error (FF replies ok_already_satisfied).
    h.fabric
        .control_plane
        .create_flow(CreateFlowInput {
            flow_id: flow_id.clone(),
            flow_kind: "cairn_session".to_owned(),
            namespace,
        })
        .await
        .expect("create_flow idempotent");

    // First cancel must land as Cancelled.
    let first = h
        .fabric
        .control_plane
        .cancel_flow(CancelFlowInput {
            flow_id: flow_id.clone(),
            reason: "test".to_owned(),
            cancel_mode: "cancel_all".to_owned(),
        })
        .await
        .expect("cancel_flow first");
    assert_eq!(first, FlowCancelOutcome::Cancelled);

    // Re-cancel on a terminal flow must surface the typed
    // AlreadyTerminal variant — services depend on this for
    // idempotent archive semantics.
    let second = h
        .fabric
        .control_plane
        .cancel_flow(CancelFlowInput {
            flow_id,
            reason: "test".to_owned(),
            cancel_mode: "cancel_all".to_owned(),
        })
        .await
        .expect("cancel_flow second");
    assert_eq!(second, FlowCancelOutcome::AlreadyTerminal);

    h.teardown().await;
}

// Note: `issue_grant_and_claim`, `create_run_execution`,
// `complete_run_execution`, `fail_run_execution`, `cancel_run_execution`,
// `suspend_run_execution`, `resume_run_execution`, and
// `deliver_approval_signal` are exercised end-to-end by
// `test_run_lifecycle`, `test_suspension`, and `test_session` through
// the service shims that delegate to this trait. Direct-trait tests
// would duplicate those paths — the service-level assertions catch
// regressions closer to where operators actually see the behaviour.

#[tokio::test]
async fn engine_register_heartbeat_mark_dead_roundtrip() {
    let h = TestHarness::setup().await;
    let wid = WorkerId::new(format!("cp_w_{}", uuid::Uuid::new_v4()));
    let iid = WorkerInstanceId::new(format!("cp_i_{}", uuid::Uuid::new_v4()));
    let caps = vec!["gpu=true".to_owned(), "linux=x86_64".to_owned()];

    let reg = h
        .fabric
        .worker
        .register_worker(&wid, &iid, &caps)
        .await
        .expect("register_worker");
    assert_eq!(reg.worker_id, wid);
    assert_eq!(reg.instance_id, iid);
    assert_eq!(reg.capabilities.len(), 2);
    assert!(
        reg.registered_at_ms > 1_700_000_000_000,
        "registered_at_ms must be a real epoch ms, got {}",
        reg.registered_at_ms
    );

    // Heartbeat must succeed.
    h.fabric
        .worker
        .heartbeat_worker(&iid)
        .await
        .expect("heartbeat_worker");

    // Mark dead must succeed (idempotent — no pre-state required).
    h.fabric
        .worker
        .mark_worker_dead(&iid)
        .await
        .expect("mark_worker_dead");

    h.teardown().await;
}

// ── Phase D PR 2b: task lifecycle through ControlPlaneBackend ───────────

#[tokio::test]
async fn control_plane_submit_task_is_idempotent() {
    let h = TestHarness::setup().await;
    let task_id = cairn_domain::TaskId::new(format!("cp_task_{}", uuid::Uuid::new_v4()));
    let eid =
        cairn_fabric::id_map::task_to_execution_id(&h.project, &task_id, h.partition_config());
    let namespace = cairn_fabric::id_map::tenant_to_namespace(&h.project.tenant_id);
    let lane = cairn_fabric::id_map::project_to_lane(&h.project);

    let mut tags = HashMap::new();
    tags.insert("cairn.task_id".to_owned(), task_id.as_str().to_owned());
    tags.insert(
        "cairn.project".to_owned(),
        format!(
            "{}/{}/{}",
            h.project.tenant_id, h.project.workspace_id, h.project.project_id
        ),
    );

    let first = h
        .fabric
        .control_plane
        .submit_task_execution(SubmitTaskInput {
            execution_id: eid.clone(),
            namespace: namespace.clone(),
            lane_id: lane.clone(),
            priority: 7,
            tags: tags.clone(),
            policy_json: String::new(),
        })
        .await
        .expect("submit_task_execution");
    assert!(
        first.newly_created,
        "first submit must report newly_created"
    );

    // Idempotent replay: same ExecutionId must return newly_created=false
    // and NOT error — projections rely on this to skip the second emit.
    let second = h
        .fabric
        .control_plane
        .submit_task_execution(SubmitTaskInput {
            execution_id: eid,
            namespace,
            lane_id: lane,
            priority: 7,
            tags,
            policy_json: String::new(),
        })
        .await
        .expect("submit_task_execution idempotent");
    assert!(
        !second.newly_created,
        "idempotent submit must report !newly_created"
    );

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_add_execution_to_flow_is_idempotent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = cairn_domain::TaskId::new(format!("cp_flow_task_{}", uuid::Uuid::new_v4()));
    let fid = cairn_fabric::id_map::session_to_flow_id(&h.project, &session_id);
    let eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_id,
        h.partition_config(),
    );
    let namespace = cairn_fabric::id_map::tenant_to_namespace(&h.project.tenant_id);

    // First submit the task so it exists, then bind.
    let mut tags = HashMap::new();
    tags.insert("cairn.task_id".to_owned(), task_id.as_str().to_owned());
    tags.insert(
        "cairn.session_id".to_owned(),
        session_id.as_str().to_owned(),
    );
    h.fabric
        .control_plane
        .submit_task_execution(SubmitTaskInput {
            execution_id: eid.clone(),
            namespace: namespace.clone(),
            lane_id: cairn_fabric::id_map::project_to_lane(&h.project),
            priority: 0,
            tags,
            policy_json: String::new(),
        })
        .await
        .expect("submit_task_execution");

    // First bind — creates flow + adds member.
    h.fabric
        .control_plane
        .add_execution_to_flow(AddExecutionToFlowInput {
            flow_id: fid.clone(),
            execution_id: eid.clone(),
            namespace: namespace.clone(),
            flow_kind: "cairn_session".to_owned(),
        })
        .await
        .expect("add_execution_to_flow");

    // Idempotent replay — must not error.
    h.fabric
        .control_plane
        .add_execution_to_flow(AddExecutionToFlowInput {
            flow_id: fid,
            execution_id: eid,
            namespace,
            flow_kind: "cairn_session".to_owned(),
        })
        .await
        .expect("add_execution_to_flow idempotent");

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_stage_dependency_already_exists_on_replay() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();
    let task_b = h.unique_task_id();

    // Submit both tasks via the service (which also binds them to the flow).
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

    let fid = cairn_fabric::id_map::session_to_flow_id(&h.project, &session_id);
    let pre_eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_a,
        h.partition_config(),
    );
    let dep_eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_b,
        h.partition_config(),
    );
    let edge_id = cairn_fabric::id_map::dependency_edge_id(&fid, &pre_eid, &dep_eid);

    // Read current graph_revision so the first stage lands clean.
    let current_rev = h
        .fabric
        .engine
        .describe_flow(&fid)
        .await
        .expect("describe_flow")
        .map(|s| s.graph_revision)
        .unwrap_or(0);

    let first = h
        .fabric
        .control_plane
        .stage_dependency_edge(StageDependencyEdgeInput {
            flow_id: fid.clone(),
            edge_id: edge_id.clone(),
            upstream_execution_id: pre_eid.clone(),
            downstream_execution_id: dep_eid.clone(),
            dependency_kind: "success_only".to_owned(),
            data_passing_ref: String::new(),
            expected_graph_revision: current_rev,
        })
        .await
        .expect("stage first");
    let new_rev = match first {
        StageDependencyOutcome::Staged { new_graph_revision } => new_graph_revision,
        other => panic!("expected Staged, got {other:?}"),
    };
    assert!(new_rev > current_rev, "graph_revision must advance");

    // Apply to child so the edge reaches its terminal shape.
    h.fabric
        .control_plane
        .apply_dependency_to_child(ApplyDependencyToChildInput {
            downstream_execution_id: dep_eid.clone(),
            flow_id: fid.clone(),
            upstream_execution_id: pre_eid.clone(),
            edge_id: edge_id.clone(),
            lane_id: cairn_fabric::id_map::project_to_lane(&h.project),
            graph_revision: new_rev,
            dependency_kind: "success_only".to_owned(),
            data_passing_ref: String::new(),
        })
        .await
        .expect("apply first");

    // Replay stage must surface AlreadyExists (service uses this to
    // trigger describe_edge reconcile).
    let second = h
        .fabric
        .control_plane
        .stage_dependency_edge(StageDependencyEdgeInput {
            flow_id: fid,
            edge_id,
            upstream_execution_id: pre_eid,
            downstream_execution_id: dep_eid,
            dependency_kind: "success_only".to_owned(),
            data_passing_ref: String::new(),
            expected_graph_revision: new_rev,
        })
        .await
        .expect("stage replay");
    assert!(
        matches!(second, StageDependencyOutcome::AlreadyExists),
        "replay must surface AlreadyExists, got {second:?}"
    );

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_stage_dependency_self_referencing_is_typed() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();

    h.fabric
        .tasks
        .submit(&h.project, task_a.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit A");

    let fid = cairn_fabric::id_map::session_to_flow_id(&h.project, &session_id);
    let a_eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_a,
        h.partition_config(),
    );
    // Intentionally build a self-referencing edge id (upstream==downstream).
    let edge_id = EdgeId::new();

    let outcome = h
        .fabric
        .control_plane
        .stage_dependency_edge(StageDependencyEdgeInput {
            flow_id: fid,
            edge_id,
            upstream_execution_id: a_eid.clone(),
            downstream_execution_id: a_eid,
            dependency_kind: "success_only".to_owned(),
            data_passing_ref: String::new(),
            expected_graph_revision: 0,
        })
        .await
        .expect("stage self-ref");
    assert!(
        matches!(outcome, StageDependencyOutcome::SelfReferencing),
        "self-ref must surface as typed variant, got {outcome:?}"
    );

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_evaluate_flow_eligibility_blocks_when_dep_present() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();
    let task_b = h.unique_task_id();

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

    // Declare B depends on A via the service (exercises the full
    // retry loop + apply path).
    h.fabric
        .tasks
        .declare_dependency(
            &h.project,
            &session_id,
            &task_b,
            &task_a,
            cairn_domain::DependencyKind::SuccessOnly,
            None,
        )
        .await
        .expect("declare");

    // B's partition should report blocked_by_dependencies through the
    // trait surface.
    let b_eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_b,
        h.partition_config(),
    );
    let result = h
        .fabric
        .control_plane
        .evaluate_flow_eligibility(&b_eid)
        .await
        .expect("evaluate_flow_eligibility");
    assert_eq!(
        result,
        EligibilityResult::BlockedByDependencies,
        "B must report BlockedByDependencies"
    );

    h.teardown().await;
}

#[tokio::test]
async fn engine_list_expired_leases_returns_empty_when_none_present() {
    let h = TestHarness::setup().await;
    // Pick a near-zero `now_ms` so the ZRANGEBYSCORE window is empty
    // regardless of whether sibling tests populated lease_expiry keys.
    let expired = h
        .fabric
        .engine
        .list_expired_leases(0, 16)
        .await
        .expect("list_expired_leases");
    assert!(
        expired.is_empty(),
        "zero-upper-bound scan must return empty, got {} rows",
        expired.len()
    );

    h.teardown().await;
}
