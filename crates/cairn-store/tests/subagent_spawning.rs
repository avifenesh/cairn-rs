//! RFC 002 subagent spawning integration tests.
//!
//! Validates the subagent hierarchy pipeline through InMemoryStore:
//! - SubagentSpawned event is recorded in the event log.
//! - Child run is created with parent_run_id pointing to the parent.
//! - list_by_session returns both parent and child runs.
//! - Child run state transitions are independent of the parent.
//! - The parent→child hierarchy is queryable via parent_run_id on RunRecord.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, StateTransition, SubagentSpawned, TaskCreated, TaskId,
};
use cairn_domain::events::RunStateChanged;
use cairn_domain::lifecycle::RunState;
use cairn_store::{
    projections::RunReadModel,
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_sub", "ws_sub", "proj_sub")
}

fn parent_session() -> SessionId { SessionId::new("sess_parent") }
fn parent_run()     -> RunId     { RunId::new("run_parent") }
fn child_run()      -> RunId     { RunId::new("run_child") }
fn child_task()     -> TaskId    { TaskId::new("task_child") }

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

/// Seed: session + parent run.
async fn seed_parent(store: &Arc<InMemoryStore>) {
    store.append(&[
        ev("evt_sess", RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: parent_session(),
        })),
        ev("evt_parent_run", RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: parent_session(),
            run_id: parent_run(),
            parent_run_id: None,       // root run
            prompt_release_id: None,
            agent_role_id: None,
        })),
    ]).await.unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): Session + parent run seeded; SubagentSpawned event appended;
/// child run created with parent_run_id linking back to the parent.
#[tokio::test]
async fn subagent_spawned_links_child_to_parent() {
    let store = Arc::new(InMemoryStore::new());
    seed_parent(&store).await;

    // (2) Append SubagentSpawned — records the spawn in the event log.
    store.append(&[ev("evt_spawned", RuntimeEvent::SubagentSpawned(SubagentSpawned {
        project: project(),
        parent_run_id: parent_run(),
        parent_task_id: None,
        child_task_id: child_task(),
        child_session_id: parent_session(), // same session for this test
        child_run_id: Some(child_run()),
    }))]).await.unwrap();

    // The SubagentSpawned event must be in the log.
    let events = EventLog::read_stream(store.as_ref(), None, 100).await.unwrap();
    let has_spawn = events.iter().any(|e| {
        matches!(
            &e.envelope.payload,
            RuntimeEvent::SubagentSpawned(s)
                if s.parent_run_id == parent_run()
                && s.child_run_id == Some(child_run())
        )
    });
    assert!(has_spawn, "SubagentSpawned event must be in the event log");

    // (2b) Create the child run with parent_run_id set (RFC 002: the child run
    // record carries the link back to its parent).
    store.append(&[ev("evt_child_run", RuntimeEvent::RunCreated(RunCreated {
        project: project(),
        session_id: parent_session(),
        run_id: child_run(),
        parent_run_id: Some(parent_run()), // ← hierarchy link
        prompt_release_id: None,
        agent_role_id: None,
    }))]).await.unwrap();

    // (3) Verify child run has parent_run_id set correctly.
    let child = RunReadModel::get(store.as_ref(), &child_run())
        .await.unwrap()
        .expect("child run must exist after RunCreated");

    assert_eq!(
        child.parent_run_id,
        Some(parent_run()),
        "child run must have parent_run_id pointing to the parent"
    );
    assert_eq!(child.session_id, parent_session());
    assert_eq!(child.state, RunState::Pending, "child run starts Pending");
}

/// (4): list_by_session returns both parent and child runs when they share a session.
#[tokio::test]
async fn list_by_session_returns_parent_and_child_runs() {
    let store = Arc::new(InMemoryStore::new());
    seed_parent(&store).await;

    // Child run in the same session.
    store.append(&[
        ev("evt_spawn", RuntimeEvent::SubagentSpawned(SubagentSpawned {
            project: project(),
            parent_run_id: parent_run(),
            parent_task_id: None,
            child_task_id: child_task(),
            child_session_id: parent_session(),
            child_run_id: Some(child_run()),
        })),
        ev("evt_child", RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: parent_session(),
            run_id: child_run(),
            parent_run_id: Some(parent_run()),
            prompt_release_id: None,
            agent_role_id: None,
        })),
    ]).await.unwrap();

    let runs = RunReadModel::list_by_session(store.as_ref(), &parent_session(), 100, 0)
        .await.unwrap();

    assert_eq!(runs.len(), 2, "session must contain both parent and child run");

    let run_ids: Vec<&RunId> = runs.iter().map(|r| &r.run_id).collect();
    assert!(run_ids.contains(&&parent_run()), "parent run must be in the session listing");
    assert!(run_ids.contains(&&child_run()),  "child run must be in the session listing");

    // Parent is the root run (no parent_run_id); child has a parent.
    let parent_rec = runs.iter().find(|r| r.run_id == parent_run()).unwrap();
    let child_rec  = runs.iter().find(|r| r.run_id == child_run()).unwrap();
    assert!(parent_rec.parent_run_id.is_none(), "parent must have no parent_run_id");
    assert_eq!(child_rec.parent_run_id, Some(parent_run()));
}

/// (5): Child run state transitions independently of the parent.
///
/// Parent can complete while the child is still running, and vice versa.
/// Each run follows its own state machine.
#[tokio::test]
async fn child_run_state_is_independent_of_parent() {
    let store = Arc::new(InMemoryStore::new());
    seed_parent(&store).await;

    // Spawn and create the child.
    store.append(&[
        ev("evt_spawn_ind", RuntimeEvent::SubagentSpawned(SubagentSpawned {
            project: project(),
            parent_run_id: parent_run(),
            parent_task_id: None,
            child_task_id: child_task(),
            child_session_id: parent_session(),
            child_run_id: Some(child_run()),
        })),
        ev("evt_child_ind", RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: parent_session(),
            run_id: child_run(),
            parent_run_id: Some(parent_run()),
            prompt_release_id: None,
            agent_role_id: None,
        })),
    ]).await.unwrap();

    // Transition parent → Running → Completed.
    store.append(&[
        ev("evt_parent_run", RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project(),
            run_id: parent_run(),
            transition: StateTransition { from: Some(RunState::Pending), to: RunState::Running },
            failure_class: None, pause_reason: None, resume_trigger: None,
        })),
        ev("evt_parent_done", RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project(),
            run_id: parent_run(),
            transition: StateTransition { from: Some(RunState::Running), to: RunState::Completed },
            failure_class: None, pause_reason: None, resume_trigger: None,
        })),
    ]).await.unwrap();

    // Transition child → Running only (still active).
    store.append(&[ev("evt_child_run", RuntimeEvent::RunStateChanged(RunStateChanged {
        project: project(),
        run_id: child_run(),
        transition: StateTransition { from: Some(RunState::Pending), to: RunState::Running },
        failure_class: None, pause_reason: None, resume_trigger: None,
    }))]).await.unwrap();

    let parent_rec = RunReadModel::get(store.as_ref(), &parent_run()).await.unwrap().unwrap();
    let child_rec  = RunReadModel::get(store.as_ref(), &child_run()).await.unwrap().unwrap();

    // Parent completed; child still running — fully independent.
    assert_eq!(
        parent_rec.state, RunState::Completed,
        "parent must be Completed"
    );
    assert_eq!(
        child_rec.state, RunState::Running,
        "child must be Running independently of the parent's terminal state"
    );
    assert!(
        parent_rec.state.is_terminal(),
        "parent state must be terminal"
    );
    assert!(
        !child_rec.state.is_terminal(),
        "child state must NOT be terminal — it is still running"
    );
}

/// (6): The subagent tree (parent→child hierarchy) is queryable via parent_run_id.
///
/// Proves a two-level tree: parent → child → grandchild.
/// Each level carries a `parent_run_id` pointing to its parent.
#[tokio::test]
async fn subagent_tree_hierarchy_is_queryable() {
    let store = Arc::new(InMemoryStore::new());
    let grandchild = RunId::new("run_grandchild");
    let grandchild_task = TaskId::new("task_grandchild");

    seed_parent(&store).await;

    // Level 1: spawn child from parent.
    store.append(&[
        ev("evt_spawn_l1", RuntimeEvent::SubagentSpawned(SubagentSpawned {
            project: project(),
            parent_run_id: parent_run(),
            parent_task_id: None,
            child_task_id: child_task(),
            child_session_id: parent_session(),
            child_run_id: Some(child_run()),
        })),
        ev("evt_child_l1", RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: parent_session(),
            run_id: child_run(),
            parent_run_id: Some(parent_run()),  // level 1 → parent
            prompt_release_id: None,
            agent_role_id: None,
        })),
    ]).await.unwrap();

    // Level 2: spawn grandchild from child.
    store.append(&[
        ev("evt_spawn_l2", RuntimeEvent::SubagentSpawned(SubagentSpawned {
            project: project(),
            parent_run_id: child_run(),          // child is the new parent
            parent_task_id: None,
            child_task_id: grandchild_task.clone(),
            child_session_id: parent_session(),
            child_run_id: Some(grandchild.clone()),
        })),
        ev("evt_grandchild", RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: parent_session(),
            run_id: grandchild.clone(),
            parent_run_id: Some(child_run()),    // level 2 → child
            prompt_release_id: None,
            agent_role_id: None,
        })),
    ]).await.unwrap();

    // All three runs are in the session.
    let all_runs = RunReadModel::list_by_session(store.as_ref(), &parent_session(), 100, 0)
        .await.unwrap();
    assert_eq!(all_runs.len(), 3, "session must contain parent + child + grandchild");

    // Traverse the hierarchy by following parent_run_id links.
    let parent_rec      = RunReadModel::get(store.as_ref(), &parent_run()).await.unwrap().unwrap();
    let child_rec       = RunReadModel::get(store.as_ref(), &child_run()).await.unwrap().unwrap();
    let grandchild_rec  = RunReadModel::get(store.as_ref(), &grandchild).await.unwrap().unwrap();

    // Root: no parent.
    assert!(parent_rec.parent_run_id.is_none(),
        "root run must have no parent_run_id");

    // Level 1: child points to parent.
    assert_eq!(child_rec.parent_run_id, Some(parent_run()),
        "child must point to parent via parent_run_id");

    // Level 2: grandchild points to child.
    assert_eq!(grandchild_rec.parent_run_id, Some(child_run()),
        "grandchild must point to child via parent_run_id");

    // Walk the tree upward from grandchild → child → parent (root).
    let mut cursor = grandchild_rec.parent_run_id.clone();
    let mut depth = 0usize;
    while let Some(pid) = cursor {
        let rec = RunReadModel::get(store.as_ref(), &pid).await.unwrap().unwrap();
        cursor = rec.parent_run_id.clone();
        depth += 1;
    }
    assert_eq!(depth, 2,
        "walking up from grandchild must traverse exactly 2 hops to reach the root");
}

/// Multiple subagents spawned from the same parent are all independently tracked.
#[tokio::test]
async fn multiple_subagents_from_same_parent() {
    let store = Arc::new(InMemoryStore::new());
    seed_parent(&store).await;

    let child_a = RunId::new("run_child_a");
    let child_b = RunId::new("run_child_b");
    let child_c = RunId::new("run_child_c");

    // Spawn 3 children from the same parent.
    for (child, task_n) in [
        (child_a.clone(), "task_ca"),
        (child_b.clone(), "task_cb"),
        (child_c.clone(), "task_cc"),
    ] {
        store.append(&[
            ev(&format!("spawn_{}", child.as_str()), RuntimeEvent::SubagentSpawned(SubagentSpawned {
                project: project(),
                parent_run_id: parent_run(),
                parent_task_id: None,
                child_task_id: TaskId::new(task_n),
                child_session_id: parent_session(),
                child_run_id: Some(child.clone()),
            })),
            ev(&format!("create_{}", child.as_str()), RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: parent_session(),
                run_id: child.clone(),
                parent_run_id: Some(parent_run()),
                prompt_release_id: None,
                agent_role_id: None,
            })),
        ]).await.unwrap();
    }

    // All 4 runs (1 parent + 3 children) must be listed.
    let all = RunReadModel::list_by_session(store.as_ref(), &parent_session(), 100, 0)
        .await.unwrap();
    assert_eq!(all.len(), 4, "session must contain parent + 3 children");

    // All children point to the same parent.
    let children: Vec<_> = all.iter().filter(|r| r.parent_run_id.is_some()).collect();
    assert_eq!(children.len(), 3, "exactly 3 child runs");
    assert!(children.iter().all(|c| c.parent_run_id == Some(parent_run())),
        "all children must share the same parent_run_id");

    // Each child has a distinct run_id.
    let child_ids: Vec<&RunId> = children.iter().map(|c| &c.run_id).collect();
    assert!(child_ids.contains(&&child_a));
    assert!(child_ids.contains(&&child_b));
    assert!(child_ids.contains(&&child_c));
}
