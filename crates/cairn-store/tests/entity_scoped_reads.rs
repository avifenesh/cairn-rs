//! RFC 002 entity-scoped event read integration tests.
//!
//! Validates the entity-scoped event log contract:
//! - read_by_entity filters to events that match the specific entity ID.
//! - Events from unrelated entities of the same type are excluded.
//! - Cursor-based pagination (after: Some(position)) skips earlier events.
//! - head_position reflects the last appended event position.
//! - read_stream returns all events globally in append order.
//! - EntityRef matching works for Session, Run, Task, Approval, and Checkpoint.

use std::sync::Arc;

use cairn_domain::events::{RunStateChanged, TaskStateChanged};
use cairn_domain::lifecycle::{RunState, TaskState};
use cairn_domain::policy::ApprovalRequirement;
use cairn_domain::{
    ApprovalId, ApprovalRequested, CheckpointDisposition, CheckpointId, CheckpointRecorded,
    EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, StateTransition, TaskCreated, TaskId,
};
use cairn_store::{event_log::EntityRef, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_esc", "ws_esc", "proj_esc")
}

fn session_id(n: &str) -> SessionId {
    SessionId::new(format!("sess_{n}"))
}

fn run_id(n: &str) -> RunId {
    RunId::new(format!("run_{n}"))
}

fn task_id(n: &str) -> TaskId {
    TaskId::new(format!("task_{n}"))
}

fn approval_id(n: &str) -> ApprovalId {
    ApprovalId::new(format!("appr_{n}"))
}

fn checkpoint_id(n: &str) -> CheckpointId {
    CheckpointId::new(format!("ckpt_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): Append events for 3 entities (Session, Run, Task).
/// read_by_entity for each returns ONLY its own events.
#[tokio::test]
async fn read_by_entity_returns_only_matching_events() {
    let store = Arc::new(InMemoryStore::new());

    // Session events.
    store
        .append(&[
            ev(
                "evt_sess_a_created",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id("a"),
                }),
            ),
            ev(
                "evt_sess_b_created",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id("b"),
                }),
            ),
        ])
        .await
        .unwrap();

    // Run events for two distinct runs.
    store
        .append(&[
            ev(
                "evt_run_1_created",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id("a"),
                    run_id: run_id("1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                "evt_run_1_state",
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project(),
                    run_id: run_id("1"),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
            ev(
                "evt_run_2_created",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id("b"),
                    run_id: run_id("2"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Task event for task_1 (child of run_1).
    store
        .append(&[
            ev(
                "evt_task_1_created",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("1"),
                    parent_run_id: Some(run_id("1")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            ev(
                "evt_task_1_state",
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: task_id("1"),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Leased,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // --- session_a: exactly 1 event ---
    let sess_a_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(session_id("a")),
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(
        sess_a_events.len(),
        1,
        "session_a must have exactly 1 event"
    );
    assert!(matches!(
        &sess_a_events[0].envelope.payload,
        RuntimeEvent::SessionCreated(e) if e.session_id == session_id("a")
    ));

    // --- session_b: 1 event ---
    let sess_b_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(session_id("b")),
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(sess_b_events.len(), 1);
    assert!(matches!(
        &sess_b_events[0].envelope.payload,
        RuntimeEvent::SessionCreated(e) if e.session_id == session_id("b")
    ));

    // --- run_1: Created + StateChanged = 2 events ---
    let run1_events =
        EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run_id("1")), None, 100)
            .await
            .unwrap();
    assert_eq!(
        run1_events.len(),
        2,
        "run_1 must have Created + StateChanged = 2 events"
    );
    assert!(
        run1_events.iter().all(|e| {
            match &e.envelope.payload {
                RuntimeEvent::RunCreated(r) => r.run_id == run_id("1"),
                RuntimeEvent::RunStateChanged(r) => r.run_id == run_id("1"),
                _ => false,
            }
        }),
        "all run_1 events must reference run_1"
    );

    // --- run_2: only its own Created event ---
    let run2_events =
        EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run_id("2")), None, 100)
            .await
            .unwrap();
    assert_eq!(run2_events.len(), 1, "run_2 must have only 1 event");
    assert!(
        !run2_events.iter().any(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::RunCreated(r) if r.run_id == run_id("1"))
        }),
        "run_2 events must not include run_1's events"
    );

    // --- task_1: Created + StateChanged = 2 events ---
    let task1_events =
        EventLog::read_by_entity(store.as_ref(), &EntityRef::Task(task_id("1")), None, 100)
            .await
            .unwrap();
    assert_eq!(
        task1_events.len(),
        2,
        "task_1 must have Created + StateChanged = 2 events"
    );
}

/// (3) Cursor-based pagination: read first N events, then continue from cursor.
#[tokio::test]
async fn cursor_based_pagination_skips_prior_events() {
    let store = Arc::new(InMemoryStore::new());

    // Append 5 run events for run_page.
    for i in 1..=5u8 {
        if i == 1 {
            store
                .append(&[ev(
                    &format!("evt_run_page_{i}"),
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project(),
                        session_id: session_id("page"),
                        run_id: run_id("page"),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: None,
                    }),
                )])
                .await
                .unwrap();
        } else {
            store
                .append(&[ev(
                    &format!("evt_run_page_{i}"),
                    RuntimeEvent::RunStateChanged(RunStateChanged {
                        project: project(),
                        run_id: run_id("page"),
                        transition: StateTransition {
                            from: Some(RunState::Pending),
                            to: RunState::Running,
                        },
                        failure_class: None,
                        pause_reason: None,
                        resume_trigger: None,
                    }),
                )])
                .await
                .unwrap();
        }
    }

    // Page 1: first 2 events.
    let page1 = EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run_id("page")), None, 2)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2, "page 1 must return exactly 2 events");

    // Cursor: position of the last event on page 1.
    let cursor = page1.last().unwrap().position;

    // Page 2: read after cursor.
    let page2 = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Run(run_id("page")),
        Some(cursor),
        10,
    )
    .await
    .unwrap();
    assert_eq!(
        page2.len(),
        3,
        "page 2 must return the remaining 3 events after cursor"
    );

    // All page 2 events must be strictly after the cursor.
    for event in &page2 {
        assert!(
            event.position > cursor,
            "page 2 event at position {:?} must be after cursor {:?}",
            event.position,
            cursor
        );
    }

    // No overlap between pages.
    let page1_positions: std::collections::HashSet<_> = page1.iter().map(|e| e.position).collect();
    let page2_positions: std::collections::HashSet<_> = page2.iter().map(|e| e.position).collect();
    assert!(
        page1_positions.is_disjoint(&page2_positions),
        "pages must not overlap"
    );
}

/// (4) head_position reflects the position of the most recently appended event.
#[tokio::test]
async fn head_position_reflects_latest_append() {
    let store = Arc::new(InMemoryStore::new());

    // Empty store: head position must be None.
    let head_empty = EventLog::head_position(store.as_ref()).await.unwrap();
    assert!(
        head_empty.is_none(),
        "empty store must report head_position = None"
    );

    // Append first event.
    let pos1 = EventLog::append(
        store.as_ref(),
        &[ev(
            "evt_head_1",
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: session_id("head1"),
            }),
        )],
    )
    .await
    .unwrap()[0];

    let head1 = EventLog::head_position(store.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head1, pos1,
        "head_position must equal position of first appended event"
    );

    // Append two more events.
    let mut positions = EventLog::append(
        store.as_ref(),
        &[
            ev(
                "evt_head_2",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id("head2"),
                }),
            ),
            ev(
                "evt_head_3",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id("head3"),
                }),
            ),
        ],
    )
    .await
    .unwrap();

    let last_pos = positions.pop().unwrap();
    let head3 = EventLog::head_position(store.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head3, last_pos,
        "head_position must reflect the position of the last appended event"
    );
    assert!(head3 > head1, "head must advance with each append");
}

/// (5) read_stream returns ALL events globally in append order.
#[tokio::test]
async fn read_stream_returns_all_events_in_order() {
    let store = Arc::new(InMemoryStore::new());

    // Append a mix of different entity events.
    store
        .append(&[
            ev(
                "evt_s1",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id("s1"),
                }),
            ),
            ev(
                "evt_r1",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id("s1"),
                    run_id: run_id("r1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                "evt_t1",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("t1"),
                    parent_run_id: Some(run_id("r1")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            ev(
                "evt_s2",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id("s2"),
                }),
            ),
        ])
        .await
        .unwrap();

    // read_stream(None) must return all 4 events.
    let all = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(all.len(), 4, "read_stream must return all appended events");

    // Events must be in monotonically increasing position order.
    for window in all.windows(2) {
        assert!(
            window[0].position < window[1].position,
            "events must be in ascending position order: {:?} < {:?}",
            window[0].position,
            window[1].position
        );
    }

    // read_stream after cursor returns only subsequent events.
    let cursor = all[1].position; // after the second event
    let tail = EventLog::read_stream(store.as_ref(), Some(cursor), 100)
        .await
        .unwrap();
    assert_eq!(
        tail.len(),
        2,
        "read_stream after cursor must return only events after it"
    );
    assert!(tail.iter().all(|e| e.position > cursor));

    // read_stream with limit truncates the result set.
    let limited = EventLog::read_stream(store.as_ref(), None, 2)
        .await
        .unwrap();
    assert_eq!(
        limited.len(),
        2,
        "read_stream with limit=2 must return exactly 2 events"
    );
}

/// (6) EntityRef matching for all entity types: Session, Run, Task, Approval,
/// and Checkpoint all resolve correctly from the event log.
#[tokio::test]
async fn entity_ref_matching_all_types() {
    let store = Arc::new(InMemoryStore::new());

    // Append one event for each entity type.
    store
        .append(&[
            // Session
            ev(
                "evt_sess_x",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id("x"),
                }),
            ),
            // Run
            ev(
                "evt_run_x",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id("x"),
                    run_id: run_id("x"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            // Task
            ev(
                "evt_task_x",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("x"),
                    parent_run_id: Some(run_id("x")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            // Approval
            ev(
                "evt_appr_x",
                RuntimeEvent::ApprovalRequested(ApprovalRequested {
                    project: project(),
                    approval_id: approval_id("x"),
                    run_id: Some(run_id("x")),
                    task_id: None,
                    requirement: ApprovalRequirement::Required,
                    title: None,
                    description: None,
                }),
            ),
            // Checkpoint
            ev(
                "evt_ckpt_x",
                RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
                    project: project(),
                    run_id: run_id("x"),
                    checkpoint_id: checkpoint_id("x"),
                    disposition: CheckpointDisposition::Latest,
                    data: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Each EntityRef variant must resolve to exactly its own event.
    let entity_checks: &[(&str, EntityRef)] = &[
        ("session", EntityRef::Session(session_id("x"))),
        ("run", EntityRef::Run(run_id("x"))),
        ("task", EntityRef::Task(task_id("x"))),
        ("approval", EntityRef::Approval(approval_id("x"))),
        ("checkpoint", EntityRef::Checkpoint(checkpoint_id("x"))),
    ];

    for (label, entity_ref) in entity_checks {
        let events = EventLog::read_by_entity(store.as_ref(), entity_ref, None, 100)
            .await
            .unwrap();
        assert_eq!(
            events.len(),
            1,
            "EntityRef::{label} must match exactly 1 event"
        );
    }

    // Each entity must return ONLY its own event — not events from other entities.
    let sess_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(session_id("x")),
        None,
        100,
    )
    .await
    .unwrap();
    assert!(
        !sess_events
            .iter()
            .any(|e| matches!(&e.envelope.payload, RuntimeEvent::RunCreated(_))),
        "Session entity read must not include Run events"
    );

    let run_events =
        EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run_id("x")), None, 100)
            .await
            .unwrap();
    assert!(
        !run_events
            .iter()
            .any(|e| matches!(&e.envelope.payload, RuntimeEvent::TaskCreated(_))),
        "Run entity read must not include Task events"
    );

    // Non-existent entity returns empty.
    let missing = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(session_id("does_not_exist")),
        None,
        100,
    )
    .await
    .unwrap();
    assert!(
        missing.is_empty(),
        "non-existent entity must return empty event list"
    );
}

/// Entity reads for different IDs of the same type are independent.
/// session_a events must not appear in session_b reads.
#[tokio::test]
async fn same_type_different_id_events_are_independent() {
    let store = Arc::new(InMemoryStore::new());

    // Append 2 sessions with 3 events each.
    for n in ["alpha", "beta"] {
        store
            .append(&[ev(
                &format!("evt_{n}_created"),
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id(n),
                }),
            )])
            .await
            .unwrap();
    }

    // run under session_alpha
    for i in 1..=3u8 {
        store
            .append(&[ev(
                &format!("evt_run_alpha_{i}"),
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id("alpha"),
                    run_id: run_id(&format!("alpha_{i}")),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            )])
            .await
            .unwrap();
    }

    // session_alpha read: only 1 SessionCreated event (not the runs).
    let alpha_sess = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(session_id("alpha")),
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(alpha_sess.len(), 1);

    // session_beta read: only 1 SessionCreated event (not alpha's runs).
    let beta_sess = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(session_id("beta")),
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(beta_sess.len(), 1);
    assert!(
        !beta_sess.iter().any(|e|
            matches!(&e.envelope.payload, RuntimeEvent::SessionCreated(s) if s.session_id == session_id("alpha"))
        ),
        "beta session read must not include alpha session events"
    );
}
