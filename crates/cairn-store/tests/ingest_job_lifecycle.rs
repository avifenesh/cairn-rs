//! Ingest job lifecycle integration tests (RFC 003).
//!
//! Validates the memory ingest pipeline using `InMemoryStore` + `EventLog::append`.
//! Ingest jobs represent the process of pulling documents from a knowledge source,
//! chunking them, and embedding them into the memory store.
//!
//! Projection contract:
//!   IngestJobStarted   → state = Processing  (Pending is a domain state but not emitted)
//!   IngestJobCompleted → state = Completed (success=true) or Failed (success=false)
//!
//! Read-model contract:
//!   get              → single job by ID
//!   list_by_project  → all jobs for a project, ordered by created_at ascending

use cairn_domain::{
    EventEnvelope, EventId, EventSource, IngestJobCompleted, IngestJobId, IngestJobStarted,
    IngestJobState, ProjectId, ProjectKey, RuntimeEvent, SourceId, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::IngestJobReadModel,
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str, workspace: &str, proj: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
        project_id: ProjectId::new(proj),
    }
}

fn default_project() -> ProjectKey {
    project("t_ingest", "w_ingest", "p_ingest")
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

// ── 1. IngestJobStarted → state = Processing ─────────────────────────────────

#[tokio::test]
async fn ingest_job_started_shows_processing_state() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let job_id = IngestJobId::new("job_001");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::IngestJobStarted(IngestJobStarted {
                project: default_project(),
                job_id: job_id.clone(),
                source_id: Some(SourceId::new("src_docs")),
                document_count: 42,
                started_at: ts,
            }),
        )])
        .await
        .unwrap();

    let record = IngestJobReadModel::get(&store, &job_id)
        .await
        .unwrap()
        .expect("IngestJobRecord must exist after IngestJobStarted");

    // The projection maps IngestJobStarted directly to Processing
    // (Pending is a domain state but is never written by the event pipeline).
    assert_eq!(record.state, IngestJobState::Processing);
    assert_eq!(record.id, job_id);
    assert_eq!(record.project, default_project());
    assert_eq!(record.source_id, Some(SourceId::new("src_docs")));
    assert_eq!(record.document_count, 42);
    assert_eq!(record.created_at, ts);
    assert_eq!(record.updated_at, ts);
    assert!(record.error_message.is_none());
}

// ── 2. IngestJobCompleted (success) → state = Completed ──────────────────────

#[tokio::test]
async fn ingest_job_completed_successfully_updates_state() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let job_id = IngestJobId::new("job_success");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::IngestJobStarted(IngestJobStarted {
                project: default_project(),
                job_id: job_id.clone(),
                source_id: Some(SourceId::new("src_wiki")),
                document_count: 100,
                started_at: ts,
            }),
        )])
        .await
        .unwrap();

    // Verify Processing state before completion.
    let before = IngestJobReadModel::get(&store, &job_id).await.unwrap().unwrap();
    assert_eq!(before.state, IngestJobState::Processing);

    store
        .append(&[evt(
            "e2",
            RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
                project: default_project(),
                job_id: job_id.clone(),
                success: true,
                error_message: None,
                completed_at: ts + 5_000,
            }),
        )])
        .await
        .unwrap();

    let after = IngestJobReadModel::get(&store, &job_id).await.unwrap().unwrap();

    assert_eq!(after.state, IngestJobState::Completed);
    assert!(after.error_message.is_none(), "successful job has no error message");
    assert_eq!(after.updated_at, ts + 5_000, "updated_at reflects completion time");
    assert_eq!(after.created_at, ts, "created_at must not change");
    assert_eq!(after.document_count, 100, "document_count is preserved from start event");
}

// ── 3. IngestJobCompleted (failure) → state = Failed with error ───────────────

#[tokio::test]
async fn ingest_job_completed_with_failure_sets_failed_state() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let job_id = IngestJobId::new("job_fail");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: default_project(),
                    job_id: job_id.clone(),
                    source_id: Some(SourceId::new("src_broken")),
                    document_count: 0,
                    started_at: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
                    project: default_project(),
                    job_id: job_id.clone(),
                    success: false,
                    error_message: Some("connection refused: source unreachable".to_owned()),
                    completed_at: ts + 1_000,
                }),
            ),
        ])
        .await
        .unwrap();

    let record = IngestJobReadModel::get(&store, &job_id).await.unwrap().unwrap();

    assert_eq!(record.state, IngestJobState::Failed);
    assert_eq!(
        record.error_message.as_deref(),
        Some("connection refused: source unreachable")
    );
    assert_eq!(record.updated_at, ts + 1_000);
}

// ── 4. list_by_project returns only the project's jobs ────────────────────────

#[tokio::test]
async fn list_by_project_returns_only_matching_project_jobs() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let proj_a = project("ta", "wa", "pa");
    let proj_b = project("tb", "wb", "pb");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: proj_a.clone(),
                    job_id: IngestJobId::new("job_a1"),
                    source_id: None,
                    document_count: 10,
                    started_at: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: proj_a.clone(),
                    job_id: IngestJobId::new("job_a2"),
                    source_id: None,
                    document_count: 20,
                    started_at: ts + 1,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: proj_b.clone(),
                    job_id: IngestJobId::new("job_b1"),
                    source_id: None,
                    document_count: 5,
                    started_at: ts + 2,
                }),
            ),
        ])
        .await
        .unwrap();

    let jobs_a = IngestJobReadModel::list_by_project(&store, &proj_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(jobs_a.len(), 2, "project A has 2 jobs");
    assert!(jobs_a.iter().all(|j| j.project == proj_a));

    let ids_a: Vec<_> = jobs_a.iter().map(|j| j.id.as_str()).collect();
    assert!(ids_a.contains(&"job_a1"));
    assert!(ids_a.contains(&"job_a2"));
    assert!(!ids_a.contains(&"job_b1"), "project B job must not appear in A's list");

    let jobs_b = IngestJobReadModel::list_by_project(&store, &proj_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(jobs_b.len(), 1);
    assert_eq!(jobs_b[0].id.as_str(), "job_b1");

    // Project with no jobs returns empty.
    let jobs_c = IngestJobReadModel::list_by_project(
        &store,
        &project("tc", "wc", "pc"),
        10,
        0,
    )
    .await
    .unwrap();
    assert!(jobs_c.is_empty());
}

// ── 5. list_by_project orders by created_at ascending ────────────────────────

#[tokio::test]
async fn list_by_project_ordered_by_created_at_ascending() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Append in reverse chronological order to prove sorting.
    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: default_project(),
                    job_id: IngestJobId::new("job_newest"),
                    source_id: None,
                    document_count: 1,
                    started_at: ts + 200,   // latest
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: default_project(),
                    job_id: IngestJobId::new("job_oldest"),
                    source_id: None,
                    document_count: 1,
                    started_at: ts,         // earliest
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: default_project(),
                    job_id: IngestJobId::new("job_middle"),
                    source_id: None,
                    document_count: 1,
                    started_at: ts + 100,   // middle
                }),
            ),
        ])
        .await
        .unwrap();

    let jobs = IngestJobReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();

    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0].id.as_str(), "job_oldest",  "earliest created_at first");
    assert_eq!(jobs[1].id.as_str(), "job_middle",  "middle created_at second");
    assert_eq!(jobs[2].id.as_str(), "job_newest",  "latest created_at last");
}

// ── 6. list_by_project pagination ────────────────────────────────────────────

#[tokio::test]
async fn list_by_project_respects_limit_and_offset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u64..5 {
        store
            .append(&[evt(
                &format!("e{i}"),
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: default_project(),
                    job_id: IngestJobId::new(format!("job_pg_{i:02}")),
                    source_id: None,
                    document_count: i as u32,
                    started_at: ts + i * 10,
                }),
            )])
            .await
            .unwrap();
    }

    let page1 = IngestJobReadModel::list_by_project(&store, &default_project(), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].id.as_str(), "job_pg_00");
    assert_eq!(page1[1].id.as_str(), "job_pg_01");

    let page2 = IngestJobReadModel::list_by_project(&store, &default_project(), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].id.as_str(), "job_pg_02");
    assert_eq!(page2[1].id.as_str(), "job_pg_03");

    let page3 = IngestJobReadModel::list_by_project(&store, &default_project(), 2, 4)
        .await
        .unwrap();
    assert_eq!(page3.len(), 1);
    assert_eq!(page3[0].id.as_str(), "job_pg_04");
}

// ── 7. document_count preserved through the full lifecycle ───────────────────

#[tokio::test]
async fn document_count_preserved_through_completion() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let job_id = IngestJobId::new("job_docs");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::IngestJobStarted(IngestJobStarted {
                    project: default_project(),
                    job_id: job_id.clone(),
                    source_id: Some(SourceId::new("src_corpus")),
                    document_count: 1_234,
                    started_at: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
                    project: default_project(),
                    job_id: job_id.clone(),
                    success: true,
                    error_message: None,
                    completed_at: ts + 30_000,
                }),
            ),
        ])
        .await
        .unwrap();

    let record = IngestJobReadModel::get(&store, &job_id).await.unwrap().unwrap();

    assert_eq!(record.state, IngestJobState::Completed);
    assert_eq!(record.document_count, 1_234,
        "document_count from the start event must survive completion");
    assert_eq!(record.source_id, Some(SourceId::new("src_corpus")));
}

// ── 8. Source-less job (no source_id) is valid ────────────────────────────────

#[tokio::test]
async fn ingest_job_without_source_id_is_valid() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let job_id = IngestJobId::new("job_no_source");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::IngestJobStarted(IngestJobStarted {
                project: default_project(),
                job_id: job_id.clone(),
                source_id: None,           // manual / ad-hoc ingest
                document_count: 3,
                started_at: ts,
            }),
        )])
        .await
        .unwrap();

    let record = IngestJobReadModel::get(&store, &job_id).await.unwrap().unwrap();

    assert!(record.source_id.is_none(), "source_id=None is valid for ad-hoc ingests");
    assert_eq!(record.document_count, 3);
    assert_eq!(record.state, IngestJobState::Processing);
}

// ── 9. Mixed states in one project reflect independent job tracking ────────────

#[tokio::test]
async fn mixed_state_jobs_tracked_independently() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Job A: completed successfully.
    // Job B: failed.
    // Job C: still processing (no completion event).
    store
        .append(&[
            evt("e1", RuntimeEvent::IngestJobStarted(IngestJobStarted {
                project: default_project(),
                job_id: IngestJobId::new("job_mix_a"),
                source_id: None,
                document_count: 50,
                started_at: ts,
            })),
            evt("e2", RuntimeEvent::IngestJobStarted(IngestJobStarted {
                project: default_project(),
                job_id: IngestJobId::new("job_mix_b"),
                source_id: None,
                document_count: 10,
                started_at: ts + 1,
            })),
            evt("e3", RuntimeEvent::IngestJobStarted(IngestJobStarted {
                project: default_project(),
                job_id: IngestJobId::new("job_mix_c"),
                source_id: None,
                document_count: 20,
                started_at: ts + 2,
            })),
            evt("e4", RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
                project: default_project(),
                job_id: IngestJobId::new("job_mix_a"),
                success: true,
                error_message: None,
                completed_at: ts + 100,
            })),
            evt("e5", RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
                project: default_project(),
                job_id: IngestJobId::new("job_mix_b"),
                success: false,
                error_message: Some("timeout".to_owned()),
                completed_at: ts + 200,
            })),
        ])
        .await
        .unwrap();

    let a = IngestJobReadModel::get(&store, &IngestJobId::new("job_mix_a")).await.unwrap().unwrap();
    let b = IngestJobReadModel::get(&store, &IngestJobId::new("job_mix_b")).await.unwrap().unwrap();
    let c = IngestJobReadModel::get(&store, &IngestJobId::new("job_mix_c")).await.unwrap().unwrap();

    assert_eq!(a.state, IngestJobState::Completed);
    assert!(a.error_message.is_none());

    assert_eq!(b.state, IngestJobState::Failed);
    assert_eq!(b.error_message.as_deref(), Some("timeout"));

    assert_eq!(c.state, IngestJobState::Processing, "job_mix_c has no completion event");

    // list_by_project sees all three, ordered by created_at.
    let all = IngestJobReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].id.as_str(), "job_mix_a");
    assert_eq!(all[1].id.as_str(), "job_mix_b");
    assert_eq!(all[2].id.as_str(), "job_mix_c");
}
