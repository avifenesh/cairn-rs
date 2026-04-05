//! RFC 002 ingest job error handling integration tests.
//!
//! Validates the ingest job lifecycle through InMemoryStore:
//! - IngestJobStarted creates a Processing record with document_count.
//! - IngestJobCompleted(success=false) transitions to Failed with error_message.
//! - Partial completion preserves the document_count from job start.
//! - list_by_project returns failed and succeeded jobs together.
//! - error_message round-trips through the event log without loss.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, IngestJobId, ProjectKey, SourceId,
    RuntimeEvent,
};
use cairn_domain::events::{IngestJobCompleted, IngestJobStarted};
use cairn_domain::IngestJobState;
use cairn_store::{
    projections::IngestJobReadModel,
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project_a() -> ProjectKey { ProjectKey::new("tenant_ingest", "ws_ingest", "proj_a") }
fn project_b() -> ProjectKey { ProjectKey::new("tenant_ingest", "ws_ingest", "proj_b") }

fn job_id(n: &str) -> IngestJobId { IngestJobId::new(format!("job_{n}")) }

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn start_event(n: &str, project: ProjectKey, doc_count: u32, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_start_{n}"),
        RuntimeEvent::IngestJobStarted(IngestJobStarted {
            project,
            job_id: job_id(n),
            source_id: Some(SourceId::new(format!("src_{n}"))),
            document_count: doc_count,
            started_at: ts,
        }),
    )
}

fn complete_success(n: &str, project: ProjectKey, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_ok_{n}"),
        RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
            project,
            job_id: job_id(n),
            success: true,
            error_message: None,
            completed_at: ts,
        }),
    )
}

fn complete_failure(n: &str, project: ProjectKey, error: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_fail_{n}"),
        RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
            project,
            job_id: job_id(n),
            success: false,
            error_message: Some(error.to_owned()),
            completed_at: ts,
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2) + (3): IngestJobStarted creates Processing record;
/// IngestJobCompleted(success=false) transitions to Failed with error_message.
#[tokio::test]
async fn failed_ingest_job_state_persisted() {
    let store = Arc::new(InMemoryStore::new());

    // (1) Start the ingest job.
    store.append(&[start_event("fail1", project_a(), 20, 1_000)]).await.unwrap();

    let started = IngestJobReadModel::get(store.as_ref(), &job_id("fail1"))
        .await.unwrap()
        .expect("job must exist after IngestJobStarted");
    assert_eq!(started.state, IngestJobState::Processing, "job must start in Processing");
    assert_eq!(started.document_count, 20, "document_count must be set from start event");
    assert!(started.error_message.is_none(), "no error yet while Processing");

    // (2) Complete with failure.
    store.append(&[complete_failure(
        "fail1", project_a(),
        "source unreachable: connection refused after 3 retries",
        2_000,
    )]).await.unwrap();

    // (3) Verify failed state.
    let failed = IngestJobReadModel::get(store.as_ref(), &job_id("fail1"))
        .await.unwrap().unwrap();
    assert_eq!(failed.state, IngestJobState::Failed, "state must be Failed after success=false");
    assert!(failed.error_message.is_some(), "error_message must be set on failure");
    assert!(
        failed.error_message.as_ref().unwrap().contains("connection refused"),
        "error_message must contain the failure reason"
    );
    assert_eq!(failed.updated_at, 2_000, "updated_at must reflect completion timestamp");
}

/// (4): Partial completion — document_count from IngestJobStarted is preserved
/// even when the job fails mid-way. The count reflects intended scope,
/// not successfully processed documents.
#[tokio::test]
async fn partial_completion_preserves_document_count() {
    let store = Arc::new(InMemoryStore::new());

    // Start with 500 documents targeted.
    store.append(&[start_event("partial", project_a(), 500, 1_000)]).await.unwrap();

    // Fail after processing some documents (the error says 237 of 500 processed).
    store.append(&[complete_failure(
        "partial", project_a(),
        "disk quota exceeded after processing 237 of 500 documents",
        3_000,
    )]).await.unwrap();

    let job = IngestJobReadModel::get(store.as_ref(), &job_id("partial"))
        .await.unwrap().unwrap();

    // (4) document_count is preserved from the start event.
    assert_eq!(
        job.document_count, 500,
        "document_count must reflect the originally intended scope (500), not partial processed count"
    );
    assert_eq!(job.state, IngestJobState::Failed);
    assert!(
        job.error_message.as_ref().unwrap().contains("237 of 500"),
        "error_message preserves the partial progress detail"
    );
}

/// (5): list_by_project returns failed and succeeded jobs together,
/// letting operators see the full job history for a project.
#[tokio::test]
async fn list_by_project_returns_failed_and_succeeded_jobs() {
    let store = Arc::new(InMemoryStore::new());

    // 2 failed, 2 succeeded, 1 still processing.
    store.append(&[
        start_event("s1",  project_a(), 100, 1_000),
        start_event("s2",  project_a(), 200, 2_000),
        start_event("f1",  project_a(), 50,  3_000),
        start_event("f2",  project_a(), 75,  4_000),
        start_event("run", project_a(), 300, 5_000),
    ]).await.unwrap();

    store.append(&[
        complete_success("s1", project_a(), 6_000),
        complete_success("s2", project_a(), 7_000),
        complete_failure("f1", project_a(), "parse error: invalid UTF-8 in document 12", 8_000),
        complete_failure("f2", project_a(), "timeout waiting for embeddings API", 9_000),
        // "run" is left in Processing.
    ]).await.unwrap();

    let jobs = IngestJobReadModel::list_by_project(store.as_ref(), &project_a(), 100, 0)
        .await.unwrap();

    assert_eq!(jobs.len(), 5, "all 5 jobs must appear in list_by_project");

    let completed: Vec<_> = jobs.iter().filter(|j| j.state == IngestJobState::Completed).collect();
    let failed: Vec<_>    = jobs.iter().filter(|j| j.state == IngestJobState::Failed).collect();
    let processing: Vec<_> = jobs.iter().filter(|j| j.state == IngestJobState::Processing).collect();

    assert_eq!(completed.len(), 2,   "2 completed jobs");
    assert_eq!(failed.len(), 2,      "2 failed jobs");
    assert_eq!(processing.len(), 1,  "1 still-processing job");

    // Verify the error messages on the failed jobs.
    assert!(failed.iter().any(|j| j.error_message.as_ref().map_or(false, |e| e.contains("UTF-8"))));
    assert!(failed.iter().any(|j| j.error_message.as_ref().map_or(false, |e| e.contains("timeout"))));

    // Completed jobs have no error_message.
    assert!(completed.iter().all(|j| j.error_message.is_none()),
        "completed jobs must have no error_message");
}

/// (6): error_message round-trips through the event log without loss.
/// Tests Unicode, long messages, special characters, and structured errors.
#[tokio::test]
async fn error_message_round_trips_without_loss() {
    let store = Arc::new(InMemoryStore::new());

    let long_msg = "E".repeat(2048);
    let test_cases: &[(&str, &str)] = &[
        ("ascii",    "connection refused: TCP connect to 10.0.0.1:9200 failed"),
        ("unicode",  "解析エラー: 無効なUTF-8シーケンス at byte offset 1024"),
        ("json_err", r#"{"code": "RATE_LIMIT", "retry_after": 60, "limit": "100/min"}"#),
        ("multiline","Step 1: fetch — ok\nStep 2: parse — ok\nStep 3: embed — FAILED: 503"),
        ("long",     &long_msg),
    ];

    for (i, (suffix, error_msg)) in test_cases.iter().enumerate() {
        let ts = (i as u64 + 1) * 1_000;
        store.append(&[
            start_event(suffix, project_a(), 10, ts),
            complete_failure(suffix, project_a(), error_msg, ts + 500),
        ]).await.unwrap();
    }

    // Read each job back and verify the error_message survived.
    for (suffix, expected_error) in test_cases {
        let job = IngestJobReadModel::get(store.as_ref(), &job_id(suffix))
            .await.unwrap()
            .unwrap_or_else(|| panic!("job_{suffix} must exist"));

        assert_eq!(job.state, IngestJobState::Failed);
        let stored_error = job.error_message.as_deref()
            .unwrap_or_else(|| panic!("job_{suffix} must have error_message"));
        assert_eq!(
            stored_error, *expected_error,
            "error_message for job_{suffix} must round-trip exactly (len: {})",
            expected_error.len()
        );
    }
}

/// Project isolation: list_by_project for project_a does not return project_b jobs.
#[tokio::test]
async fn ingest_jobs_are_project_scoped() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        start_event("pa1", project_a(), 10, 1_000),
        start_event("pb1", project_b(), 20, 2_000),
        complete_failure("pa1", project_a(), "failed in project_a", 3_000),
        complete_success("pb1", project_b(), 4_000),
    ]).await.unwrap();

    let a_jobs = IngestJobReadModel::list_by_project(store.as_ref(), &project_a(), 100, 0)
        .await.unwrap();
    let b_jobs = IngestJobReadModel::list_by_project(store.as_ref(), &project_b(), 100, 0)
        .await.unwrap();

    assert_eq!(a_jobs.len(), 1, "project_a must see only 1 job");
    assert_eq!(a_jobs[0].state, IngestJobState::Failed);
    assert!(a_jobs.iter().all(|j| j.project == project_a()),
        "all project_a jobs must be in project_a");

    assert_eq!(b_jobs.len(), 1, "project_b must see only 1 job");
    assert_eq!(b_jobs[0].state, IngestJobState::Completed);
    assert!(
        !b_jobs.iter().any(|j| j.project == project_a()),
        "project_b must not see project_a jobs"
    );
}

/// read_by_entity for IngestJob returns both start and completion events.
#[tokio::test]
async fn entity_scoped_read_returns_ingest_job_events() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        start_event("entity1", project_a(), 15, 1_000),
        complete_failure("entity1", project_a(), "quota exceeded", 2_000),
    ]).await.unwrap();

    use cairn_store::event_log::EntityRef;
    let job_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::IngestJob(job_id("entity1")),
        None, 100,
    ).await.unwrap();

    assert_eq!(job_events.len(), 2, "IngestJobStarted + IngestJobCompleted = 2 events");
    assert!(matches!(&job_events[0].envelope.payload, RuntimeEvent::IngestJobStarted(s) if s.job_id == job_id("entity1")));
    assert!(matches!(&job_events[1].envelope.payload, RuntimeEvent::IngestJobCompleted(c) if !c.success));
}
