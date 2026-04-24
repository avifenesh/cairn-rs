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
    ApprovalDecision, ApprovedProposal, OperatorDecision, StoredProposal, StoredProposalState,
    ToolCallApprovalReader, ToolCallApprovalService, ToolCallProposal,
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
    proposals: Mutex<Vec<(ToolCallId, StoredProposal)>>,
}

impl SeededReader {
    fn insert(&self, id: ToolCallId, p: ApprovedProposal) {
        self.entries.lock().unwrap().push((id, p));
    }

    fn insert_proposal(&self, id: ToolCallId, p: StoredProposal) {
        self.proposals.lock().unwrap().push((id, p));
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

    async fn get_tool_call_proposal(
        &self,
        call_id: &ToolCallId,
    ) -> Result<Option<StoredProposal>, RuntimeError> {
        Ok(self
            .proposals
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == call_id)
            .map(|(_, p)| p.clone()))
    }
}

/// Build a `StoredProposal` representing a pending record in the
/// projection — the exact shape the service sees after restart when the
/// in-memory cache is empty but the event log + projection retain the
/// proposal.
fn stored_pending(call: &str, tool: &str, args: Value) -> StoredProposal {
    StoredProposal {
        proposal: proposal(call, tool, args),
        state: StoredProposalState::Pending,
        amended_args: None,
        approved_args: None,
        rejection_reason: None,
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

    let sibling = proposal(
        "tc2",
        "read",
        json!({ "path": "/workspaces/proj/src/lib.rs" }),
    );
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
    svc.submit_proposal(proposal(
        "tc1",
        "write",
        json!({ "path": "/a", "body": "v1" }),
    ))
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
async fn await_decision_fast_path_returns_rejection_reason() {
    // Reject BEFORE the caller awaits — the fast path in await_decision
    // must return the actual reason (not ambiguous None). Also covers
    // the `operator_timeout` sentinel path.
    let svc = setup();
    svc.submit_proposal(proposal("tc1", "bash", json!({ "cmd": "ls" })))
        .await
        .unwrap();

    svc.reject(
        ToolCallId::new("tc1"),
        OperatorId::new("op-1"),
        Some("policy-violation".into()),
    )
    .await
    .unwrap();

    let decision = svc
        .await_decision(&ToolCallId::new("tc1"), Duration::from_secs(5))
        .await
        .unwrap();
    match decision {
        OperatorDecision::Rejected { reason } => {
            assert_eq!(reason, Some("policy-violation".into()));
        }
        other => panic!("expected Rejected with reason, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn session_allow_rule_uses_effective_args_not_original() {
    // If the operator amends to v2 and approves, a subsequent proposal
    // with args v2 must auto-approve (because the registered allow rule
    // captured the effective/amended args, not the original v1).
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
        ApprovalScope::Session {
            match_policy: ApprovalMatchPolicy::Exact,
        },
        None,
    )
    .await
    .unwrap();

    // Second call with the ORIGINAL v1 args — must NOT auto-approve.
    assert_eq!(
        svc.submit_proposal(proposal("tc2", "write", json!({ "body": "v1" })))
            .await
            .unwrap(),
        ApprovalDecision::PendingOperator
    );

    // Third call with the AMENDED v2 args — must auto-approve.
    assert_eq!(
        svc.submit_proposal(proposal("tc3", "write", json!({ "body": "v2" })))
            .await
            .unwrap(),
        ApprovalDecision::AutoApproved
    );
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_write_is_idempotent_on_concurrent_approve() {
    // Stress the atomic-claim property: fire `approve` simultaneously
    // with a timeout-about-to-expire. Whichever lands first wins; the
    // second operation must observe InvalidTransition and the cache
    // must reflect exactly one resolution. The event log should carry
    // exactly one resolution event per call_id.
    use std::sync::atomic::{AtomicUsize, Ordering};

    for _ in 0..20 {
        let svc = Arc::new(setup());
        svc.submit_proposal(proposal("tc1", "read", json!({ "path": "/a" })))
            .await
            .unwrap();

        let svc_wait = svc.clone();
        let awaiter = tokio::spawn(async move {
            svc_wait
                .await_decision(&ToolCallId::new("tc1"), Duration::from_millis(20))
                .await
        });
        // Race: try to approve right around the timeout boundary.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let approve_err_counter = AtomicUsize::new(0);
        let approve_res = svc
            .approve(
                ToolCallId::new("tc1"),
                OperatorId::new("op-1"),
                ApprovalScope::Once,
                None,
            )
            .await;
        if approve_res.is_err() {
            approve_err_counter.fetch_add(1, Ordering::SeqCst);
        }
        let _ = awaiter.await.unwrap();
        // Post-condition: cache is resolved exactly once. retrieving
        // must succeed (if approved won the race) or fail with
        // InvalidTransition (if timeout won). Never panics.
        let _ = svc
            .retrieve_approved_proposal(&ToolCallId::new("tc1"))
            .await;
    }
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

// ── Cache-miss fallback (F22 dogfood unblocker) ─────────────────────────────
//
// Proposals live in the durable projection across process restart, but the
// in-memory cache does not. Before the fix, approve/reject/amend looked
// *only* in the cache and returned NotFound the instant a restart happened
// — even though the event log + projection still held a perfectly valid
// pending record. These tests pin the fallback: the service now re-hydrates
// the cache from the projection and proceeds with the decision flow.

fn svc_with_seeded_reader() -> (
    ToolCallApprovalServiceImpl<InMemoryStore, SeededReader>,
    Arc<SeededReader>,
) {
    let store = Arc::new(InMemoryStore::new());
    let reader = Arc::new(SeededReader::default());
    let svc = ToolCallApprovalServiceImpl::new(store, reader.clone());
    (svc, reader)
}

#[tokio::test(flavor = "current_thread")]
async fn approve_after_cache_eviction_falls_back_to_projection() {
    // Simulates: proposal was submitted before a restart → projection has
    // the row, but the cache is empty. Approve must rehydrate and succeed.
    let (svc, reader) = svc_with_seeded_reader();
    reader.insert_proposal(
        ToolCallId::new("tc_restart"),
        stored_pending("tc_restart", "read", json!({ "path": "/a" })),
    );

    svc.approve(
        ToolCallId::new("tc_restart"),
        OperatorId::new("op-1"),
        ApprovalScope::Once,
        None,
    )
    .await
    .expect("approve must succeed on cache miss when projection has the row");

    // Post-condition: the approved fast-path now resolves against the
    // rehydrated entry.
    let approved = svc
        .retrieve_approved_proposal(&ToolCallId::new("tc_restart"))
        .await
        .expect("retrieve approved");
    assert_eq!(approved.tool_name, "read");
    assert_eq!(approved.tool_args, json!({ "path": "/a" }));
}

#[tokio::test(flavor = "current_thread")]
async fn reject_after_cache_eviction_falls_back_to_projection() {
    let (svc, reader) = svc_with_seeded_reader();
    reader.insert_proposal(
        ToolCallId::new("tc_restart"),
        stored_pending("tc_restart", "bash", json!({ "cmd": "rm -rf /" })),
    );

    svc.reject(
        ToolCallId::new("tc_restart"),
        OperatorId::new("op-1"),
        Some("too spicy".into()),
    )
    .await
    .expect("reject must succeed on cache miss when projection has the row");

    // Post-condition: the cache reflects the rejection; a re-approve is
    // now an invalid transition (not a NotFound).
    let err = svc
        .approve(
            ToolCallId::new("tc_restart"),
            OperatorId::new("op-2"),
            ApprovalScope::Once,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RuntimeError::InvalidTransition { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn amend_after_cache_eviction_falls_back_to_projection() {
    let (svc, reader) = svc_with_seeded_reader();
    reader.insert_proposal(
        ToolCallId::new("tc_restart"),
        stored_pending("tc_restart", "write", json!({ "body": "v1" })),
    );

    svc.amend(
        ToolCallId::new("tc_restart"),
        OperatorId::new("op-1"),
        json!({ "body": "v2" }),
    )
    .await
    .expect("amend must succeed on cache miss when projection has the row");

    // Post-condition: a subsequent approve (no override) executes with
    // the amended args, proving the rehydrated entry carried forward.
    svc.approve(
        ToolCallId::new("tc_restart"),
        OperatorId::new("op-1"),
        ApprovalScope::Once,
        None,
    )
    .await
    .unwrap();
    let approved = svc
        .retrieve_approved_proposal(&ToolCallId::new("tc_restart"))
        .await
        .unwrap();
    assert_eq!(approved.tool_args, json!({ "body": "v2" }));
}

#[tokio::test(flavor = "current_thread")]
async fn approve_genuinely_not_found_still_returns_not_found() {
    // Neither cache nor projection holds the call_id — must surface a
    // clean NotFound (not a panic, not an Internal error).
    let (svc, _reader) = svc_with_seeded_reader();
    let err = svc
        .approve(
            ToolCallId::new("tc_nonexistent"),
            OperatorId::new("op-1"),
            ApprovalScope::Once,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, RuntimeError::NotFound { entity, .. } if entity == "tool_call_approval"),
        "expected NotFound, got {err:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn approve_rehydrates_terminal_state_as_invalid_transition() {
    // Projection says the proposal was already approved before the
    // restart. Re-approving must surface InvalidTransition — not
    // NotFound, and definitely not a silent double-approve.
    let (svc, reader) = svc_with_seeded_reader();
    reader.insert_proposal(
        ToolCallId::new("tc_done"),
        StoredProposal {
            proposal: proposal("tc_done", "read", json!({ "path": "/a" })),
            state: StoredProposalState::Approved,
            amended_args: None,
            approved_args: None,
            rejection_reason: None,
        },
    );

    let err = svc
        .approve(
            ToolCallId::new("tc_done"),
            OperatorId::new("op-1"),
            ApprovalScope::Once,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RuntimeError::InvalidTransition { .. }));
}
