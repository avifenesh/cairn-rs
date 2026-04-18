#![cfg(feature = "in-memory-runtime")]

//! RFC 010 — Approval blocking end-to-end integration test.
//!
//! Verifies the full approval lifecycle:
//!   1. Create a session and run.
//!   2. Request an approval gating the run.
//!   3. Attempt to resume the run — must fail with `PolicyDenied` while the
//!      approval is pending.
//!   4. Approve the approval.
//!   5. Resume the run — must now succeed.

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::{
    ApprovalService, ApprovalServiceImpl, RunService, RunServiceImpl, RuntimeError, SessionService,
    SessionServiceImpl,
};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("t_approval", "ws_approval", "proj_approval")
}

#[tokio::test]
async fn approval_blocks_resume_then_allows_after_decision() {
    let store = Arc::new(InMemoryStore::new());

    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = ApprovalRunSvc {
        run: RunServiceImpl::new(store.clone()),
        approval: ApprovalServiceImpl::new(store.clone()),
    };

    let project = project();
    let session_id = SessionId::new("sess_approval");
    let run_id = RunId::new("run_approval");
    let approval_id = ApprovalId::new("appr_1");

    // ── Step 1: create session and run ───────────────────────────────────────
    session_svc
        .create(&project, session_id.clone())
        .await
        .unwrap();

    let run = run_svc
        .run
        .start(&project, &session_id, run_id.clone(), None)
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Pending);

    // ── Step 2: request an approval gating the run ───────────────────────────
    let approval = run_svc
        .approval
        .request(
            &project,
            approval_id.clone(),
            Some(run_id.clone()),
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    assert_eq!(approval.run_id, Some(run_id.clone()));
    assert!(approval.decision.is_none(), "approval must start undecided");

    // ── Step 3: resume must fail while approval is pending ───────────────────
    let resume_err = run_svc
        .run
        .resume(
            &run_id,
            ResumeTrigger::OperatorResume,
            RunResumeTarget::Running,
        )
        .await
        .expect_err("resume must fail while approval is pending");

    assert!(
        matches!(resume_err, RuntimeError::PolicyDenied { .. }),
        "expected PolicyDenied, got: {resume_err:?}"
    );

    // ── Step 4: approve the approval ─────────────────────────────────────────
    let resolved = run_svc
        .approval
        .resolve(&approval_id, ApprovalDecision::Approved)
        .await
        .unwrap();

    assert_eq!(
        resolved.decision,
        Some(ApprovalDecision::Approved),
        "approval must carry the Approved decision after resolution"
    );

    // ── Step 5: run must be Running after approval resolution ────────────────
    //
    // Approval resolution cascades the state transition automatically
    // (WaitingApproval → Running), so no explicit resume is required.
    let run_after = run_svc
        .run
        .get(&run_id)
        .await
        .unwrap()
        .expect("run must exist");

    assert_eq!(
        run_after.state,
        RunState::Running,
        "run must be Running after approval resolution"
    );
}

#[tokio::test]
async fn rejection_also_unblocks_run_but_leaves_approval_rejected() {
    let store = Arc::new(InMemoryStore::new());

    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = ApprovalRunSvc {
        run: RunServiceImpl::new(store.clone()),
        approval: ApprovalServiceImpl::new(store.clone()),
    };

    let project = project();
    let session_id = SessionId::new("sess_reject");
    let run_id = RunId::new("run_reject");
    let approval_id = ApprovalId::new("appr_reject");

    session_svc
        .create(&project, session_id.clone())
        .await
        .unwrap();
    run_svc
        .run
        .start(&project, &session_id, run_id.clone(), None)
        .await
        .unwrap();

    // Gate the run with an approval.
    run_svc
        .approval
        .request(
            &project,
            approval_id.clone(),
            Some(run_id.clone()),
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    // Reject — the approval is resolved, so the pending gate is lifted.
    let rejected = run_svc
        .approval
        .resolve(&approval_id, ApprovalDecision::Rejected)
        .await
        .unwrap();

    assert_eq!(
        rejected.decision,
        Some(ApprovalDecision::Rejected),
        "approval must carry Rejected decision"
    );

    // After rejection the approval is no longer pending, so resume is not
    // blocked by it.  The run may still fail for other reasons (state machine),
    // but the PolicyDenied gate must be gone.
    let resume_result = run_svc
        .run
        .resume(
            &run_id,
            ResumeTrigger::OperatorResume,
            RunResumeTarget::Running,
        )
        .await;

    // Must NOT be PolicyDenied (approval no longer pending).
    assert!(
        !matches!(resume_result, Err(RuntimeError::PolicyDenied { .. })),
        "rejected approval must not block resume; got: {resume_result:?}"
    );
}

#[tokio::test]
async fn resolving_already_resolved_approval_returns_error() {
    let store = Arc::new(InMemoryStore::new());
    let approval_svc = ApprovalServiceImpl::new(store.clone());
    let project = project();
    let approval_id = ApprovalId::new("appr_double");

    // Create and resolve the approval.
    approval_svc
        .request(
            &project,
            approval_id.clone(),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    approval_svc
        .resolve(&approval_id, ApprovalDecision::Approved)
        .await
        .unwrap();

    // Second resolution must fail.
    let err = approval_svc
        .resolve(&approval_id, ApprovalDecision::Rejected)
        .await
        .expect_err("double-resolution must fail");

    assert!(
        matches!(err, RuntimeError::InvalidTransition { .. }),
        "expected InvalidTransition on double-resolve, got: {err:?}"
    );
}

// ── Helper ────────────────────────────────────────────────────────────────────

/// Bundles run + approval services for cleaner test code.
struct ApprovalRunSvc {
    run: RunServiceImpl<InMemoryStore>,
    approval: ApprovalServiceImpl<InMemoryStore>,
}
