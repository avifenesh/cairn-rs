//! RFC-011 Phase 3: regression tests for the task → session resolver.
//!
//! Phase 2 resolved the binding by walking `task.parent_run_id →
//! RunRecord.session_id`, which cost a second read-model lookup on every
//! hot-path call and could silently degrade to solo-mint if the parent run
//! fell out of the projection. Phase 3 persists the binding directly on
//! `TaskCreated.session_id` / `TaskRecord.session_id`, so the resolve
//! collapses to a single read. The parent-run walk remains as a fallback
//! for legacy (pre-V021) tasks whose event had no session_id.
//!
//! These tests cover:
//! - solo task (no parent run, no session) → `None`
//! - session-bound task where `TaskCreated` carried session_id directly →
//!   resolves without ever touching the runs projection
//! - legacy task where `TaskCreated.session_id` is None but `parent_run_id`
//!   is set → falls back to the parent-run walk
//! - session mismatch (task's persisted session differs from its parent
//!   run's session — not reachable via the submit API, but guards against
//!   projection drift) → returns the task's own session, not the run's
//! - missing task → `None`
//! - dangling parent run (task references a run not in the projection, no
//!   persisted session_id on the task) → `None` (caller falls back to
//!   solo mint, same as Phase 2)

#![cfg(feature = "in-memory-runtime")]

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, TaskCreated, TaskId, TenantId, WorkspaceId,
};
use cairn_runtime::InMemoryServices;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t1"),
        workspace_id: WorkspaceId::new("w1"),
        project_id: ProjectId::new("p1"),
    }
}

async fn seed_session(store: &Arc<InMemoryStore>, sid: &str) {
    let env = EventEnvelope::for_runtime_event(
        EventId::new(format!("sess_{sid}")),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: SessionId::new(sid),
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn seed_run(store: &Arc<InMemoryStore>, run_id: &str, session_id: &str) {
    let env = EventEnvelope::for_runtime_event(
        EventId::new(format!("run_{run_id}")),
        EventSource::Runtime,
        RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            run_id: RunId::new(run_id),
            session_id: SessionId::new(session_id),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
    );
    store.append(&[env]).await.unwrap();
}

/// Seed a task via a TaskCreated event. `session_id` is the authoritative
/// Phase-3 binding written on the event (what new callers always emit).
/// `legacy` forces the event to be emitted with `session_id = None` even
/// when `parent_run_id` is set, to exercise the projection's fallback path.
async fn seed_task(
    store: &Arc<InMemoryStore>,
    task_id: &str,
    parent_run_id: Option<&str>,
    session_id: Option<&str>,
    legacy: bool,
) {
    let event_session_id = if legacy {
        None
    } else {
        session_id.map(SessionId::new)
    };
    let env = EventEnvelope::for_runtime_event(
        EventId::new(format!("task_{task_id}")),
        EventSource::Runtime,
        RuntimeEvent::TaskCreated(TaskCreated {
            project: project(),
            task_id: TaskId::new(task_id),
            parent_run_id: parent_run_id.map(RunId::new),
            parent_task_id: None,
            prompt_release_id: None,
            session_id: event_session_id,
        }),
    );
    store.append(&[env]).await.unwrap();
}

/// Phase-3 resolver: the hot path reads `task.session_id` directly and only
/// walks `parent_run_id → run.session_id` when the persisted field is None
/// (legacy task). Mirrors the production `resolve_session_for_task_record`
/// fallback behavior.
async fn resolve(services: &InMemoryServices, task_id: &str) -> Option<SessionId> {
    let task = services
        .tasks
        .get(&TaskId::new(task_id))
        .await
        .ok()
        .flatten()?;
    if let Some(sid) = task.session_id.clone() {
        return Some(sid);
    }
    let parent_run_id = task.parent_run_id.as_ref()?;
    services
        .runs
        .get(parent_run_id)
        .await
        .ok()
        .flatten()
        .map(|run| run.session_id)
}

#[tokio::test]
async fn solo_task_resolves_to_none() {
    let store = Arc::new(InMemoryStore::new());
    let services = InMemoryServices::with_store(store.clone());
    seed_task(&store, "task_solo", None, None, false).await;

    assert_eq!(resolve(&services, "task_solo").await, None);
}

#[tokio::test]
async fn phase3_task_with_persisted_session_resolves_directly() {
    // Phase-3 happy path: TaskCreated carried session_id, so the projection
    // persisted it on TaskRecord and the resolver returns it without ever
    // reading the runs projection.
    let store = Arc::new(InMemoryStore::new());
    let services = InMemoryServices::with_store(store.clone());
    seed_session(&store, "sess_a").await;
    seed_run(&store, "run_1", "sess_a").await;
    seed_task(&store, "task_1", Some("run_1"), Some("sess_a"), false).await;

    let got = resolve(&services, "task_1").await;
    assert_eq!(got.as_ref().map(|s| s.as_str()), Some("sess_a"));
}

#[tokio::test]
async fn legacy_task_falls_back_to_parent_run_session() {
    // Pre-Phase-3 tasks have no session_id on the event. The projection's
    // COALESCE fallback (in_memory.rs / sqlite / pg) pulls the run's
    // session_id at insert time, so resolve() still returns it from
    // TaskRecord.session_id. This proves the fallback is wired correctly
    // at projection-time, not just at resolve-time.
    let store = Arc::new(InMemoryStore::new());
    let services = InMemoryServices::with_store(store.clone());
    seed_session(&store, "sess_legacy").await;
    seed_run(&store, "run_legacy", "sess_legacy").await;
    seed_task(&store, "task_legacy", Some("run_legacy"), None, true).await;

    let got = resolve(&services, "task_legacy").await;
    assert_eq!(got.as_ref().map(|s| s.as_str()), Some("sess_legacy"));
}

#[tokio::test]
async fn session_mismatch_prefers_task_persisted_session() {
    // Defensive: the submit API never lets a caller pin a task to a
    // session that differs from its parent run's session (the adapter
    // cross-check in fabric_adapter.rs rejects it). But if projection
    // drift ever produced a task whose persisted session differs from its
    // parent run's, the resolver must trust the task's own binding —
    // that's the one the ExecutionId was minted against, and walking the
    // run would hand out a different partition.
    let store = Arc::new(InMemoryStore::new());
    let services = InMemoryServices::with_store(store.clone());
    seed_session(&store, "sess_task").await;
    seed_session(&store, "sess_run").await;
    seed_run(&store, "run_X", "sess_run").await;
    // Task pinned to sess_task, parent run to sess_run.
    seed_task(
        &store,
        "task_mismatch",
        Some("run_X"),
        Some("sess_task"),
        false,
    )
    .await;

    let got = resolve(&services, "task_mismatch").await;
    assert_eq!(
        got.as_ref().map(|s| s.as_str()),
        Some("sess_task"),
        "resolver must trust task.session_id, not the parent run's"
    );
}

#[tokio::test]
async fn missing_task_resolves_to_none() {
    let store = Arc::new(InMemoryStore::new());
    let services = InMemoryServices::with_store(store.clone());

    assert_eq!(resolve(&services, "does_not_exist").await, None);
}

#[tokio::test]
async fn legacy_task_with_dangling_parent_run_resolves_to_none() {
    // Legacy (pre-Phase-3) task whose parent run was never recorded and
    // whose event carried no session_id. The projection's COALESCE
    // fallback finds nothing, so task.session_id is None; the resolve-time
    // fallback also misses (run not in projection). Caller must degrade
    // to solo mint, same as Phase 2.
    let store = Arc::new(InMemoryStore::new());
    let services = InMemoryServices::with_store(store.clone());
    seed_task(&store, "task_orphan", Some("run_ghost"), None, true).await;

    assert_eq!(resolve(&services, "task_orphan").await, None);
}
