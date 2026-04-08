//! RFC 002 task dependency integration tests.
//!
//! Validates the task dependency pipeline through InMemoryStore:
//! - Session, run, and task creation via the event log.
//! - Dependency registration via TaskDependencyReadModel::insert_dependency.
//! - list_blocking returns correct per-task dependency set (the DAG edges).
//! - list_unresolved returns only unresolved dependencies.
//! - resolve_dependency marks all dependents of a completed task as unblocked.
//! - Inline cycle detection: a DAG with no cycles passes; one with a cycle is flagged.

use std::sync::Arc;

use cairn_domain::lifecycle::TaskState;
use cairn_domain::task_dependencies::TaskDependency;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, StateTransition, TaskCreated, TaskDependencyAdded, TaskId,
    TaskStateChanged,
};
use cairn_store::projections::TaskDependencyRecord;
use cairn_store::{
    projections::{TaskDependencyReadModel, TaskReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_dep", "ws_dep", "proj_dep")
}

fn run_id() -> RunId {
    RunId::new("run_dep_1")
}

fn session_id() -> SessionId {
    SessionId::new("sess_dep_1")
}

fn task_id(n: &str) -> TaskId {
    TaskId::new(format!("task_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn dep_record(dependent: &str, prerequisite: &str) -> TaskDependencyRecord {
    TaskDependencyRecord {
        dependency: TaskDependency {
            dependent_task_id: task_id(dependent),
            depends_on_task_id: task_id(prerequisite),
            project: project(),
            created_at_ms: 1_000,
        },
        resolved_at_ms: None,
    }
}

/// Seed the store with session + run + 3 tasks (a, b, c).
async fn seed_tasks(store: &Arc<InMemoryStore>) {
    store
        .append(&[
            ev(
                "evt_sess",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id(),
                }),
            ),
            ev(
                "evt_run",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id(),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                "evt_task_a",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("a"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            ev(
                "evt_task_b",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("b"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            ev(
                "evt_task_c",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: task_id("c"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
        ])
        .await
        .unwrap();
}

// ── cycle detection helper ────────────────────────────────────────────────────

/// Returns `true` if adding `(dependent → prerequisite)` would create a cycle
/// in the existing dependency graph (DFS reachability from prerequisite back to
/// dependent through existing deps).
///
/// This is the correctness property that RFC 002 requires implementations to
/// enforce before accepting a `TaskDependencyAdded` command.
fn would_create_cycle(
    deps: &[TaskDependencyRecord],
    new_dependent: &TaskId,
    new_prerequisite: &TaskId,
) -> bool {
    // Build adjacency: node → nodes it depends_on.
    let mut adj: std::collections::HashMap<&TaskId, Vec<&TaskId>> =
        std::collections::HashMap::new();
    for r in deps {
        adj.entry(&r.dependency.dependent_task_id)
            .or_default()
            .push(&r.dependency.depends_on_task_id);
    }
    // Add the proposed edge.
    adj.entry(new_dependent).or_default().push(new_prerequisite);

    // DFS: can we reach `new_dependent` starting from `new_prerequisite`?
    // If yes, adding this edge creates a cycle.
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![new_prerequisite];
    while let Some(node) = stack.pop() {
        if node == new_dependent {
            return true; // cycle found
        }
        if visited.insert(node) {
            if let Some(neighbours) = adj.get(node) {
                stack.extend(neighbours.iter());
            }
        }
    }
    false
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): session, run, and 3 tasks are seeded correctly.
#[tokio::test]
async fn session_run_tasks_seeded() {
    let store = Arc::new(InMemoryStore::new());
    seed_tasks(&store).await;

    // All three tasks must exist in the read model.
    for name in ["a", "b", "c"] {
        let t = TaskReadModel::get(store.as_ref(), &task_id(name))
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("task_{name} must exist after TaskCreated event"));
        assert_eq!(t.task_id, task_id(name));
    }
}

/// (3) + (4): add chain a→b→c; list_blocking returns the correct DAG edges.
///
/// b depends on a, c depends on b.
#[tokio::test]
async fn list_blocking_returns_correct_dag_edges() {
    let store = Arc::new(InMemoryStore::new());
    seed_tasks(&store).await;

    // b depends_on a (a must complete before b can run).
    TaskDependencyReadModel::insert_dependency(store.as_ref(), dep_record("b", "a"))
        .await
        .unwrap();

    // c depends_on b.
    TaskDependencyReadModel::insert_dependency(store.as_ref(), dep_record("c", "b"))
        .await
        .unwrap();

    // list_blocking(task_b) → should return the b→a dependency.
    let b_blocked_by = TaskDependencyReadModel::list_blocking(store.as_ref(), &task_id("b"))
        .await
        .unwrap();
    assert_eq!(
        b_blocked_by.len(),
        1,
        "task_b must have exactly one blocker"
    );
    assert_eq!(
        b_blocked_by[0].dependency.depends_on_task_id,
        task_id("a"),
        "task_b must be blocked by task_a"
    );
    assert_eq!(
        b_blocked_by[0].resolved_at_ms, None,
        "dependency must not yet be resolved"
    );

    // list_blocking(task_c) → should return the c→b dependency.
    let c_blocked_by = TaskDependencyReadModel::list_blocking(store.as_ref(), &task_id("c"))
        .await
        .unwrap();
    assert_eq!(
        c_blocked_by.len(),
        1,
        "task_c must have exactly one blocker"
    );
    assert_eq!(c_blocked_by[0].dependency.depends_on_task_id, task_id("b"));

    // list_blocking(task_a) → no dependencies (task_a is the root).
    let a_blocked_by = TaskDependencyReadModel::list_blocking(store.as_ref(), &task_id("a"))
        .await
        .unwrap();
    assert!(a_blocked_by.is_empty(), "task_a has no prerequisites");

    // (5) list_unresolved returns all 2 pending deps.
    let unresolved = TaskDependencyReadModel::list_unresolved(store.as_ref(), &project())
        .await
        .unwrap();
    assert_eq!(
        unresolved.len(),
        2,
        "both dependencies must be unresolved initially"
    );
}

/// (6) After task_a completes, task_b's dependency is resolved (unblocked).
///
/// Appends TaskStateChanged(a → Completed), then calls resolve_dependency
/// and verifies task_b is no longer blocked.
#[tokio::test]
async fn resolve_dependency_unblocks_task_b_when_task_a_completes() {
    let store = Arc::new(InMemoryStore::new());
    seed_tasks(&store).await;

    // Set up: b depends_on a, c depends_on b.
    TaskDependencyReadModel::insert_dependency(store.as_ref(), dep_record("b", "a"))
        .await
        .unwrap();
    TaskDependencyReadModel::insert_dependency(store.as_ref(), dep_record("c", "b"))
        .await
        .unwrap();

    // Verify both unresolved before completion.
    let before = TaskDependencyReadModel::list_unresolved(store.as_ref(), &project())
        .await
        .unwrap();
    assert_eq!(
        before.len(),
        2,
        "both deps unresolved before task_a completes"
    );

    // Append TaskStateChanged: task_a → Completed.
    store
        .append(&[ev(
            "evt_task_a_complete",
            RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: project(),
                task_id: task_id("a"),
                transition: StateTransition {
                    from: Some(TaskState::Running),
                    to: TaskState::Completed,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        )])
        .await
        .unwrap();

    // task_a is now completed in the read model.
    let task_a = TaskReadModel::get(store.as_ref(), &task_id("a"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task_a.state,
        TaskState::Completed,
        "task_a must be Completed"
    );

    // Resolve the dependency: mark all deps whose prerequisite is task_a as resolved.
    TaskDependencyReadModel::resolve_dependency(store.as_ref(), &task_id("a"), 50_000)
        .await
        .unwrap();

    // task_b's blocker must now be resolved.
    let b_blocked = TaskDependencyReadModel::list_blocking(store.as_ref(), &task_id("b"))
        .await
        .unwrap();
    assert_eq!(b_blocked.len(), 1);
    assert!(
        b_blocked[0].resolved_at_ms.is_some(),
        "task_b's dependency on task_a must be resolved after task_a completes"
    );
    assert_eq!(b_blocked[0].resolved_at_ms, Some(50_000));

    // task_c's blocker (on task_b) is still unresolved.
    let c_blocked = TaskDependencyReadModel::list_blocking(store.as_ref(), &task_id("c"))
        .await
        .unwrap();
    assert_eq!(
        c_blocked[0].resolved_at_ms, None,
        "task_c's dep on task_b must still be pending"
    );

    // Only 1 dependency remains unresolved (b→a is resolved; c→b is not).
    let still_unresolved = TaskDependencyReadModel::list_unresolved(store.as_ref(), &project())
        .await
        .unwrap();
    assert_eq!(
        still_unresolved.len(),
        1,
        "after resolving task_a's dep, only 1 dependency remains unresolved"
    );
    assert_eq!(
        still_unresolved[0].dependency.depends_on_task_id,
        task_id("b"),
        "the remaining unresolved dep must be c→b"
    );
}

/// (7) Circular dependency detection: adding a→b→c→a would create a cycle.
///
/// RFC 002 requires that circular dependencies be rejected. The store itself
/// does not enforce this — it is the command layer's responsibility. This test
/// proves the detection helper correctly identifies cycles vs. valid DAGs.
#[test]
fn circular_dependency_detection() {
    // Valid chain: a → b → c (no cycle).
    let deps = vec![
        dep_record("b", "a"), // b depends_on a
        dep_record("c", "b"), // c depends_on b
    ];

    // Adding d→c (a new leaf) must NOT create a cycle.
    assert!(
        !would_create_cycle(&deps, &task_id("d"), &task_id("c")),
        "adding d→c to a→b→c must not be a cycle"
    );

    // Adding a→c (adding a dependency: a also waits for c) WOULD close the cycle a←b←c←a.
    assert!(
        would_create_cycle(&deps, &task_id("a"), &task_id("c")),
        "adding a→c to a→b→c must be detected as a cycle (c depends on b, b depends on a)"
    );

    // Self-loop: a depends on itself.
    assert!(
        would_create_cycle(&[], &task_id("a"), &task_id("a")),
        "a→a self-loop must be detected as a cycle"
    );

    // Diamond (valid): b→a, c→a, d→b, d→c — no cycle.
    let diamond = vec![
        dep_record("b", "a"),
        dep_record("c", "a"),
        dep_record("d", "b"),
        dep_record("d", "c"),
    ];
    assert!(
        !would_create_cycle(&diamond, &task_id("e"), &task_id("d")),
        "adding e→d to a diamond DAG must not create a cycle"
    );

    // Close a diamond into a cycle: a depends on d (a→d in an a→b,c→d graph).
    assert!(
        would_create_cycle(&diamond, &task_id("a"), &task_id("d")),
        "closing a diamond (a→d with b,c→a→...→d) must be detected as a cycle"
    );
}

/// Multiple tasks can share the same prerequisite (fan-out).
#[tokio::test]
async fn fan_out_multiple_tasks_depend_on_same_prerequisite() {
    let store = Arc::new(InMemoryStore::new());
    seed_tasks(&store).await;

    // Both b and c depend on a.
    TaskDependencyReadModel::insert_dependency(store.as_ref(), dep_record("b", "a"))
        .await
        .unwrap();
    TaskDependencyReadModel::insert_dependency(store.as_ref(), dep_record("c", "a"))
        .await
        .unwrap();

    // Both are unresolved.
    let unresolved = TaskDependencyReadModel::list_unresolved(store.as_ref(), &project())
        .await
        .unwrap();
    assert_eq!(unresolved.len(), 2);

    // Resolve task_a → both b and c get unblocked.
    TaskDependencyReadModel::resolve_dependency(store.as_ref(), &task_id("a"), 99_000)
        .await
        .unwrap();

    let still_unresolved = TaskDependencyReadModel::list_unresolved(store.as_ref(), &project())
        .await
        .unwrap();
    assert!(
        still_unresolved.is_empty(),
        "after task_a completes, both b and c must be unblocked (fan-out resolved)"
    );

    // Both b and c now have their deps resolved.
    for name in ["b", "c"] {
        let blocked = TaskDependencyReadModel::list_blocking(store.as_ref(), &task_id(name))
            .await
            .unwrap();
        assert!(
            blocked[0].resolved_at_ms.is_some(),
            "task_{name}'s dep on task_a must be resolved"
        );
    }
}
