//! Unit + integration tests for [`ToolCallApprovalService`].
//!
//! These tests pair the in-memory store with a stub
//! [`ToolCallApprovalReader`] so the service can be exercised without
//! PR-2's store projection. When PR-2 lands, the `StubReader` below is
//! replaced at the integration layer by the real projection, and the
//! cache-miss tests gain a store-backed assertion.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cairn_domain::{
    ApprovalMatchPolicy, ApprovalScope, OperatorId, ProjectKey, RunId, SessionId, ToolCallId,
};
use cairn_runtime::error::RuntimeError;
use cairn_runtime::services::ToolCallApprovalServiceImpl;
use cairn_runtime::tool_call_approvals::{
    ApprovalDecision, ApprovedProposal, OperatorDecision, ToolCallApprovalReader,
    ToolCallApprovalService, ToolCallProposal,
};
use cairn_store::InMemoryStore;
use serde_json::{json, Value};

/// In-test reader that never finds anything (simulates a missing
/// projection). Covers the cache-miss path when PR-2's real
/// implementation is unavailable.
#[derive(Default)]
struct EmptyReader;

#[async_trait]
impl ToolCallApprovalReader for EmptyReader {
    async fn get_tool_call_approval(
        &self,
        _call_id: &ToolCallId,
    ) -> Result<Option<ApprovedProposal>, RuntimeError> {
        Ok(None)
    }
}

/// In-test reader that holds hand-seeded approvals. Lets cache-miss
/// retrieval tests assert that the service falls through to the store.
#[derive(Default)]
struct SeededReader {
    entries: Mutex<Vec<(ToolCallId, ApprovedProposal)>>,
}

impl SeededReader {
    fn insert(&self, id: ToolCallId, p: ApprovedProposal) {
        self.entries.lock().unwrap().push((id, p));
    }
}

#[async_trait]
impl ToolCallApprovalReader for SeededReader {
    async fn get_tool_call_approval(
        &self,
        call_id: &ToolCallId,
    ) -> Result<Option<ApprovedProposal>, RuntimeError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == call_id)
            .map(|(_, p)| p.clone()))
    }
}

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn proposal(call: &str, tool: &str, args: Value) -> ToolCallProposal {
    ToolCallProposal {
        call_id: ToolCallId::new(call),
        session_id: SessionId::new("sess-1"),
        run_id: RunId::new("run-1"),
        project: project(),
        tool_name: tool.to_owned(),
        tool_args: args,
        display_summary: Some(format!("calling {tool}")),
        match_policy: ApprovalMatchPolicy::Exact,
    }
}

fn setup() -> ToolCallApprovalServiceImpl<InMemoryStore, EmptyReader> {
    let store = Arc::new(InMemoryStore::new());
    ToolCallApprovalServiceImpl::new(store, Arc::new(EmptyReader))
}

#[tokio::test(flavor = "current_thread")]
async fn submit_proposal_no_match_returns_pending() {
    let svc = setup();
    let decision = svc
        .submit_proposal(proposal("tc1", "read", json!({ "path": "/a/b" })))
        .await
        .expect("submit");
    assert_eq!(decision, ApprovalDecision::PendingOperator);
}

#[tokio::test(flavor = "current_thread")]
async fn submit_proposal_exact_session_match_auto_approves() {
    let svc = setup();

    // First proposal — pending.
    let p1 = proposal("tc1", "read", json!({ "path": "/a/b" }));
    assert_eq!(
        svc.submit_proposal(p1.clone()).await.unwrap(),
        ApprovalDecision::PendingOperator
    );

    // Approve with Session/Exact — widens allow registry.
    svc.approve(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        ApprovalScope::Session {
            match_policy: ApprovalMatchPolicy::Exact,
        },
        None,
    )
    .await
    .unwrap();

    // Second, byte-identical proposal should auto-approve.
    let p2 = proposal("tc2", "read", json!({ "path": "/a/b" }));
    assert_eq!(
        svc.submit_proposal(p2).await.unwrap(),
        ApprovalDecision::AutoApproved
    );

    // Third, different args — still pending.
    let p3 = proposal("tc3", "read", json!({ "path": "/a/c" }));
    assert_eq!(
        svc.submit_proposal(p3).await.unwrap(),
        ApprovalDecision::PendingOperator
    );
}

#[tokio::test(flavor = "current_thread")]
async fn submit_proposal_project_scoped_path_matches_sibling_file() {
    let svc = setup();

    let p1 = proposal(
        "tc1",
        "read",
        json!({ "path": "/workspaces/proj/README.md" }),
    );
    svc.submit_proposal(p1).await.unwrap();

    svc.approve(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        ApprovalScope::Session {
            match_policy: ApprovalMatchPolicy::ProjectScopedPath {
                project_root: "/workspaces/proj".into(),
            },
        },
        None,
    )
    .await
    .unwrap();

    let sibling = proposal("tc2", "read", json!({ "path": "/workspaces/proj/src/lib.rs" }));
    assert_eq!(
        svc.submit_proposal(sibling).await.unwrap(),
        ApprovalDecision::AutoApproved
    );
}

#[tokio::test(flavor = "current_thread")]
async fn submit_proposal_project_scoped_path_rejects_outside_file() {
    let svc = setup();

    let p1 = proposal("tc1", "read", json!({ "path": "/workspaces/proj/a" }));
    svc.submit_proposal(p1).await.unwrap();
    svc.approve(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        ApprovalScope::Session {
            match_policy: ApprovalMatchPolicy::ProjectScopedPath {
                project_root: "/workspaces/proj".into(),
            },
        },
        None,
    )
    .await
    .unwrap();

    // Sibling workspace — must not match (path-component boundary).
    let outside = proposal("tc2", "read", json!({ "path": "/workspaces/proj2/a" }));
    assert_eq!(
        svc.submit_proposal(outside).await.unwrap(),
        ApprovalDecision::PendingOperator
    );
}

#[tokio::test(flavor = "current_thread")]
async fn approve_fires_oneshot_and_resolves_await() {
    let svc = Arc::new(setup());
    svc.submit_proposal(proposal("tc1", "read", json!({ "path": "/a" })))
        .await
        .unwrap();

    let svc_bg = svc.clone();
    let approve = tokio::spawn(async move {
        // Tiny delay so the awaiter parks first.
        tokio::time::sleep(Duration::from_millis(10)).await;
        svc_bg
            .approve(
                ToolCallId::new("tc1"),
                OperatorId::new("op-1"),
                ApprovalScope::Once,
                None,
            )
            .await
            .unwrap();
    });

    let decision = svc
        .await_decision(&ToolCallId::new("tc1"), Duration::from_secs(2))
        .await
        .unwrap();

    approve.await.unwrap();
    match decision {
        OperatorDecision::Approved { approved_args } => {
            assert_eq!(approved_args, json!({ "path": "/a" }));
        }
        other => panic!("expected Approved, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn reject_fires_oneshot_with_reason() {
    let svc = Arc::new(setup());
    svc.submit_proposal(proposal("tc1", "bash", json!({ "cmd": "rm -rf /" })))
        .await
        .unwrap();

    let svc_bg = svc.clone();
    let reject = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        svc_bg
            .reject(
                ToolCallId::new("tc1"),
                OperatorId::new("op-1"),
                Some("too spicy".into()),
            )
            .await
            .unwrap();
    });

    let decision = svc
        .await_decision(&ToolCallId::new("tc1"), Duration::from_secs(2))
        .await
        .unwrap();

    reject.await.unwrap();
    match decision {
        OperatorDecision::Rejected { reason } => {
            assert_eq!(reason, Some("too spicy".into()));
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn amend_does_not_fire_oneshot() {
    let svc = Arc::new(setup());
    svc.submit_proposal(proposal("tc1", "write", json!({ "path": "/a", "body": "v1" })))
        .await
        .unwrap();

    let svc_bg = svc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        svc_bg
            .amend(
                ToolCallId::new("tc1"),
                OperatorId::new("op-1"),
                json!({ "path": "/a", "body": "v2" }),
            )
            .await
            .unwrap();
    });

    // Timeout short enough to assert amend did NOT resolve the awaiter.
    let decision = svc
        .await_decision(&ToolCallId::new("tc1"), Duration::from_millis(150))
        .await
        .unwrap();
    assert!(
        matches!(decision, OperatorDecision::Timeout),
        "expected Timeout after amend-only, got {decision:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn await_decision_timeout_auto_rejects() {
    let svc = setup();
    svc.submit_proposal(proposal("tc1", "read", json!({ "path": "/a" })))
        .await
        .unwrap();

    let decision = svc
        .await_decision(&ToolCallId::new("tc1"), Duration::from_millis(50))
        .await
        .unwrap();
    assert!(matches!(decision, OperatorDecision::Timeout));

    // After timeout, attempting to approve should fail (already rejected).
    let err = svc
        .approve(
            ToolCallId::new("tc1"),
            OperatorId::new("op-1"),
            ApprovalScope::Once,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, RuntimeError::InvalidTransition { .. }),
        "expected InvalidTransition after timeout-reject, got {err:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn retrieve_approved_proposal_uses_amended_args() {
    let svc = setup();
    svc.submit_proposal(proposal("tc1", "write", json!({ "body": "v1" })))
        .await
        .unwrap();

    svc.amend(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        json!({ "body": "v2" }),
    )
    .await
    .unwrap();

    svc.approve(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        ApprovalScope::Once,
        None, // no override — effective args come from the amendment.
    )
    .await
    .unwrap();

    let approved = svc
        .retrieve_approved_proposal(&ToolCallId::new("tc1"))
        .await
        .unwrap();
    assert_eq!(approved.tool_name, "write");
    assert_eq!(approved.tool_args, json!({ "body": "v2" }));
}

#[tokio::test(flavor = "current_thread")]
async fn retrieve_approved_proposal_prefers_explicit_override() {
    let svc = setup();
    svc.submit_proposal(proposal("tc1", "write", json!({ "body": "v1" })))
        .await
        .unwrap();

    svc.amend(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        json!({ "body": "v2" }),
    )
    .await
    .unwrap();

    // Override at approval time with v3 — wins over the amended v2.
    svc.approve(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        ApprovalScope::Once,
        Some(json!({ "body": "v3" })),
    )
    .await
    .unwrap();

    let approved = svc
        .retrieve_approved_proposal(&ToolCallId::new("tc1"))
        .await
        .unwrap();
    assert_eq!(approved.tool_args, json!({ "body": "v3" }));
}

#[tokio::test(flavor = "current_thread")]
async fn retrieve_approved_proposal_from_store_on_cache_miss() {
    // PR-2 dependency: real store-projection reader lands there. Until
    // then we stand up the service with a hand-seeded `SeededReader`
    // and drive the service directly (bypassing the cache) to assert
    // that the fallback path reads from the reader.
    let store = Arc::new(InMemoryStore::new());
    let reader = Arc::new(SeededReader::default());
    reader.insert(
        ToolCallId::new("tc_gone"),
        ApprovedProposal {
            call_id: ToolCallId::new("tc_gone"),
            tool_name: "read".into(),
            tool_args: json!({ "path": "/recovered" }),
        },
    );
    let svc = ToolCallApprovalServiceImpl::new(store, reader);

    let approved = svc
        .retrieve_approved_proposal(&ToolCallId::new("tc_gone"))
        .await
        .unwrap();
    assert_eq!(approved.tool_args, json!({ "path": "/recovered" }));
}

#[tokio::test(flavor = "current_thread")]
async fn retrieve_approved_proposal_not_found_when_missing_everywhere() {
    let svc = setup();
    let err = svc
        .retrieve_approved_proposal(&ToolCallId::new("nope"))
        .await
        .unwrap_err();
    assert!(matches!(err, RuntimeError::NotFound { entity, .. } if entity == "tool_call_approval"));
}

#[tokio::test(flavor = "current_thread")]
async fn approve_after_approve_is_invalid_transition() {
    let svc = setup();
    svc.submit_proposal(proposal("tc1", "read", json!({ "path": "/a" })))
        .await
        .unwrap();

    svc.approve(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        ApprovalScope::Once,
        None,
    )
    .await
    .unwrap();

    let err = svc
        .approve(
            ToolCallId::new("tc1"),
            OperatorId::new("op-2"),
            ApprovalScope::Once,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RuntimeError::InvalidTransition { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_await_decision_is_rejected() {
    let svc = Arc::new(setup());
    svc.submit_proposal(proposal("tc1", "read", json!({ "path": "/a" })))
        .await
        .unwrap();

    let svc2 = svc.clone();
    let first = tokio::spawn(async move {
        svc2.await_decision(&ToolCallId::new("tc1"), Duration::from_millis(200))
            .await
    });
    // Let the first awaiter register its pending sender.
    tokio::time::sleep(Duration::from_millis(10)).await;

    let err = svc
        .await_decision(&ToolCallId::new("tc1"), Duration::from_millis(50))
        .await
        .unwrap_err();
    assert!(
        matches!(err, RuntimeError::Conflict { .. }),
        "expected Conflict for concurrent await, got {err:?}"
    );
    // Clean up the first awaiter (it'll time-out).
    let _ = first.await.unwrap();
}
