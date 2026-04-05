//! Session state machine integration tests (RFC 002).
//!
//! Validates the session lifecycle using `InMemoryStore` + `EventLog::append`.
//! Sessions are long-lived execution contexts that aggregate one or more runs.
//!
//! SessionState enum: Open | Completed | Failed | Archived
//!   (note: there is no Paused at the session level; Paused belongs to RunState)
//!   is_terminal() returns true for all states except Open.
//!
//! Projection contract:
//!   SessionCreated       → state = Open, version = 1
//!   SessionStateChanged  → state updated to transition.to, version bumped
//!
//! Read-model contract:
//!   get              → single session by ID
//!   list_by_project  → all sessions for a project, ordered by (created_at, session_id)
//!   list_active      → only Open sessions, most-recently-updated first

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, SessionCreated,
    SessionId, SessionState, SessionStateChanged, StateTransition, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::SessionReadModel,
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
    project("t_sess", "w_sess", "p_sess")
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

/// Convenience: emit a SessionCreated event.
fn session_created(evt_id: &str, session_id: &str) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: default_project(),
            session_id: SessionId::new(session_id),
        }),
    )
}

/// Convenience: emit a SessionStateChanged event.
fn session_transition(
    evt_id: &str,
    session_id: &str,
    from: Option<SessionState>,
    to: SessionState,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::SessionStateChanged(SessionStateChanged {
            project: default_project(),
            session_id: SessionId::new(session_id),
            transition: StateTransition { from, to },
        }),
    )
}

// ── 1. SessionCreated → state = Open ─────────────────────────────────────────

#[tokio::test]
async fn session_created_has_open_state() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let session_id = SessionId::new("sess_open");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::SessionCreated(SessionCreated {
                project: default_project(),
                session_id: session_id.clone(),
            }),
        )])
        .await
        .unwrap();

    let record = SessionReadModel::get(&store, &session_id)
        .await
        .unwrap()
        .expect("SessionRecord must exist after SessionCreated");

    assert_eq!(record.state, SessionState::Open);
    assert_eq!(record.session_id, session_id);
    assert_eq!(record.project, default_project());
    assert_eq!(record.version, 1);
    assert!(record.created_at >= ts);
    assert_eq!(record.created_at, record.updated_at, "fresh session: created_at == updated_at");
}

// ── 2. Open → Failed (non-happy terminal path) ───────────────────────────────

#[tokio::test]
async fn session_transitions_open_to_failed() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            session_created("e1", "sess_fail"),
            session_transition("e2", "sess_fail", Some(SessionState::Open), SessionState::Failed),
        ])
        .await
        .unwrap();

    let record = SessionReadModel::get(&store, &SessionId::new("sess_fail"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, SessionState::Failed);
    assert_eq!(record.version, 2, "version bumped by state change");
    assert!(record.updated_at >= ts);
    assert!(
        record.updated_at >= record.created_at,
        "updated_at must not precede created_at"
    );
}

// ── 3. Open → Completed (happy terminal path) ────────────────────────────────

#[tokio::test]
async fn session_transitions_open_to_completed() {
    let store = InMemoryStore::new();

    store
        .append(&[
            session_created("e1", "sess_complete"),
            session_transition(
                "e2",
                "sess_complete",
                Some(SessionState::Open),
                SessionState::Completed,
            ),
        ])
        .await
        .unwrap();

    let record = SessionReadModel::get(&store, &SessionId::new("sess_complete"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, SessionState::Completed);
    assert!(record.state.is_terminal(), "Completed is a terminal state");
    assert_eq!(record.version, 2);
}

// ── 4. Open → Archived (explicit archival path) ───────────────────────────────

#[tokio::test]
async fn session_transitions_open_to_archived() {
    let store = InMemoryStore::new();

    store
        .append(&[
            session_created("e1", "sess_archive"),
            session_transition(
                "e2",
                "sess_archive",
                Some(SessionState::Open),
                SessionState::Archived,
            ),
        ])
        .await
        .unwrap();

    let record = SessionReadModel::get(&store, &SessionId::new("sess_archive"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, SessionState::Archived);
    assert!(record.state.is_terminal(), "Archived is a terminal state");
}

// ── 5. Multi-hop: Open → Failed → Archived ───────────────────────────────────

#[tokio::test]
async fn session_multi_hop_open_to_failed_to_archived() {
    let store = InMemoryStore::new();

    store
        .append(&[
            session_created("e1", "sess_multi"),
            session_transition("e2", "sess_multi", Some(SessionState::Open), SessionState::Failed),
        ])
        .await
        .unwrap();

    let after_fail = SessionReadModel::get(&store, &SessionId::new("sess_multi"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_fail.state, SessionState::Failed);
    assert_eq!(after_fail.version, 2);

    store
        .append(&[session_transition(
            "e3",
            "sess_multi",
            Some(SessionState::Failed),
            SessionState::Archived,
        )])
        .await
        .unwrap();

    let after_archive = SessionReadModel::get(&store, &SessionId::new("sess_multi"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_archive.state, SessionState::Archived);
    assert_eq!(after_archive.version, 3, "each transition bumps version");
}

// ── 6. is_terminal() contracts ────────────────────────────────────────────────

#[test]
fn only_open_is_non_terminal() {
    assert!(!SessionState::Open.is_terminal());
    assert!(SessionState::Completed.is_terminal());
    assert!(SessionState::Failed.is_terminal());
    assert!(SessionState::Archived.is_terminal());
}

// ── 7. Multiple sessions in same project tracked independently ────────────────

#[tokio::test]
async fn multiple_sessions_in_project_tracked_independently() {
    let store = InMemoryStore::new();

    store
        .append(&[
            session_created("e1", "s_indep_a"),
            session_created("e2", "s_indep_b"),
            session_created("e3", "s_indep_c"),
        ])
        .await
        .unwrap();

    // Transition A and C; leave B open.
    store
        .append(&[
            session_transition(
                "e4", "s_indep_a", Some(SessionState::Open), SessionState::Completed,
            ),
            session_transition(
                "e5", "s_indep_c", Some(SessionState::Open), SessionState::Failed,
            ),
        ])
        .await
        .unwrap();

    let a = SessionReadModel::get(&store, &SessionId::new("s_indep_a")).await.unwrap().unwrap();
    let b = SessionReadModel::get(&store, &SessionId::new("s_indep_b")).await.unwrap().unwrap();
    let c = SessionReadModel::get(&store, &SessionId::new("s_indep_c")).await.unwrap().unwrap();

    assert_eq!(a.state, SessionState::Completed, "A was transitioned to Completed");
    assert_eq!(b.state, SessionState::Open,      "B was never transitioned");
    assert_eq!(c.state, SessionState::Failed,    "C was transitioned to Failed");

    // States don't bleed between sessions.
    assert_ne!(a.state, b.state);
    assert_ne!(b.state, c.state);
}

// ── 8. count_by_state: derive counts from list_by_project ────────────────────

#[tokio::test]
async fn count_by_state_returns_correct_counts() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Create 4 sessions: 2 Open, 1 Completed, 1 Failed.
    store
        .append(&[
            session_created("e1", "sc_open_1"),
            session_created("e2", "sc_open_2"),
            session_created("e3", "sc_done"),
            session_created("e4", "sc_fail"),
        ])
        .await
        .unwrap();

    store
        .append(&[
            session_transition(
                "e5", "sc_done", Some(SessionState::Open), SessionState::Completed,
            ),
            session_transition(
                "e6", "sc_fail", Some(SessionState::Open), SessionState::Failed,
            ),
        ])
        .await
        .unwrap();

    let all = SessionReadModel::list_by_project(&store, &default_project(), 100, 0)
        .await
        .unwrap();

    // Derive counts by state.
    let open_count = all.iter().filter(|s| s.state == SessionState::Open).count();
    let completed_count = all.iter().filter(|s| s.state == SessionState::Completed).count();
    let failed_count = all.iter().filter(|s| s.state == SessionState::Failed).count();
    let archived_count = all.iter().filter(|s| s.state == SessionState::Archived).count();
    let terminal_count = all.iter().filter(|s| s.state.is_terminal()).count();

    assert_eq!(open_count, 2,     "2 sessions are Open");
    assert_eq!(completed_count, 1, "1 session is Completed");
    assert_eq!(failed_count, 1,   "1 session is Failed");
    assert_eq!(archived_count, 0, "no sessions Archived yet");
    assert_eq!(terminal_count, 2, "2 terminal sessions total");
    assert_eq!(all.len(), 4,      "all 4 sessions tracked");
}

// ── 9. list_active returns only Open sessions, most-recently-updated first ────

#[tokio::test]
async fn list_active_returns_open_sessions_only() {
    let store = InMemoryStore::new();

    store
        .append(&[
            session_created("e1", "sa_open"),
            session_created("e2", "sa_done"),
            session_created("e3", "sa_also_open"),
        ])
        .await
        .unwrap();

    store
        .append(&[session_transition(
            "e4", "sa_done", Some(SessionState::Open), SessionState::Completed,
        )])
        .await
        .unwrap();

    let active = SessionReadModel::list_active(&store, 10).await.unwrap();

    assert_eq!(active.len(), 2, "only Open sessions appear in list_active");
    assert!(
        active.iter().all(|s| s.state == SessionState::Open),
        "every active record must be Open"
    );
    let ids: Vec<_> = active.iter().map(|s| s.session_id.as_str()).collect();
    assert!(ids.contains(&"sa_open"));
    assert!(ids.contains(&"sa_also_open"));
    assert!(!ids.contains(&"sa_done"), "Completed session must not appear in list_active");
}

// ── 10. list_by_project is sorted by (created_at, session_id) ─────────────────

#[tokio::test]
async fn list_by_project_ordered_by_created_at() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Create three sessions in one batch — they share the same stored_at timestamp.
    // With equal timestamps the secondary sort by session_id kicks in.
    store
        .append(&[
            session_created("e1", "sort_z"),
            session_created("e2", "sort_a"),
            session_created("e3", "sort_m"),
        ])
        .await
        .unwrap();

    let sessions = SessionReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();

    assert_eq!(sessions.len(), 3);
    // All share the same created_at; secondary sort is session_id lexicographic.
    let ids: Vec<_> = sessions.iter().map(|s| s.session_id.as_str()).collect();
    assert_eq!(ids, vec!["sort_a", "sort_m", "sort_z"]);
}

// ── 11. list_by_project isolation across projects ─────────────────────────────

#[tokio::test]
async fn list_by_project_scoped_to_project() {
    let store = InMemoryStore::new();
    let proj_a = project("ta", "wa", "pa");
    let proj_b = project("tb", "wb", "pb");

    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: proj_a.clone(),
                session_id: SessionId::new("sess_proj_a"),
            })),
            evt("e2", RuntimeEvent::SessionCreated(SessionCreated {
                project: proj_b.clone(),
                session_id: SessionId::new("sess_proj_b"),
            })),
        ])
        .await
        .unwrap();

    let a = SessionReadModel::list_by_project(&store, &proj_a, 10, 0).await.unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].session_id.as_str(), "sess_proj_a");
    assert_eq!(a[0].project, proj_a);

    let b = SessionReadModel::list_by_project(&store, &proj_b, 10, 0).await.unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].session_id.as_str(), "sess_proj_b");
}

// ── 12. list_by_project pagination ───────────────────────────────────────────

#[tokio::test]
async fn list_by_project_respects_limit_and_offset() {
    let store = InMemoryStore::new();

    // Create 4 sessions in distinct append calls to get distinct created_at.
    for i in 0u32..4 {
        store
            .append(&[session_created(&format!("e{i}"), &format!("sess_pg_{i:02}"))])
            .await
            .unwrap();
    }

    let page1 = SessionReadModel::list_by_project(&store, &default_project(), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].session_id.as_str(), "sess_pg_00");
    assert_eq!(page1[1].session_id.as_str(), "sess_pg_01");

    let page2 = SessionReadModel::list_by_project(&store, &default_project(), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].session_id.as_str(), "sess_pg_02");
    assert_eq!(page2[1].session_id.as_str(), "sess_pg_03");
}
