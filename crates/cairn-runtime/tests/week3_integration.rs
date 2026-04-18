#![cfg(feature = "in-memory-runtime")]

//! Week 3 integration tests: approvals, checkpoints, mailbox.
//!
//! Recovery tests that lived here were removed in the Fabric finalization
//! round — FF's LeaseExpiryScanner owns expired-lease recovery
//! unconditionally.

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::{
    ApprovalService, ApprovalServiceImpl, CheckpointService, CheckpointServiceImpl, MailboxService,
    MailboxServiceImpl, RunService, RunServiceImpl, SessionService, SessionServiceImpl,
};
use cairn_store::InMemoryStore;

fn test_project() -> ProjectKey {
    ProjectKey::new("tenant_acme", "ws_main", "project_alpha")
}

// -- Approval tests --

#[tokio::test]
async fn approval_request_and_resolve() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ApprovalServiceImpl::new(store);
    let project = test_project();

    let approval = svc
        .request(
            &project,
            ApprovalId::new("appr_1"),
            Some(RunId::new("run_1")),
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    assert!(approval.decision.is_none());
    assert_eq!(approval.requirement, ApprovalRequirement::Required);

    let resolved = svc
        .resolve(&ApprovalId::new("appr_1"), ApprovalDecision::Approved)
        .await
        .unwrap();

    assert_eq!(resolved.decision, Some(ApprovalDecision::Approved));
}

#[tokio::test]
async fn cannot_resolve_already_resolved_approval() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ApprovalServiceImpl::new(store);
    let project = test_project();

    svc.request(
        &project,
        ApprovalId::new("appr_1"),
        None,
        None,
        ApprovalRequirement::Required,
    )
    .await
    .unwrap();

    svc.resolve(&ApprovalId::new("appr_1"), ApprovalDecision::Approved)
        .await
        .unwrap();

    let result = svc
        .resolve(&ApprovalId::new("appr_1"), ApprovalDecision::Rejected)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_pending_approvals() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ApprovalServiceImpl::new(store);
    let project = test_project();

    svc.request(
        &project,
        ApprovalId::new("appr_1"),
        None,
        None,
        ApprovalRequirement::Required,
    )
    .await
    .unwrap();
    svc.request(
        &project,
        ApprovalId::new("appr_2"),
        None,
        None,
        ApprovalRequirement::Required,
    )
    .await
    .unwrap();
    svc.resolve(&ApprovalId::new("appr_1"), ApprovalDecision::Approved)
        .await
        .unwrap();

    let pending = svc.list_pending(&project, 10, 0).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].approval_id, ApprovalId::new("appr_2"));
}

// -- Checkpoint tests --

#[tokio::test]
async fn checkpoint_save_and_supersede() {
    let store = Arc::new(InMemoryStore::new());
    let svc = CheckpointServiceImpl::new(store);
    let project = test_project();
    let run_id = RunId::new("run_1");

    let cp1 = svc
        .save(&project, &run_id, CheckpointId::new("cp_1"))
        .await
        .unwrap();
    assert_eq!(cp1.disposition, CheckpointDisposition::Latest);

    let cp2 = svc
        .save(&project, &run_id, CheckpointId::new("cp_2"))
        .await
        .unwrap();
    assert_eq!(cp2.disposition, CheckpointDisposition::Latest);

    // cp1 should now be superseded
    let cp1_after = svc.get(&CheckpointId::new("cp_1")).await.unwrap().unwrap();
    assert_eq!(cp1_after.disposition, CheckpointDisposition::Superseded);

    // latest_for_run should return cp2
    let latest = svc.latest_for_run(&run_id).await.unwrap().unwrap();
    assert_eq!(latest.checkpoint_id, CheckpointId::new("cp_2"));
}

// -- Mailbox tests --

#[tokio::test]
async fn mailbox_append_and_list() {
    let store = Arc::new(InMemoryStore::new());
    let svc = MailboxServiceImpl::new(store);
    let project = test_project();
    let run_id = RunId::new("run_1");

    svc.append(
        &project,
        MailboxMessageId::new("msg_1"),
        Some(run_id.clone()),
        None,
        "".to_owned(),
        None,
        0,
    )
    .await
    .unwrap();
    svc.append(
        &project,
        MailboxMessageId::new("msg_2"),
        Some(run_id.clone()),
        None,
        "".to_owned(),
        None,
        0,
    )
    .await
    .unwrap();

    let messages = svc.list_by_run(&run_id, 10, 0).await.unwrap();
    assert_eq!(messages.len(), 2);
}

// Recovery tests deleted in the Fabric finalization round — FF's
// LeaseExpiryScanner owns expired-lease recovery unconditionally
// (ff-engine/src/scanner/lease_expiry.rs). The cairn-side
// `RecoveryServiceImpl::recover_expired_leases` no longer exists.

// -- End-to-end with approvals --

#[tokio::test]
async fn run_with_approval_gate() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let approval_svc = ApprovalServiceImpl::new(store);
    let project = test_project();

    session_svc
        .create(&project, SessionId::new("sess_1"))
        .await
        .unwrap();
    let run = run_svc
        .start(
            &project,
            &SessionId::new("sess_1"),
            RunId::new("run_1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Pending);

    // Request approval linked to the run
    let approval = approval_svc
        .request(
            &project,
            ApprovalId::new("appr_1"),
            Some(RunId::new("run_1")),
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    assert!(approval.decision.is_none());

    // Approve
    let resolved = approval_svc
        .resolve(&ApprovalId::new("appr_1"), ApprovalDecision::Approved)
        .await
        .unwrap();
    assert_eq!(resolved.decision, Some(ApprovalDecision::Approved));

    // Approval resolution auto-transitions the run from WaitingApproval → Running.
    let run = run_svc
        .get(&RunId::new("run_1"))
        .await
        .unwrap()
        .expect("run must exist");
    assert_eq!(run.state, RunState::Running);

    let run = run_svc.complete(&RunId::new("run_1")).await.unwrap();
    assert_eq!(run.state, RunState::Completed);
}
