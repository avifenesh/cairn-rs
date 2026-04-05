//! Human-in-the-loop approval workflow integration tests (RFC 005).
//!
//! Validates the full approval pipeline using `InMemoryStore` + `EventLog::append`.
//! Approvals are the key differentiator: the system gates tool execution on an
//! explicit human decision before proceeding.
//!
//! Pipeline under test:
//!   RunCreated + ToolInvocationStarted
//!     → ApprovalRequested  (decision = None, pending)
//!       → ApprovalResolved(Approved)  → decision updated, version bumped
//!   ApprovalRequested (second)
//!     → ApprovalResolved(Rejected)   → rejection path
//!   ApprovalPolicyCreated → queryable by policy ID and tenant

use cairn_domain::{
    ApprovalId, ApprovalPolicyCreated, ApprovalRequested, ApprovalResolved, EventEnvelope,
    EventId, EventSource, ExecutionClass, ProjectId, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, TenantId, ToolInvocationId, ToolInvocationStarted, WorkspaceId,
};
use cairn_domain::policy::{ApprovalDecision, ApprovalRequirement};
use cairn_domain::tool_invocation::ToolInvocationTarget;
use cairn_domain::tenancy::WorkspaceRole;
use cairn_store::{
    projections::{ApprovalPolicyReadModel, ApprovalReadModel, ToolInvocationReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_approval"),
        workspace_id: WorkspaceId::new("w_approval"),
        project_id: ProjectId::new("p_approval"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── 1. Run + ToolInvocation setup ────────────────────────────────────────────

#[tokio::test]
async fn tool_invocation_is_recorded_before_approval() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = RunId::new("run_tool_1");
    let invocation_id = ToolInvocationId::new("inv_1");

    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new("sess_1"),
            })),
            evt("e2", RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_1"),
                run_id: run_id.clone(),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            evt("e3", RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project(),
                invocation_id: invocation_id.clone(),
                session_id: Some(SessionId::new("sess_1")),
                run_id: Some(run_id.clone()),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "write_file".to_owned(),
                },
                execution_class: ExecutionClass::SupervisedProcess,
                prompt_release_id: None,
                requested_at_ms: ts,
                started_at_ms: ts,
            })),
        ])
        .await
        .unwrap();

    let inv = ToolInvocationReadModel::get(&store, &invocation_id)
        .await
        .unwrap()
        .expect("tool invocation must exist after ToolInvocationStarted");

    assert_eq!(inv.invocation_id, invocation_id);
    assert_eq!(inv.run_id, Some(run_id));
    assert!(matches!(inv.target, ToolInvocationTarget::Builtin { ref tool_name } if tool_name == "write_file"));
}

// ── 2. ApprovalRequested creates pending record ───────────────────────────────

#[tokio::test]
async fn approval_requested_creates_pending_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = RunId::new("run_appr_2");
    let approval_id = ApprovalId::new("appr_2");

    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new("sess_2"),
            })),
            evt("e2", RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_2"),
                run_id: run_id.clone(),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            evt("e3", RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project(),
                approval_id: approval_id.clone(),
                run_id: Some(run_id.clone()),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
        ])
        .await
        .unwrap();

    let record = ApprovalReadModel::get(&store, &approval_id)
        .await
        .unwrap()
        .expect("ApprovalRecord must exist after ApprovalRequested");

    assert_eq!(record.approval_id, approval_id);
    assert_eq!(record.run_id, Some(run_id));
    assert_eq!(record.requirement, ApprovalRequirement::Required);
    assert!(record.decision.is_none(), "pending approval has no decision yet");
    assert_eq!(record.version, 1, "initial version is 1");

    // Appears in the pending inbox.
    let pending = ApprovalReadModel::list_pending(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].approval_id, approval_id);
}

// ── 3. ApprovalResolved(Approved) updates the record ─────────────────────────

#[tokio::test]
async fn approval_resolved_approved_updates_decision() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = RunId::new("run_appr_3");
    let approval_id = ApprovalId::new("appr_3");

    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new("sess_3"),
            })),
            evt("e2", RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_3"),
                run_id: run_id.clone(),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            evt("e3", RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project(),
                approval_id: approval_id.clone(),
                run_id: Some(run_id.clone()),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
        ])
        .await
        .unwrap();

    // Verify pending before resolution.
    let before = ApprovalReadModel::get(&store, &approval_id).await.unwrap().unwrap();
    assert!(before.decision.is_none());

    // Operator approves.
    store
        .append(&[evt("e4", RuntimeEvent::ApprovalResolved(ApprovalResolved {
            project: project(),
            approval_id: approval_id.clone(),
            decision: ApprovalDecision::Approved,
        }))])
        .await
        .unwrap();

    let after = ApprovalReadModel::get(&store, &approval_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(after.decision, Some(ApprovalDecision::Approved));
    assert_eq!(after.version, 2, "version bumped after resolution");
    assert!(after.updated_at >= before.updated_at, "updated_at must advance");

    // Resolved approval no longer in pending inbox.
    let pending = ApprovalReadModel::list_pending(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert!(pending.is_empty(), "resolved approval must not appear in pending list");
}

// ── 4. Rejection path ─────────────────────────────────────────────────────────

#[tokio::test]
async fn approval_resolved_rejected_records_rejection() {
    let store = InMemoryStore::new();
    let run_id = RunId::new("run_rej_4");
    let approval_id = ApprovalId::new("appr_rej_4");

    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new("sess_4"),
            })),
            evt("e2", RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_4"),
                run_id: run_id.clone(),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            evt("e3", RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project(),
                approval_id: approval_id.clone(),
                run_id: Some(run_id.clone()),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
            evt("e4", RuntimeEvent::ApprovalResolved(ApprovalResolved {
                project: project(),
                approval_id: approval_id.clone(),
                decision: ApprovalDecision::Rejected,
            })),
        ])
        .await
        .unwrap();

    let record = ApprovalReadModel::get(&store, &approval_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.decision, Some(ApprovalDecision::Rejected));
    assert_eq!(record.version, 2);

    // Rejected approval is not pending.
    let pending = ApprovalReadModel::list_pending(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert!(pending.is_empty());
}

// ── 5. Multiple concurrent approvals — pending list shows all unresolved ──────

#[tokio::test]
async fn multiple_approvals_pending_list_shows_unresolved_only() {
    let store = InMemoryStore::new();
    let run_id = RunId::new("run_multi_5");
    let ap1 = ApprovalId::new("appr_5a");
    let ap2 = ApprovalId::new("appr_5b");
    let ap3 = ApprovalId::new("appr_5c");

    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new("sess_5"),
            })),
            evt("e2", RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_5"),
                run_id: run_id.clone(),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            // Three approvals requested.
            evt("e3", RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project(),
                approval_id: ap1.clone(),
                run_id: Some(run_id.clone()),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
            evt("e4", RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project(),
                approval_id: ap2.clone(),
                run_id: Some(run_id.clone()),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
            evt("e5", RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project(),
                approval_id: ap3.clone(),
                run_id: Some(run_id.clone()),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
        ])
        .await
        .unwrap();

    let pending_before = ApprovalReadModel::list_pending(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(pending_before.len(), 3, "all three approvals are pending");

    // Resolve the first two.
    store
        .append(&[
            evt("e6", RuntimeEvent::ApprovalResolved(ApprovalResolved {
                project: project(),
                approval_id: ap1.clone(),
                decision: ApprovalDecision::Approved,
            })),
            evt("e7", RuntimeEvent::ApprovalResolved(ApprovalResolved {
                project: project(),
                approval_id: ap2.clone(),
                decision: ApprovalDecision::Rejected,
            })),
        ])
        .await
        .unwrap();

    let pending_after = ApprovalReadModel::list_pending(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(pending_after.len(), 1, "only the third approval remains pending");
    assert_eq!(pending_after[0].approval_id, ap3);

    // Decisions are correct.
    let r1 = ApprovalReadModel::get(&store, &ap1).await.unwrap().unwrap();
    assert_eq!(r1.decision, Some(ApprovalDecision::Approved));
    let r2 = ApprovalReadModel::get(&store, &ap2).await.unwrap().unwrap();
    assert_eq!(r2.decision, Some(ApprovalDecision::Rejected));
}

// ── 6. ApprovalPolicy is queryable ────────────────────────────────────────────

#[tokio::test]
async fn approval_policy_created_is_queryable() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::ApprovalPolicyCreated(ApprovalPolicyCreated {
                project: project(),
                policy_id: "policy_strict_001".to_owned(),
                tenant_id: TenantId::new("t_approval"),
                name: "Strict Approval Policy".to_owned(),
                required_approvers: 2,
                allowed_approver_roles: vec![WorkspaceRole::Admin, WorkspaceRole::Owner],
                auto_approve_after_ms: None,
                auto_reject_after_ms: Some(86_400_000), // 24 h
                created_at_ms: ts,
            }),
        )])
        .await
        .unwrap();

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_strict_001")
        .await
        .unwrap()
        .expect("policy must exist after ApprovalPolicyCreated");

    assert_eq!(policy.policy_id, "policy_strict_001");
    assert_eq!(policy.name, "Strict Approval Policy");
    assert_eq!(policy.required_approvers, 2);
    assert!(policy.auto_approve_after_ms.is_none());
    assert_eq!(policy.auto_reject_after_ms, Some(86_400_000));

    // List by tenant returns it.
    let by_tenant = ApprovalPolicyReadModel::list_by_tenant(
        &store,
        &TenantId::new("t_approval"),
        10,
        0,
    )
    .await
    .unwrap();
    assert_eq!(by_tenant.len(), 1);
    assert_eq!(by_tenant[0].policy_id, "policy_strict_001");

    // Another tenant returns nothing.
    let other = ApprovalPolicyReadModel::list_by_tenant(
        &store,
        &TenantId::new("other_tenant"),
        10,
        0,
    )
    .await
    .unwrap();
    assert!(other.is_empty(), "policy is tenant-scoped");
}

// ── 7. Full workflow: tool invocation → approval → resolved ───────────────────

#[tokio::test]
async fn full_approval_workflow_tool_to_resolution() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = RunId::new("run_full_7");
    let invocation_id = ToolInvocationId::new("inv_full_7");
    let approval_id = ApprovalId::new("appr_full_7");

    // Step 1: session + run.
    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new("sess_7"),
            })),
            evt("e2", RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_7"),
                run_id: run_id.clone(),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
        ])
        .await
        .unwrap();

    // Step 2: tool invocation starts, requiring approval.
    store
        .append(&[evt("e3", RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
            project: project(),
            invocation_id: invocation_id.clone(),
            session_id: Some(SessionId::new("sess_7")),
            run_id: Some(run_id.clone()),
            task_id: None,
            target: ToolInvocationTarget::Builtin {
                tool_name: "deploy_code".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            prompt_release_id: None,
            requested_at_ms: ts,
            started_at_ms: ts,
        }))])
        .await
        .unwrap();

    // Step 3: approval gate — run is paused waiting for human.
    store
        .append(&[evt("e4", RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project(),
            approval_id: approval_id.clone(),
            run_id: Some(run_id.clone()),
            task_id: None,
            requirement: ApprovalRequirement::Required,
        }))])
        .await
        .unwrap();

    // Verify gate is active.
    let pending = ApprovalReadModel::list_pending(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "run is gated on one pending approval");
    assert_eq!(pending[0].run_id, Some(run_id.clone()));

    // The tool invocation is correctly linked to the run.
    let invocations = ToolInvocationReadModel::list_by_run(&store, &run_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].invocation_id, invocation_id);

    // Step 4: operator approves → gate lifts.
    store
        .append(&[evt("e5", RuntimeEvent::ApprovalResolved(ApprovalResolved {
            project: project(),
            approval_id: approval_id.clone(),
            decision: ApprovalDecision::Approved,
        }))])
        .await
        .unwrap();

    // Gate is lifted.
    let pending_after = ApprovalReadModel::list_pending(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert!(pending_after.is_empty(), "no pending approvals after resolution");

    // Final record shows approved.
    let final_record = ApprovalReadModel::get(&store, &approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_record.decision, Some(ApprovalDecision::Approved));
    assert_eq!(final_record.version, 2);
}
