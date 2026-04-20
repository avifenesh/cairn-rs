//! Tests for the metrics-core feature: event-bridge tap derives
//! counters from the canonical RuntimeEvent stream, and
//! `AppMetrics::render_prometheus` emits the right series.
//!
//! These tests target the metrics module in isolation (bypassing
//! full AppState bootstrap) by writing events directly into an
//! `InMemoryStore` with a live `MetricsTap` attached. The tap's
//! broadcast consumer is what's under test; the events it consumes
//! are the same shape any production service emits.

#![cfg(feature = "metrics-core")]

use std::sync::Arc;
use std::time::Duration;

use cairn_app::metrics::AppMetrics;
use cairn_app::metrics_tap::MetricsTap;
use cairn_domain::tool_invocation::ToolInvocationOutcomeKind;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, FailureClass, ProjectKey, RunCreated, RunId, RunState,
    RunStateChanged, RuntimeEvent, StateTransition, TaskCreated, TaskId, TaskState,
    TaskStateChanged, ToolInvocationCompleted, ToolInvocationId,
};
use cairn_store::event_log::EventLog;
use cairn_store::InMemoryStore;

/// Spawn a store + metrics + tap triple. Returns the store so tests
/// can append events, the metrics handle so tests can call
/// `render_prometheus`, and the tap so teardown can shut down
/// cleanly.
async fn setup() -> (Arc<InMemoryStore>, Arc<AppMetrics>, MetricsTap) {
    let store = Arc::new(InMemoryStore::new());
    let metrics = Arc::new(AppMetrics::default());
    let tap = MetricsTap::spawn(store.clone(), metrics.clone());
    (store, metrics, tap)
}

/// The tap is async — give it a moment to drain appended events into
/// the counter mutex. 50ms is generous: in-process broadcast
/// typically resolves in microseconds, but CI noise can stretch it.
async fn drain_tap() {
    tokio::time::sleep(Duration::from_millis(200)).await;
}

fn project(tenant: &str, workspace: &str, project: &str) -> ProjectKey {
    ProjectKey::new(tenant, workspace, project)
}

fn envelope(event: RuntimeEvent, id_suffix: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("test_{id_suffix}")),
        EventSource::Runtime,
        event,
    )
}

#[tokio::test]
async fn run_created_bumps_counter() {
    let (store, metrics, tap) = setup().await;
    let p = project("t1", "w1", "p1");

    store
        .append(&[envelope(
            RuntimeEvent::RunCreated(RunCreated {
                project: p.clone(),
                run_id: RunId::new("run_1"),
                session_id: cairn_domain::SessionId::new("sess_1"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
            "run_1",
        )])
        .await
        .unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(
        output.contains(r#"cairn_runs_created_total{tenant="t1",workspace="w1"} 1"#),
        "render_prometheus output missing counter bump:\n{output}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn run_terminal_transitions_split_by_outcome_and_failure_class() {
    let (store, metrics, tap) = setup().await;
    let p = project("t1", "w1", "p1");

    // Three runs reaching different terminal states.
    let transitions = vec![
        ("run_ok", RunState::Completed, None),
        (
            "run_fail",
            RunState::Failed,
            Some(FailureClass::ExecutionError),
        ),
        ("run_cancel", RunState::Canceled, None),
    ];

    for (run_id, to_state, fc) in transitions {
        store
            .append(&[envelope(
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: p.clone(),
                    run_id: RunId::new(run_id),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: to_state,
                    },
                    failure_class: fc,
                    pause_reason: None,
                    resume_trigger: None,
                }),
                run_id,
            )])
            .await
            .unwrap();
    }

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(
        output.contains(
            r#"cairn_runs_terminal_total{tenant="t1",workspace="w1",outcome="completed",failure_class=""} 1"#
        ),
        "missing completed series:\n{output}"
    );
    assert!(
        output.contains(
            r#"cairn_runs_terminal_total{tenant="t1",workspace="w1",outcome="failed",failure_class="execution_error"} 1"#
        ),
        "missing failed series with failure_class:\n{output}"
    );
    assert!(
        output.contains(
            r#"cairn_runs_terminal_total{tenant="t1",workspace="w1",outcome="canceled",failure_class=""} 1"#
        ),
        "missing canceled series:\n{output}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn task_retryable_failed_with_lease_expired_bumps_lease_expiry_counter() {
    // This is the lease-history subscriber's end-to-end signal: it
    // emits BridgeEvent::TaskStateChanged{ to: RetryableFailed,
    // failure_class: LeaseExpired }, which the bridge writes as a
    // RuntimeEvent, which the metrics tap observes. The counter
    // surfaces worker-death events in Prometheus without any direct
    // coupling from the subscriber to AppMetrics.
    let (store, metrics, tap) = setup().await;
    let p = project("t1", "w1", "p1");

    store
        .append(&[envelope(
            RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: p.clone(),
                task_id: TaskId::new("task_dead"),
                transition: StateTransition {
                    from: Some(TaskState::Running),
                    to: TaskState::RetryableFailed,
                },
                failure_class: Some(FailureClass::LeaseExpired),
                pause_reason: None,
                resume_trigger: None,
            }),
            "task_dead",
        )])
        .await
        .unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(
        output.contains(
            r#"cairn_tasks_terminal_total{tenant="t1",workspace="w1",outcome="retryable_failed",failure_class="lease_expired"} 1"#
        ),
        "missing task terminal row:\n{output}"
    );
    assert!(
        output.contains(r#"cairn_lease_expiries_total{entity="task"} 1"#),
        "missing lease_expiry counter:\n{output}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn tool_invocations_counted_by_name_and_outcome() {
    let (store, metrics, tap) = setup().await;
    let p = project("t1", "w1", "p1");

    // Three invocations of two tools with different outcomes.
    let rows = vec![
        ("inv_1", "fs.read", ToolInvocationOutcomeKind::Success),
        ("inv_2", "fs.read", ToolInvocationOutcomeKind::Timeout),
        (
            "inv_3",
            "shell.exec",
            ToolInvocationOutcomeKind::PermanentFailure,
        ),
    ];
    for (id, tool, outcome) in rows {
        store
            .append(&[envelope(
                RuntimeEvent::ToolInvocationCompleted(ToolInvocationCompleted {
                    project: p.clone(),
                    invocation_id: ToolInvocationId::new(id),
                    task_id: None,
                    tool_name: tool.to_owned(),
                    finished_at_ms: 0,
                    outcome,
                }),
                id,
            )])
            .await
            .unwrap();
    }

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(
        output.contains(r#"cairn_tool_invocations_total{tool="fs.read",outcome="success"} 1"#),
        "missing fs.read/success row:\n{output}"
    );
    assert!(
        output.contains(r#"cairn_tool_invocations_total{tool="fs.read",outcome="timeout"} 1"#),
        "missing fs.read/timeout row:\n{output}"
    );
    assert!(
        output.contains(
            r#"cairn_tool_invocations_total{tool="shell.exec",outcome="permanent_failure"} 1"#
        ),
        "missing shell.exec/permanent_failure row:\n{output}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn unrelated_events_do_not_bump_counters() {
    // Regression guard: non-lifecycle events (e.g. checkpoint
    // recorded, approval requested) must NOT touch lifecycle
    // counters. Appending a TaskCreated bumps tasks_created; ensure
    // nothing else in the TaskCreated path accidentally bumped
    // tasks_terminal.
    let (store, metrics, tap) = setup().await;
    let p = project("t1", "w1", "p1");

    store
        .append(&[envelope(
            RuntimeEvent::TaskCreated(TaskCreated {
                project: p.clone(),
                task_id: TaskId::new("task_1"),
                parent_run_id: None,
                parent_task_id: None,
                prompt_release_id: None,
                session_id: None,
            }),
            "task_1",
        )])
        .await
        .unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(output.contains(r#"cairn_tasks_created_total{tenant="t1",workspace="w1"} 1"#));
    assert!(
        !output.contains("cairn_tasks_terminal_total{tenant=\"t1\""),
        "tasks_terminal must not have any rows after a pure-create:\n{output}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn run_failed_with_lease_expired_bumps_run_entity_counter() {
    // Symmetric to the task path: the lease-history subscriber emits
    // ExecutionFailed { failure_class: LeaseExpired } when FF reclaims
    // a dead run. That surfaces as RunStateChanged to Failed with
    // failure_class=LeaseExpired, which must bump the
    // `entity="run"` series on cairn_lease_expiries_total. Without
    // this, run-level worker deaths are invisible in the metric the
    // PR description says covers them.
    let (store, metrics, tap) = setup().await;
    let p = project("t1", "w1", "p1");

    store
        .append(&[envelope(
            RuntimeEvent::RunStateChanged(RunStateChanged {
                project: p.clone(),
                run_id: RunId::new("run_dead"),
                transition: StateTransition {
                    from: Some(RunState::Running),
                    to: RunState::Failed,
                },
                failure_class: Some(FailureClass::LeaseExpired),
                pause_reason: None,
                resume_trigger: None,
            }),
            "run_dead",
        )])
        .await
        .unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(
        output.contains(r#"cairn_lease_expiries_total{entity="run"} 1"#),
        "missing run lease_expiry counter:\n{output}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn retain_tenant_queue_depth_prunes_stale_entries() {
    // Simulate a scrape cycle: set depth for three tenants, then a
    // follow-up refresh sees only two. The missing tenant's entry
    // must be dropped from the Prometheus output so deleted tenants
    // don't linger as phantom series.
    let (_store, metrics, tap) = setup().await;
    metrics.set_tenant_queue_depth("t_a", 1, 2, 3);
    metrics.set_tenant_queue_depth("t_b", 4, 5, 6);
    metrics.set_tenant_queue_depth("t_gone", 7, 8, 9);

    let before = metrics.render_prometheus();
    assert!(before.contains(r#"cairn_active_runs_by_tenant{tenant="t_gone"} 7"#));

    // Scrape-time refresh sees only t_a and t_b as live.
    metrics.retain_tenant_queue_depth(&["t_a".to_owned(), "t_b".to_owned()]);

    let after = metrics.render_prometheus();
    assert!(after.contains(r#"cairn_active_runs_by_tenant{tenant="t_a"} 1"#));
    assert!(after.contains(r#"cairn_active_runs_by_tenant{tenant="t_b"} 4"#));
    assert!(
        !after.contains("t_gone"),
        "t_gone series must be pruned from output:\n{after}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn render_prometheus_surface_always_present() {
    // Even before any events, the Prometheus render must include
    // the metric HELP/TYPE lines for every metrics-core series so
    // Prometheus scrapers don't skip the metric entirely on an
    // empty cairn.
    let (_store, metrics, tap) = setup().await;

    let output = metrics.render_prometheus();
    for line in &[
        "# TYPE cairn_runs_created_total counter",
        "# TYPE cairn_runs_terminal_total counter",
        "# TYPE cairn_tasks_created_total counter",
        "# TYPE cairn_tasks_terminal_total counter",
        "# TYPE cairn_tool_invocations_total counter",
        "# TYPE cairn_lease_expiries_total counter",
        "# TYPE cairn_projection_lag_events gauge",
        "# TYPE cairn_active_runs_by_tenant gauge",
        "# TYPE cairn_active_tasks_by_tenant gauge",
        "# TYPE cairn_pending_approvals_by_tenant gauge",
    ] {
        assert!(
            output.contains(line),
            "render_prometheus missing TYPE declaration `{line}`:\n{output}"
        );
    }

    tap.shutdown().await;
}
