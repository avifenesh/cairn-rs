//! RFC 005 approval blocking integration tests.
//!
//! Validates the approval gating pipeline through InMemoryStore:
//! - ApprovalRequested events project into ApprovalReadModel.
//! - list_pending returns only unresolved approvals for the correct project.
//! - Multiple approvals for the same run are tracked independently.
//! - Approval policy linkage: ApprovalPolicyRecord is findable via policy_id.
//! - Cross-project isolation: approvals are scoped to their project.
//! - ApprovalRecord.version increments on each ApprovalResolved event.

use std::sync::Arc;

use cairn_domain::events::RunStateChanged;
use cairn_domain::lifecycle::RunState;
use cairn_domain::policy::{ApprovalDecision, ApprovalRequirement};
use cairn_domain::{
    ApprovalId, ApprovalPolicyCreated, ApprovalRequested, ApprovalResolved, EventEnvelope, EventId,
    EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent, SessionCreated, SessionId,
    StateTransition, TenantId,
};
use cairn_store::{
    projections::{ApprovalPolicyReadModel, ApprovalReadModel, RunReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project_a() -> ProjectKey {
    ProjectKey::new("tenant_appr", "ws_appr", "proj_a")
}

fn project_b() -> ProjectKey {
    ProjectKey::new("tenant_appr", "ws_appr", "proj_b")
}

fn tenant_id() -> TenantId {
    TenantId::new("tenant_appr")
}

fn run_id(n: &str) -> RunId {
    RunId::new(format!("run_appr_{n}"))
}

fn approval_id(n: &str) -> ApprovalId {
    ApprovalId::new(format!("appr_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

/// Seed a session + run in a given project.
async fn seed_run(store: &Arc<InMemoryStore>, project: &ProjectKey, run: &RunId) {
    let sess = SessionId::new(format!("sess_{}", run.as_str()));
    store
        .append(&[
            ev(
                &format!("evt_sess_{}", run.as_str()),
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: sess.clone(),
                }),
            ),
            ev(
                &format!("evt_run_{}", run.as_str()),
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: sess,
                    run_id: run.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();
}

/// Transition a run to WaitingApproval state.
async fn set_run_waiting_approval(store: &Arc<InMemoryStore>, project: &ProjectKey, run: &RunId) {
    store
        .append(&[
            ev(
                &format!("evt_run_start_{}", run.as_str()),
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project.clone(),
                    run_id: run.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
            ev(
                &format!("evt_run_wait_{}", run.as_str()),
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project.clone(),
                    run_id: run.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::WaitingApproval,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();
}

/// Append a single ApprovalRequested event for a run.
async fn request_approval(
    store: &Arc<InMemoryStore>,
    project: &ProjectKey,
    appr_id: &ApprovalId,
    run: &RunId,
) {
    store
        .append(&[ev(
            &format!("evt_appr_req_{}", appr_id.as_str()),
            RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project.clone(),
                approval_id: appr_id.clone(),
                run_id: Some(run.clone()),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            }),
        )])
        .await
        .unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Create run in WaitingApproval state.
/// (2) Verify ApprovalReadModel shows the pending approval.
#[tokio::test]
async fn run_in_waiting_approval_state_shows_pending() {
    let store = Arc::new(InMemoryStore::new());

    seed_run(&store, &project_a(), &run_id("1")).await;
    set_run_waiting_approval(&store, &project_a(), &run_id("1")).await;
    request_approval(&store, &project_a(), &approval_id("1"), &run_id("1")).await;

    // Step 1: run state must be WaitingApproval.
    let run = RunReadModel::get(store.as_ref(), &run_id("1"))
        .await
        .unwrap()
        .expect("run must exist");
    assert_eq!(
        run.state,
        RunState::WaitingApproval,
        "run must be in WaitingApproval state"
    );

    // Step 2: ApprovalReadModel must show the pending approval.
    let rec = ApprovalReadModel::get(store.as_ref(), &approval_id("1"))
        .await
        .unwrap()
        .expect("approval record must exist after ApprovalRequested");

    assert_eq!(rec.approval_id, approval_id("1"));
    assert_eq!(rec.run_id, Some(run_id("1")));
    assert!(
        rec.decision.is_none(),
        "approval must be pending (no decision yet)"
    );
    assert_eq!(rec.requirement, ApprovalRequirement::Required);
    assert_eq!(rec.version, 1, "initial version must be 1");

    // list_pending must include this approval.
    let pending = ApprovalReadModel::list_pending(store.as_ref(), &project_a(), 10, 0)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "one pending approval must be listed");
    assert_eq!(pending[0].approval_id, approval_id("1"));
}

/// (3) Multiple approvals for the same run are tracked independently.
#[tokio::test]
async fn multiple_approvals_for_same_run_tracked_independently() {
    let store = Arc::new(InMemoryStore::new());

    seed_run(&store, &project_a(), &run_id("multi")).await;
    set_run_waiting_approval(&store, &project_a(), &run_id("multi")).await;

    // Append 3 approvals for the same run (e.g. multi-step approval chain).
    for n in ["x", "y", "z"] {
        request_approval(&store, &project_a(), &approval_id(n), &run_id("multi")).await;
    }

    // All three must be independently retrievable.
    for n in ["x", "y", "z"] {
        let rec = ApprovalReadModel::get(store.as_ref(), &approval_id(n))
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("approval_{n} must exist"));
        assert_eq!(rec.run_id, Some(run_id("multi")));
        assert!(rec.decision.is_none(), "approval_{n} must still be pending");
    }

    // list_pending returns all 3 distinct records.
    let pending = ApprovalReadModel::list_pending(store.as_ref(), &project_a(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        3,
        "all 3 approvals must appear in pending list"
    );

    let ids: Vec<&str> = pending.iter().map(|r| r.approval_id.as_str()).collect();
    assert!(ids.contains(&"appr_x"), "appr_x must be in pending list");
    assert!(ids.contains(&"appr_y"), "appr_y must be in pending list");
    assert!(ids.contains(&"appr_z"), "appr_z must be in pending list");

    // Resolve one approval — the other two remain pending.
    store
        .append(&[ev(
            "evt_appr_resolve_x",
            RuntimeEvent::ApprovalResolved(ApprovalResolved {
                project: project_a(),
                approval_id: approval_id("x"),
                decision: ApprovalDecision::Approved,
            }),
        )])
        .await
        .unwrap();

    let still_pending = ApprovalReadModel::list_pending(store.as_ref(), &project_a(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        still_pending.len(),
        2,
        "after resolving appr_x, only 2 approvals must remain pending"
    );
    assert!(!still_pending
        .iter()
        .any(|r| r.approval_id.as_str() == "appr_x"));
}

/// (4) Approval with policy_id links to ApprovalPolicyRecord via ApprovalPolicyReadModel.
#[tokio::test]
async fn approval_policy_record_is_linkable() {
    let store = Arc::new(InMemoryStore::new());

    // Create an approval policy.
    store
        .append(&[ev(
            "evt_policy_created",
            RuntimeEvent::ApprovalPolicyCreated(ApprovalPolicyCreated {
                project: project_a(),
                policy_id: "policy_senior_review".to_owned(),
                tenant_id: tenant_id(),
                name: "Senior Review Required".to_owned(),
                required_approvers: 2,
                allowed_approver_roles: vec![],
                auto_approve_after_ms: None,
                auto_reject_after_ms: Some(86_400_000), // 24h auto-reject
                created_at_ms: 1_000_000,
            }),
        )])
        .await
        .unwrap();

    seed_run(&store, &project_a(), &run_id("policy")).await;
    set_run_waiting_approval(&store, &project_a(), &run_id("policy")).await;
    request_approval(
        &store,
        &project_a(),
        &approval_id("policy"),
        &run_id("policy"),
    )
    .await;

    // The approval record can be fetched.
    let appr = ApprovalReadModel::get(store.as_ref(), &approval_id("policy"))
        .await
        .unwrap()
        .unwrap();
    assert!(appr.decision.is_none(), "approval must be pending");

    // The policy record is independently queryable via ApprovalPolicyReadModel.
    let policy = ApprovalPolicyReadModel::get_policy(store.as_ref(), "policy_senior_review")
        .await
        .unwrap()
        .expect("policy record must exist after ApprovalPolicyCreated");

    assert_eq!(policy.policy_id, "policy_senior_review");
    assert_eq!(policy.name, "Senior Review Required");
    assert_eq!(policy.required_approvers, 2);
    assert_eq!(policy.auto_reject_after_ms, Some(86_400_000));

    // The policy tenant matches the approval's project tenant.
    assert_eq!(policy.tenant_id, tenant_id());

    // Listing policies by tenant returns the policy.
    let tenant_policies =
        ApprovalPolicyReadModel::list_by_tenant(store.as_ref(), &tenant_id(), 10, 0)
            .await
            .unwrap();
    assert_eq!(tenant_policies.len(), 1);
    assert_eq!(tenant_policies[0].policy_id, "policy_senior_review");
}

/// (5) Cross-project isolation: list_pending for project_a must not return
/// approvals from project_b and vice versa.
#[tokio::test]
async fn approvals_are_isolated_per_project() {
    let store = Arc::new(InMemoryStore::new());

    // Seed and request approvals in both projects.
    seed_run(&store, &project_a(), &run_id("proj_a")).await;
    set_run_waiting_approval(&store, &project_a(), &run_id("proj_a")).await;
    request_approval(
        &store,
        &project_a(),
        &approval_id("proj_a_1"),
        &run_id("proj_a"),
    )
    .await;
    request_approval(
        &store,
        &project_a(),
        &approval_id("proj_a_2"),
        &run_id("proj_a"),
    )
    .await;

    seed_run(&store, &project_b(), &run_id("proj_b")).await;
    set_run_waiting_approval(&store, &project_b(), &run_id("proj_b")).await;
    request_approval(
        &store,
        &project_b(),
        &approval_id("proj_b_1"),
        &run_id("proj_b"),
    )
    .await;

    // Project A sees only its own approvals.
    let a_pending = ApprovalReadModel::list_pending(store.as_ref(), &project_a(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        a_pending.len(),
        2,
        "project_a must see exactly 2 pending approvals"
    );
    assert!(
        a_pending.iter().all(|r| r.project == project_a()),
        "all project_a approvals must be scoped to project_a"
    );
    assert!(
        !a_pending
            .iter()
            .any(|r| r.approval_id.as_str() == "appr_proj_b_1"),
        "project_a must not see project_b's approval"
    );

    // Project B sees only its own approval.
    let b_pending = ApprovalReadModel::list_pending(store.as_ref(), &project_b(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        b_pending.len(),
        1,
        "project_b must see exactly 1 pending approval"
    );
    assert_eq!(b_pending[0].approval_id.as_str(), "appr_proj_b_1");
    assert!(
        !b_pending
            .iter()
            .any(|r| r.approval_id.as_str().starts_with("appr_proj_a")),
        "project_b must not see project_a's approvals"
    );
}

/// (6) ApprovalRecord.version increments on each ApprovalResolved event.
#[tokio::test]
async fn approval_version_increments_on_resolve() {
    let store = Arc::new(InMemoryStore::new());

    seed_run(&store, &project_a(), &run_id("ver")).await;
    set_run_waiting_approval(&store, &project_a(), &run_id("ver")).await;
    request_approval(&store, &project_a(), &approval_id("ver"), &run_id("ver")).await;

    // Initial version must be 1.
    let before = ApprovalReadModel::get(store.as_ref(), &approval_id("ver"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.version, 1, "initial approval version must be 1");
    assert!(
        before.decision.is_none(),
        "approval must be pending before resolve"
    );

    // Resolve the approval.
    store
        .append(&[ev(
            "evt_appr_resolve_ver",
            RuntimeEvent::ApprovalResolved(ApprovalResolved {
                project: project_a(),
                approval_id: approval_id("ver"),
                decision: ApprovalDecision::Approved,
            }),
        )])
        .await
        .unwrap();

    // Version must have incremented to 2.
    let after = ApprovalReadModel::get(store.as_ref(), &approval_id("ver"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.version, 2,
        "version must increment to 2 after ApprovalResolved"
    );
    assert_eq!(after.decision, Some(ApprovalDecision::Approved));

    // Resolved approval must no longer appear in list_pending.
    let pending = ApprovalReadModel::list_pending(store.as_ref(), &project_a(), 10, 0)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "resolved approval must not appear in pending list"
    );
}

/// Rejected approval also increments version and is removed from pending.
#[tokio::test]
async fn rejected_approval_increments_version_and_leaves_pending() {
    let store = Arc::new(InMemoryStore::new());

    seed_run(&store, &project_a(), &run_id("rej")).await;
    request_approval(&store, &project_a(), &approval_id("rej"), &run_id("rej")).await;

    store
        .append(&[ev(
            "evt_appr_reject",
            RuntimeEvent::ApprovalResolved(ApprovalResolved {
                project: project_a(),
                approval_id: approval_id("rej"),
                decision: ApprovalDecision::Rejected,
            }),
        )])
        .await
        .unwrap();

    let rec = ApprovalReadModel::get(store.as_ref(), &approval_id("rej"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        rec.version, 2,
        "rejected approval must also increment version to 2"
    );
    assert_eq!(rec.decision, Some(ApprovalDecision::Rejected));

    let pending = ApprovalReadModel::list_pending(store.as_ref(), &project_a(), 10, 0)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "rejected approval must not appear in pending list"
    );
}
