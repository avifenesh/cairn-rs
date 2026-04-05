//! Projection rebuild parity tests (RFC 002).
//!
//! Proves that projections are deterministic: replaying the same event sequence
//! into a fresh store produces identical logical state, regardless of when the
//! replay happens.  This is critical for:
//!   - Event replay after restart
//!   - Multi-node state reconstruction
//!   - Correctness audits
//!
//! Design note on timestamps:
//!   `stored_at` is set to `now_millis()` at append-time, so two separate
//!   append calls will produce slightly different `created_at`/`updated_at`
//!   values. These are NOT deterministic across replays and are excluded from
//!   the parity comparison.  What IS deterministic:
//!     - Entity IDs and their presence in the read model
//!     - State values (session/run/task/approval state)
//!     - Version counters (each event application bumps version by 1)
//!     - Parent/child relationships (parent_run_id, session_id)
//!     - Decision values (approval approved/rejected)
//!     - The full ordered event log (position, payload)

use std::sync::Arc;

use cairn_domain::{
    ApprovalId, ApprovalRequested, ApprovalResolved, EventEnvelope, EventId, EventSource,
    FailureClass, ProjectId, ProjectKey, RunCreated, RunId, RunState, RunStateChanged,
    RuntimeEvent, SessionCreated, SessionId, SessionState, SessionStateChanged, StateTransition,
    TaskCreated, TaskId, TaskState, TaskStateChanged, TenantId, WorkspaceId,
};
use cairn_domain::policy::{ApprovalDecision, ApprovalRequirement};
use cairn_store::{
    projections::{ApprovalReadModel, RunReadModel, SessionReadModel, TaskReadModel},
    EventLog, EventPosition, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project_a() -> ProjectKey {
    ProjectKey {
        tenant_id:    TenantId::new("t_rebuild"),
        workspace_id: WorkspaceId::new("w_rebuild"),
        project_id:   ProjectId::new("p_rebuild_a"),
    }
}

fn project_b() -> ProjectKey {
    ProjectKey {
        tenant_id:    TenantId::new("t_rebuild"),
        workspace_id: WorkspaceId::new("w_rebuild"),
        project_id:   ProjectId::new("p_rebuild_b"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn run_transition(
    evt_id: &str,
    run_id: &str,
    from: Option<RunState>,
    to: RunState,
    failure_class: Option<FailureClass>,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::RunStateChanged(RunStateChanged {
        project: project_a(),
        run_id: RunId::new(run_id),
        transition: StateTransition { from, to },
        failure_class,
        pause_reason: None,
        resume_trigger: None,
    }))
}

fn task_transition(
    evt_id: &str,
    task_id: &str,
    from: Option<TaskState>,
    to: TaskState,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::TaskStateChanged(cairn_domain::TaskStateChanged {
        project: project_a(),
        task_id: TaskId::new(task_id),
        transition: StateTransition { from, to },
        failure_class: None,
        pause_reason: None,
        resume_trigger: None,
    }))
}

/// The canonical 20-event sequence used for both stores.
fn build_20_events() -> Vec<EventEnvelope<RuntimeEvent>> {
    vec![
        // ── sessions (3) ──────────────────────────────────────────────
        // 1
        evt("e01", RuntimeEvent::SessionCreated(SessionCreated {
            project: project_a(), session_id: SessionId::new("sess_1"),
        })),
        // 2
        evt("e02", RuntimeEvent::SessionCreated(SessionCreated {
            project: project_a(), session_id: SessionId::new("sess_2"),
        })),
        // 3 — different project
        evt("e03", RuntimeEvent::SessionCreated(SessionCreated {
            project: project_b(), session_id: SessionId::new("sess_b1"),
        })),

        // ── runs (4, including one child run) ─────────────────────────
        // 4
        evt("e04", RuntimeEvent::RunCreated(RunCreated {
            project: project_a(), session_id: SessionId::new("sess_1"),
            run_id: RunId::new("run_1"), parent_run_id: None,
            prompt_release_id: None, agent_role_id: Some("orchestrator".to_owned()),
        })),
        // 5
        evt("e05", RuntimeEvent::RunCreated(RunCreated {
            project: project_a(), session_id: SessionId::new("sess_1"),
            run_id: RunId::new("run_2"), parent_run_id: None,
            prompt_release_id: None, agent_role_id: None,
        })),
        // 6
        evt("e06", RuntimeEvent::RunCreated(RunCreated {
            project: project_a(), session_id: SessionId::new("sess_2"),
            run_id: RunId::new("run_3"), parent_run_id: None,
            prompt_release_id: None, agent_role_id: None,
        })),
        // 7 — child/subagent run
        evt("e07", RuntimeEvent::RunCreated(RunCreated {
            project: project_a(), session_id: SessionId::new("sess_1"),
            run_id: RunId::new("run_4_child"),
            parent_run_id: Some(RunId::new("run_1")),
            prompt_release_id: None, agent_role_id: Some("researcher".to_owned()),
        })),

        // ── tasks (3) ────────────────────────────────────────────────
        // 8
        evt("e08", RuntimeEvent::TaskCreated(TaskCreated {
            project: project_a(), task_id: TaskId::new("task_1"),
            parent_run_id: Some(RunId::new("run_1")),
            parent_task_id: None, prompt_release_id: None,
        })),
        // 9
        evt("e09", RuntimeEvent::TaskCreated(TaskCreated {
            project: project_a(), task_id: TaskId::new("task_2"),
            parent_run_id: Some(RunId::new("run_2")),
            parent_task_id: None, prompt_release_id: None,
        })),
        // 10
        evt("e10", RuntimeEvent::TaskCreated(TaskCreated {
            project: project_a(), task_id: TaskId::new("task_3"),
            parent_run_id: Some(RunId::new("run_2")),
            parent_task_id: Some(TaskId::new("task_2")),
            prompt_release_id: None,
        })),

        // ── approvals (2) ────────────────────────────────────────────
        // 11
        evt("e11", RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project_a(), approval_id: ApprovalId::new("appr_1"),
            run_id: Some(RunId::new("run_1")), task_id: None,
            requirement: ApprovalRequirement::Required,
        })),
        // 12
        evt("e12", RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project_a(), approval_id: ApprovalId::new("appr_2"),
            run_id: Some(RunId::new("run_2")), task_id: None,
            requirement: ApprovalRequirement::Required,
        })),

        // ── run state transitions (4) ────────────────────────────────
        // 13 run_1: Pending→Running
        run_transition("e13", "run_1", Some(RunState::Pending), RunState::Running, None),
        // 14 run_2: Pending→Running
        run_transition("e14", "run_2", Some(RunState::Pending), RunState::Running, None),
        // 15 run_1: Running→WaitingApproval
        run_transition("e15", "run_1", Some(RunState::Running), RunState::WaitingApproval, None),

        // ── task state transition (1) ────────────────────────────────
        // 16 task_1: Queued→Running
        task_transition("e16", "task_1", Some(TaskState::Queued), TaskState::Running),

        // ── approval resolutions (2) ─────────────────────────────────
        // 17 appr_1: approved
        evt("e17", RuntimeEvent::ApprovalResolved(ApprovalResolved {
            project: project_a(), approval_id: ApprovalId::new("appr_1"),
            decision: ApprovalDecision::Approved,
        })),
        // 18 appr_2: rejected
        evt("e18", RuntimeEvent::ApprovalResolved(ApprovalResolved {
            project: project_a(), approval_id: ApprovalId::new("appr_2"),
            decision: ApprovalDecision::Rejected,
        })),

        // ── final run transitions (2) ────────────────────────────────
        // 19 run_2: Running→Failed (approval rejected)
        run_transition("e19", "run_2", Some(RunState::Running), RunState::Failed,
            Some(FailureClass::ApprovalRejected)),
        // 20 session_1: state change (Open→Completed inferred)
        evt("e20", RuntimeEvent::SessionStateChanged(SessionStateChanged {
            project: project_a(), session_id: SessionId::new("sess_1"),
            transition: StateTransition {
                from: Some(SessionState::Open),
                to:   SessionState::Completed,
            },
        })),
    ]
}

// ── snapshot helpers ──────────────────────────────────────────────────────────

/// Deterministic state fingerprint — excludes timestamps and uses sorted order.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SessionSnap {
    session_id: String,
    state:      String,
    version:    u64,
    project_id: String,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RunSnap {
    run_id:        String,
    session_id:    String,
    state:         String,
    version:       u64,
    parent_run_id: Option<String>,
    failure_class: Option<String>,
    agent_role_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TaskSnap {
    task_id:       String,
    state:         String,
    version:       u64,
    parent_run_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ApprovalSnap {
    approval_id:  String,
    requirement:  String,
    decision:     Option<String>,
    version:      u64,
}

async fn snapshot_sessions(store: &InMemoryStore) -> Vec<SessionSnap> {
    let mut snaps: Vec<SessionSnap> = vec![];
    for sess_id in ["sess_1", "sess_2", "sess_b1"] {
        if let Some(r) = SessionReadModel::get(store, &SessionId::new(sess_id))
            .await.unwrap()
        {
            snaps.push(SessionSnap {
                session_id: r.session_id.as_str().to_owned(),
                state:      format!("{:?}", r.state),
                version:    r.version,
                project_id: r.project.project_id.as_str().to_owned(),
            });
        }
    }
    snaps.sort();
    snaps
}

async fn snapshot_runs(store: &InMemoryStore) -> Vec<RunSnap> {
    let mut snaps: Vec<RunSnap> = vec![];
    for run_id in ["run_1", "run_2", "run_3", "run_4_child"] {
        if let Some(r) = RunReadModel::get(store, &RunId::new(run_id))
            .await.unwrap()
        {
            snaps.push(RunSnap {
                run_id:        r.run_id.as_str().to_owned(),
                session_id:    r.session_id.as_str().to_owned(),
                state:         format!("{:?}", r.state),
                version:       r.version,
                parent_run_id: r.parent_run_id.map(|id| id.as_str().to_owned()),
                failure_class: r.failure_class.map(|f| format!("{f:?}")),
                agent_role_id: r.agent_role_id,
            });
        }
    }
    snaps.sort();
    snaps
}

async fn snapshot_tasks(store: &InMemoryStore) -> Vec<TaskSnap> {
    let mut snaps: Vec<TaskSnap> = vec![];
    for task_id in ["task_1", "task_2", "task_3"] {
        if let Some(r) = TaskReadModel::get(store, &TaskId::new(task_id))
            .await.unwrap()
        {
            snaps.push(TaskSnap {
                task_id:       r.task_id.as_str().to_owned(),
                state:         format!("{:?}", r.state),
                version:       r.version,
                parent_run_id: r.parent_run_id.map(|id| id.as_str().to_owned()),
            });
        }
    }
    snaps.sort();
    snaps
}

async fn snapshot_approvals(store: &InMemoryStore) -> Vec<ApprovalSnap> {
    let mut snaps: Vec<ApprovalSnap> = vec![];
    for appr_id in ["appr_1", "appr_2"] {
        if let Some(r) = ApprovalReadModel::get(store, &ApprovalId::new(appr_id))
            .await.unwrap()
        {
            snaps.push(ApprovalSnap {
                approval_id:  r.approval_id.as_str().to_owned(),
                requirement:  format!("{:?}", r.requirement),
                decision:     r.decision.map(|d| format!("{d:?}")),
                version:      r.version,
            });
        }
    }
    snaps.sort();
    snaps
}

// ── 1. Main parity test: store1 == store2 after replay ───────────────────────

#[tokio::test]
async fn projection_rebuild_parity_after_replay() {
    let events = build_20_events();
    assert_eq!(events.len(), 20, "test fixture must have exactly 20 events");

    // ── Store 1: original ─────────────────────────────────────────────────
    let store1 = InMemoryStore::new();
    store1.append(&events).await.unwrap();

    let sessions1  = snapshot_sessions(&store1).await;
    let runs1      = snapshot_runs(&store1).await;
    let tasks1     = snapshot_tasks(&store1).await;
    let approvals1 = snapshot_approvals(&store1).await;
    let head1      = store1.head_position().await.unwrap();

    // ── Store 2: fresh replay ─────────────────────────────────────────────
    let store2 = InMemoryStore::new();
    store2.append(&events).await.unwrap();

    let sessions2  = snapshot_sessions(&store2).await;
    let runs2      = snapshot_runs(&store2).await;
    let tasks2     = snapshot_tasks(&store2).await;
    let approvals2 = snapshot_approvals(&store2).await;
    let head2      = store2.head_position().await.unwrap();

    // ── Parity assertions ─────────────────────────────────────────────────
    assert_eq!(sessions1,  sessions2,  "SESSION projections must be identical after replay");
    assert_eq!(runs1,      runs2,      "RUN projections must be identical after replay");
    assert_eq!(tasks1,     tasks2,     "TASK projections must be identical after replay");
    assert_eq!(approvals1, approvals2, "APPROVAL projections must be identical after replay");

    // Event log length and head position are also deterministic.
    assert_eq!(
        head1.map(|p| p.0),
        head2.map(|p| p.0),
        "head position must be identical"
    );
    assert_eq!(head1.unwrap().0, 20, "20 events → head at position 20");
}

// ── 2. Event log order is preserved ──────────────────────────────────────────

#[tokio::test]
async fn event_log_order_is_preserved_on_replay() {
    let events = build_20_events();

    let store1 = InMemoryStore::new();
    let store2 = InMemoryStore::new();
    store1.append(&events).await.unwrap();
    store2.append(&events).await.unwrap();

    let log1 = store1.read_stream(None, 100).await.unwrap();
    let log2 = store2.read_stream(None, 100).await.unwrap();

    assert_eq!(log1.len(), log2.len(), "event count must match");
    assert_eq!(log1.len(), 20);

    for (i, (e1, e2)) in log1.iter().zip(log2.iter()).enumerate() {
        assert_eq!(e1.position, e2.position,
            "event {i}: positions must match");
        assert_eq!(e1.envelope.event_id, e2.envelope.event_id,
            "event {i}: event_ids must match");
        // Payload equality (excludes stored_at which is timing-dependent).
        assert_eq!(e1.envelope.payload, e2.envelope.payload,
            "event {i}: payloads must match");
    }
}

// ── 3. Specific state assertions after replay ─────────────────────────────────

#[tokio::test]
async fn session_state_correct_after_replay() {
    let store = InMemoryStore::new();
    store.append(&build_20_events()).await.unwrap();

    // sess_1 was transitioned to Completed (event 20).
    let sess1 = SessionReadModel::get(&store, &SessionId::new("sess_1"))
        .await.unwrap().unwrap();
    assert_eq!(sess1.state, SessionState::Completed,
        "sess_1 must be Completed after transition event");
    assert_eq!(sess1.version, 2, "one create + one transition = version 2");

    // sess_2 was never transitioned.
    let sess2 = SessionReadModel::get(&store, &SessionId::new("sess_2"))
        .await.unwrap().unwrap();
    assert_eq!(sess2.state, SessionState::Open);
    assert_eq!(sess2.version, 1);

    // sess_b1 is in project_b — exists and is Open.
    let sessb1 = SessionReadModel::get(&store, &SessionId::new("sess_b1"))
        .await.unwrap().unwrap();
    assert_eq!(sessb1.state, SessionState::Open);
    assert_eq!(sessb1.project, project_b());
}

#[tokio::test]
async fn run_state_correct_after_replay() {
    let store = InMemoryStore::new();
    store.append(&build_20_events()).await.unwrap();

    // run_1: Pending(1) → Running(2) → WaitingApproval(3)
    let r1 = RunReadModel::get(&store, &RunId::new("run_1")).await.unwrap().unwrap();
    assert_eq!(r1.state, RunState::WaitingApproval);
    assert_eq!(r1.version, 3);
    assert_eq!(r1.agent_role_id.as_deref(), Some("orchestrator"));
    assert!(r1.parent_run_id.is_none());

    // run_2: Pending(1) → Running(2) → Failed(3, ApprovalRejected)
    let r2 = RunReadModel::get(&store, &RunId::new("run_2")).await.unwrap().unwrap();
    assert_eq!(r2.state, RunState::Failed);
    assert_eq!(r2.version, 3);
    assert_eq!(r2.failure_class, Some(FailureClass::ApprovalRejected));

    // run_3: Pending(1), never transitioned
    let r3 = RunReadModel::get(&store, &RunId::new("run_3")).await.unwrap().unwrap();
    assert_eq!(r3.state, RunState::Pending);
    assert_eq!(r3.version, 1);
    assert_eq!(r3.session_id.as_str(), "sess_2");

    // run_4_child: child of run_1
    let r4 = RunReadModel::get(&store, &RunId::new("run_4_child")).await.unwrap().unwrap();
    assert_eq!(r4.parent_run_id, Some(RunId::new("run_1")));
    assert_eq!(r4.agent_role_id.as_deref(), Some("researcher"));
    assert_eq!(r4.state, RunState::Pending);
}

#[tokio::test]
async fn task_state_correct_after_replay() {
    let store = InMemoryStore::new();
    store.append(&build_20_events()).await.unwrap();

    // task_1: Queued(1) → Running(2)
    let t1 = TaskReadModel::get(&store, &TaskId::new("task_1")).await.unwrap().unwrap();
    assert_eq!(t1.state, TaskState::Running);
    assert_eq!(t1.version, 2);
    assert_eq!(t1.parent_run_id, Some(RunId::new("run_1")));

    // task_2: Queued, never transitioned
    let t2 = TaskReadModel::get(&store, &TaskId::new("task_2")).await.unwrap().unwrap();
    assert_eq!(t2.state, TaskState::Queued);
    assert_eq!(t2.version, 1);

    // task_3: Queued, child of task_2
    let t3 = TaskReadModel::get(&store, &TaskId::new("task_3")).await.unwrap().unwrap();
    assert_eq!(t3.state, TaskState::Queued);
    assert_eq!(t3.parent_task_id, Some(TaskId::new("task_2")));
}

#[tokio::test]
async fn approval_state_correct_after_replay() {
    let store = InMemoryStore::new();
    store.append(&build_20_events()).await.unwrap();

    // appr_1: Requested(1) → Resolved Approved(2)
    let a1 = ApprovalReadModel::get(&store, &ApprovalId::new("appr_1")).await.unwrap().unwrap();
    assert_eq!(a1.decision, Some(ApprovalDecision::Approved));
    assert_eq!(a1.version, 2);
    assert_eq!(a1.requirement, ApprovalRequirement::Required);

    // appr_2: Requested(1) → Resolved Rejected(2)
    let a2 = ApprovalReadModel::get(&store, &ApprovalId::new("appr_2")).await.unwrap().unwrap();
    assert_eq!(a2.decision, Some(ApprovalDecision::Rejected));
    assert_eq!(a2.version, 2);

    // Both approvals resolved — pending list is empty.
    let pending = ApprovalReadModel::list_pending(&store, &project_a(), 10, 0)
        .await.unwrap();
    assert!(pending.is_empty(), "both approvals resolved — pending must be empty");
}

// ── 4. Cross-project isolation is preserved on replay ─────────────────────────

#[tokio::test]
async fn cross_project_isolation_preserved_on_replay() {
    let store = InMemoryStore::new();
    store.append(&build_20_events()).await.unwrap();

    // project_a has sessions sess_1 and sess_2.
    let proj_a_sessions = SessionReadModel::list_by_project(
        &store, &project_a(), 10, 0,
    ).await.unwrap();
    assert_eq!(proj_a_sessions.len(), 2);
    let ids: Vec<_> = proj_a_sessions.iter().map(|s| s.session_id.as_str()).collect();
    assert!(ids.contains(&"sess_1"));
    assert!(ids.contains(&"sess_2"));
    assert!(!ids.contains(&"sess_b1"), "sess_b1 belongs to project_b");

    // project_b has only sess_b1.
    let proj_b_sessions = SessionReadModel::list_by_project(
        &store, &project_b(), 10, 0,
    ).await.unwrap();
    assert_eq!(proj_b_sessions.len(), 1);
    assert_eq!(proj_b_sessions[0].session_id.as_str(), "sess_b1");
}

// ── 5. Idempotency: appending same events a third time triples the log ─────────

#[tokio::test]
async fn replay_does_not_corrupt_existing_state() {
    // Prove that a second replay into an ALREADY-populated store correctly
    // re-applies projection state (last write wins on entity_id).
    let events = build_20_events();
    let store = InMemoryStore::new();

    store.append(&events).await.unwrap();
    let snap_after_first = snapshot_runs(&store).await;

    // Second replay into the same store: each entity gets re-inserted with
    // the same logical state, so the snapshot is unchanged.
    store.append(&events).await.unwrap();
    let snap_after_second = snapshot_runs(&store).await;

    // State values are identical (last-write-wins upsert semantics).
    assert_eq!(snap_after_first, snap_after_second,
        "re-applying same events must not change logical state");

    // But the event log grows (events are appended, not deduplicated).
    let head = store.head_position().await.unwrap().unwrap();
    assert_eq!(head.0, 40, "two batches of 20 = 40 events in log");
}
