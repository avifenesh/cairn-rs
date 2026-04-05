//! RFC 003 — ingest job lifecycle end-to-end integration tests.
//!
//! Tests the full ingest job arc:
//!   1. Start an ingest job with a source and document count
//!   2. Verify the job is in Processing state
//!   3. Complete the job with success=true
//!   4. Verify the job transitions to Completed with no error_message
//!   5. Start another job and fail it with an error message
//!   6. Verify the failed job has the correct state and error_message set
//!
//! Additional coverage:
//!   - get() on unknown ID returns None
//!   - list_by_project() scopes results to the correct project
//!   - IngestJobStarted and IngestJobCompleted events are emitted
//!   - Multiple concurrent jobs tracked independently

use std::sync::Arc;

use cairn_domain::{IngestJobId, IngestJobState, ProjectKey, RuntimeEvent, SourceId};
use cairn_runtime::ingest_jobs::IngestJobService;
use cairn_runtime::services::IngestJobServiceImpl;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_ingest", "w_ingest", "p_ingest")
}

// ── Tests 1–4: start a job, verify Processing, complete with success ──────────

/// RFC 003: starting an ingest job must create a record in Processing state.
/// Completing it with success must transition to Completed.
#[tokio::test]
async fn ingest_job_start_and_complete_success() {
    let store = Arc::new(InMemoryStore::new());
    let svc = IngestJobServiceImpl::new(store.clone());

    let job_id = IngestJobId::new("job_success_1");
    let source_id = SourceId::new("src_docs");

    // ── (1) Start an ingest job ────────────────────────────────────────────
    let started = svc
        .start(&project(), job_id.clone(), Some(source_id.clone()), 12)
        .await
        .unwrap();

    // ── (2) Verify job is in Processing state ─────────────────────────────
    assert_eq!(started.id, job_id, "job ID must round-trip");
    assert_eq!(
        started.state,
        IngestJobState::Processing,
        "RFC 003: job must be Processing immediately after start"
    );
    assert_eq!(started.project, project(), "project must be recorded");
    assert_eq!(
        started.source_id,
        Some(source_id.clone()),
        "source_id must be preserved"
    );
    assert_eq!(started.document_count, 12, "document_count must be recorded");
    assert!(
        started.error_message.is_none(),
        "no error on a freshly started job"
    );
    assert!(started.created_at > 0, "created_at must be a positive timestamp");

    // get() must return the same record.
    let fetched = svc.get(&job_id).await.unwrap().unwrap();
    assert_eq!(fetched, started, "get() must return the same record as start()");

    // ── (3) Complete the job with success ─────────────────────────────────
    let completed = svc
        .complete(&project(), job_id.clone(), true, None)
        .await
        .unwrap();

    // ── (4) Verify job shows Completed ────────────────────────────────────
    assert_eq!(
        completed.state,
        IngestJobState::Completed,
        "RFC 003: job must be Completed after success=true completion"
    );
    assert!(
        completed.error_message.is_none(),
        "successful completion must not carry an error_message"
    );
    assert_eq!(completed.id, job_id, "job ID must be stable across transitions");
    assert_eq!(completed.document_count, 12, "document_count must be unchanged");

    // get() after completion must reflect the final state.
    let final_record = svc.get(&job_id).await.unwrap().unwrap();
    assert_eq!(final_record.state, IngestJobState::Completed);
}

// ── Tests 5–6: start a job and fail it, verify error_message ─────────────────

/// RFC 003: completing a job with success=false must transition to Failed and
/// preserve the error_message for operator visibility.
#[tokio::test]
async fn ingest_job_start_and_fail_with_error_message() {
    let store = Arc::new(InMemoryStore::new());
    let svc = IngestJobServiceImpl::new(store.clone());

    let job_id = IngestJobId::new("job_fail_1");

    // ── (5) Start another job ─────────────────────────────────────────────
    let started = svc
        .start(&project(), job_id.clone(), Some(SourceId::new("src_rss")), 3)
        .await
        .unwrap();

    assert_eq!(started.state, IngestJobState::Processing);

    // Fail with a descriptive error message.
    let error_msg = "connection timeout while fetching source feed".to_owned();
    let failed = svc
        .complete(
            &project(),
            job_id.clone(),
            false,
            Some(error_msg.clone()),
        )
        .await
        .unwrap();

    // ── (6) Verify failed job has error_message set ───────────────────────
    assert_eq!(
        failed.state,
        IngestJobState::Failed,
        "RFC 003: job must be Failed after success=false completion"
    );
    assert_eq!(
        failed.error_message.as_deref(),
        Some(error_msg.as_str()),
        "RFC 003: error_message must be preserved exactly on failure"
    );
    assert_eq!(failed.id, job_id);

    // get() must surface the error_message too.
    let persisted = svc.get(&job_id).await.unwrap().unwrap();
    assert_eq!(persisted.state, IngestJobState::Failed);
    assert_eq!(persisted.error_message.as_deref(), Some(error_msg.as_str()));
}

// ── IngestJobStarted and IngestJobCompleted events are emitted ────────────────

/// RFC 003: the event log must contain IngestJobStarted and IngestJobCompleted
/// events with the correct fields after the service calls.
#[tokio::test]
async fn ingest_job_emits_started_and_completed_events() {
    let store = Arc::new(InMemoryStore::new());
    let svc = IngestJobServiceImpl::new(store.clone());

    let job_id = IngestJobId::new("job_event_check");

    svc.start(&project(), job_id.clone(), Some(SourceId::new("src_e")), 7)
        .await
        .unwrap();
    svc.complete(&project(), job_id.clone(), true, None)
        .await
        .unwrap();

    let events = store.read_stream(None, 1_000).await.unwrap();

    // IngestJobStarted must be in the log.
    let started_ev = events.iter().find_map(|e| {
        if let RuntimeEvent::IngestJobStarted(ev) = &e.envelope.payload {
            if ev.job_id == job_id {
                return Some(ev.clone());
            }
        }
        None
    });
    let started_ev = started_ev.expect("IngestJobStarted event must be emitted");
    assert_eq!(started_ev.project, project());
    assert_eq!(started_ev.document_count, 7);
    assert!(started_ev.started_at > 0);

    // IngestJobCompleted must also be in the log.
    let completed_ev = events.iter().find_map(|e| {
        if let RuntimeEvent::IngestJobCompleted(ev) = &e.envelope.payload {
            if ev.job_id == job_id {
                return Some(ev.clone());
            }
        }
        None
    });
    let completed_ev = completed_ev.expect("IngestJobCompleted event must be emitted");
    assert_eq!(completed_ev.project, project());
    assert!(completed_ev.success, "success flag must be true in the event");
    assert!(completed_ev.error_message.is_none());
    assert!(completed_ev.completed_at > 0);
    assert!(
        completed_ev.completed_at >= started_ev.started_at,
        "completed_at must be >= started_at"
    );
}

// ── Failed job event carries error_message ────────────────────────────────────

/// RFC 003: IngestJobCompleted event for a failed job must carry the
/// error_message so downstream consumers can react without reading the record.
#[tokio::test]
async fn failed_job_event_carries_error_message() {
    let store = Arc::new(InMemoryStore::new());
    let svc = IngestJobServiceImpl::new(store.clone());

    let job_id = IngestJobId::new("job_fail_event");
    let error = "parse error: unexpected token at line 42".to_owned();

    svc.start(&project(), job_id.clone(), None, 1).await.unwrap();
    svc.complete(&project(), job_id.clone(), false, Some(error.clone()))
        .await
        .unwrap();

    let events = store.read_stream(None, 1_000).await.unwrap();
    let fail_ev = events.iter().find_map(|e| {
        if let RuntimeEvent::IngestJobCompleted(ev) = &e.envelope.payload {
            if ev.job_id == job_id {
                return Some(ev.clone());
            }
        }
        None
    });
    let fail_ev = fail_ev.expect("IngestJobCompleted event must exist for failed job");
    assert!(!fail_ev.success, "success must be false in the failure event");
    assert_eq!(
        fail_ev.error_message.as_deref(),
        Some(error.as_str()),
        "error_message must propagate into the event"
    );
}

// ── get() on unknown ID returns None ──────────────────────────────────────────

#[tokio::test]
async fn get_nonexistent_job_returns_none() {
    let store = Arc::new(InMemoryStore::new());
    let svc = IngestJobServiceImpl::new(store);

    let result = svc.get(&IngestJobId::new("does_not_exist")).await.unwrap();
    assert!(result.is_none(), "get() on unknown ID must return None");
}

// ── list_by_project() scopes to the correct project ───────────────────────────

/// list_by_project must only return jobs belonging to the queried project,
/// and must support pagination with limit and offset.
#[tokio::test]
async fn list_by_project_scopes_and_paginates() {
    let store = Arc::new(InMemoryStore::new());
    let svc = IngestJobServiceImpl::new(store);

    let other = ProjectKey::new("t_other", "w_other", "p_other");

    // 3 jobs for the main project, 1 for another.
    for (i, doc_count) in [(1u32, 5u32), (2, 10), (3, 15)] {
        svc.start(&project(), IngestJobId::new(format!("job_list_{i}")), None, doc_count)
            .await
            .unwrap();
    }
    svc.start(&other, IngestJobId::new("job_other"), None, 2)
        .await
        .unwrap();

    // All jobs for main project.
    let all = svc.list_by_project(&project(), 10, 0).await.unwrap();
    assert_eq!(all.len(), 3, "must return all 3 jobs for the project");
    for job in &all {
        assert_eq!(job.project, project(), "every result must belong to the queried project");
    }

    // Pagination: limit=2, offset=0 → first 2.
    let page1 = svc.list_by_project(&project(), 2, 0).await.unwrap();
    assert_eq!(page1.len(), 2);

    // Pagination: limit=2, offset=2 → last 1.
    let page2 = svc.list_by_project(&project(), 2, 2).await.unwrap();
    assert_eq!(page2.len(), 1);

    // Other project must return only its 1 job.
    let other_jobs = svc.list_by_project(&other, 10, 0).await.unwrap();
    assert_eq!(other_jobs.len(), 1);
    assert_eq!(other_jobs[0].id, IngestJobId::new("job_other"));

    // Offset past end → empty.
    let empty = svc.list_by_project(&project(), 10, 100).await.unwrap();
    assert!(empty.is_empty(), "offset past end must return empty list");
}

// ── Multiple jobs tracked independently ───────────────────────────────────────

/// Starting multiple jobs simultaneously must not bleed state between records.
/// Each job must report its own document_count, source_id, and error_message.
#[tokio::test]
async fn multiple_jobs_tracked_independently() {
    let store = Arc::new(InMemoryStore::new());
    let svc = IngestJobServiceImpl::new(store);

    let jobs = [
        (IngestJobId::new("mjob_a"), Some(SourceId::new("src_a")), 10u32),
        (IngestJobId::new("mjob_b"), Some(SourceId::new("src_b")), 20),
        (IngestJobId::new("mjob_c"), None, 5),
    ];

    for (id, src, count) in &jobs {
        svc.start(&project(), id.clone(), src.clone(), *count)
            .await
            .unwrap();
    }

    // Complete job_a → success, job_b → failure, leave job_c in Processing.
    svc.complete(&project(), IngestJobId::new("mjob_a"), true, None)
        .await
        .unwrap();
    svc.complete(
        &project(),
        IngestJobId::new("mjob_b"),
        false,
        Some("network error".to_owned()),
    )
    .await
    .unwrap();

    let a = svc.get(&IngestJobId::new("mjob_a")).await.unwrap().unwrap();
    let b = svc.get(&IngestJobId::new("mjob_b")).await.unwrap().unwrap();
    let c = svc.get(&IngestJobId::new("mjob_c")).await.unwrap().unwrap();

    assert_eq!(a.state, IngestJobState::Completed);
    assert_eq!(a.document_count, 10);
    assert_eq!(a.source_id, Some(SourceId::new("src_a")));

    assert_eq!(b.state, IngestJobState::Failed);
    assert_eq!(b.error_message.as_deref(), Some("network error"));
    assert_eq!(b.document_count, 20);

    assert_eq!(
        c.state,
        IngestJobState::Processing,
        "unfinished job must remain in Processing"
    );
    assert_eq!(c.document_count, 5);
    assert!(c.source_id.is_none());
}
