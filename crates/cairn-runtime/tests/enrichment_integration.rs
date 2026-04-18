#![cfg(feature = "in-memory-runtime")]

//! Proves RuntimeEnrichment consumes store-backed data end-to-end.
//! Worker 8 should depend on this seam for SSE/API enrichment.

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::{
    ApprovalService, ApprovalServiceImpl, RuntimeEnrichment, SessionService, SessionServiceImpl,
    StoreBackedEnrichment, TaskService, TaskServiceImpl,
};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

#[tokio::test]
async fn enrichment_returns_task_title_and_state() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store.clone());
    let enrichment = StoreBackedEnrichment::new(store.clone());

    task_svc
        .submit(&project(), TaskId::new("t1"), None, None, 0)
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("t1"), "w".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("t1")).await.unwrap();

    let enriched = enrichment
        .enrich_task(&TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(enriched.state, TaskState::Running);
    assert_eq!(enriched.lease_owner.as_deref(), Some("w"));
    // title/description are None until set — but the seam works
    assert!(enriched.title.is_none());
}

#[tokio::test]
async fn enrichment_returns_approval_state() {
    let store = Arc::new(InMemoryStore::new());
    let approval_svc = ApprovalServiceImpl::new(store.clone());
    let enrichment = StoreBackedEnrichment::new(store.clone());

    approval_svc
        .request(
            &project(),
            ApprovalId::new("a1"),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    let enriched = enrichment
        .enrich_approval(&ApprovalId::new("a1"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(enriched.requirement, ApprovalRequirement::Required);
    assert!(enriched.decision.is_none());

    approval_svc
        .resolve(&ApprovalId::new("a1"), ApprovalDecision::Approved)
        .await
        .unwrap();

    let enriched = enrichment
        .enrich_approval(&ApprovalId::new("a1"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(enriched.decision, Some(ApprovalDecision::Approved));
}

#[tokio::test]
async fn enrichment_returns_session_state() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let enrichment = StoreBackedEnrichment::new(store.clone());

    session_svc
        .create(&project(), SessionId::new("s1"))
        .await
        .unwrap();

    let enriched = enrichment
        .enrich_session(&SessionId::new("s1"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(enriched.state, SessionState::Open);
}

/// Exercises approval enrichment repeatedly across multiple approvals
/// with interleaved mutations to confirm composition stays correct.
#[tokio::test]
async fn approval_enrichment_composes_under_repeated_exercise() {
    let store = Arc::new(InMemoryStore::new());
    let approval_svc = ApprovalServiceImpl::new(store.clone());
    let enrichment = StoreBackedEnrichment::new(store.clone());
    let p = project();

    // Create 3 approvals
    for i in 1..=3 {
        approval_svc
            .request(
                &p,
                ApprovalId::new(format!("a{i}")),
                Some(RunId::new("r1")),
                None,
                ApprovalRequirement::Required,
            )
            .await
            .unwrap();
    }

    // All 3 should enrich as pending
    for i in 1..=3 {
        let e = enrichment
            .enrich_approval(&ApprovalId::new(format!("a{i}")))
            .await
            .unwrap()
            .unwrap();
        assert!(e.decision.is_none(), "a{i} should be pending");
        assert_eq!(e.run_id, Some(RunId::new("r1")));
    }

    // Resolve a1=Approved, a2=Rejected, leave a3 pending
    approval_svc
        .resolve(&ApprovalId::new("a1"), ApprovalDecision::Approved)
        .await
        .unwrap();
    approval_svc
        .resolve(&ApprovalId::new("a2"), ApprovalDecision::Rejected)
        .await
        .unwrap();

    // Enrich all 3 again — each should reflect its own state
    let e1 = enrichment
        .enrich_approval(&ApprovalId::new("a1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e1.decision, Some(ApprovalDecision::Approved));

    let e2 = enrichment
        .enrich_approval(&ApprovalId::new("a2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e2.decision, Some(ApprovalDecision::Rejected));

    let e3 = enrichment
        .enrich_approval(&ApprovalId::new("a3"))
        .await
        .unwrap()
        .unwrap();
    assert!(e3.decision.is_none(), "a3 should still be pending");

    // Enrich a1 again — should still be Approved (idempotent read)
    let e1_again = enrichment
        .enrich_approval(&ApprovalId::new("a1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e1_again.decision, e1.decision);
}

/// Exercises task enrichment through full lifecycle churn:
/// submit → claim → start → complete, enriching at each step.
#[tokio::test]
async fn task_enrichment_tracks_lifecycle_churn() {
    let store = Arc::new(InMemoryStore::new());
    let task_svc = TaskServiceImpl::new(store.clone());
    let enrichment = StoreBackedEnrichment::new(store.clone());
    let p = project();

    task_svc
        .submit(&p, TaskId::new("t1"), None, None, 0)
        .await
        .unwrap();

    let e = enrichment
        .enrich_task(&TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e.state, TaskState::Queued);
    assert!(e.lease_owner.is_none());

    task_svc
        .claim(&TaskId::new("t1"), "worker-x".to_owned(), 60_000)
        .await
        .unwrap();

    let e = enrichment
        .enrich_task(&TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e.state, TaskState::Leased);
    assert_eq!(e.lease_owner.as_deref(), Some("worker-x"));

    task_svc.start(&TaskId::new("t1")).await.unwrap();

    let e = enrichment
        .enrich_task(&TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e.state, TaskState::Running);

    task_svc.complete(&TaskId::new("t1")).await.unwrap();

    let e = enrichment
        .enrich_task(&TaskId::new("t1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e.state, TaskState::Completed);
}

#[tokio::test]
async fn enrichment_returns_none_for_missing() {
    let store = Arc::new(InMemoryStore::new());
    let enrichment = StoreBackedEnrichment::new(store);

    assert!(enrichment
        .enrich_task(&TaskId::new("nonexistent"))
        .await
        .unwrap()
        .is_none());
}
