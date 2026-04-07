//! RFC 004 eval run lifecycle integration tests.
//!
//! Validates the eval pipeline through InMemoryStore:
//! - EvalRunStarted projects into EvalRunReadModel in running state.
//! - EvalRunCompleted(success=true) updates state with success and subject link.
//! - EvalRunCompleted(success=false) records failure with error_message.
//! - list_by_project returns runs sorted by started_at.
//! - subject_kind="prompt_asset" links the eval run to a prompt asset.

use std::sync::Arc;

use cairn_domain::{
    EvalRunId, EventEnvelope, EventId, EventSource, ProjectKey, PromptAssetId, RuntimeEvent,
};
use cairn_domain::events::{EvalRunCompleted, EvalRunStarted};
use cairn_store::{projections::EvalRunReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_eval", "ws_eval", "proj_eval")
}

fn other_project() -> ProjectKey {
    ProjectKey::new("tenant_eval", "ws_eval", "proj_other")
}

fn run_id(n: &str) -> EvalRunId {
    EvalRunId::new(format!("eval_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn start_event(id: &str, subject_kind: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_start_{id}"),
        RuntimeEvent::EvalRunStarted(EvalRunStarted {
            project: project(),
            eval_run_id: run_id(id),
            subject_kind: subject_kind.to_owned(),
            evaluator_type: "llm_judge".to_owned(),
            started_at: ts,
            prompt_asset_id: None,
            prompt_version_id: None,
            prompt_release_id: None,
            created_by: None,
        }),
    )
}

fn complete_success(id: &str, subject_node_id: Option<&str>, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_complete_{id}"),
        RuntimeEvent::EvalRunCompleted(EvalRunCompleted {
            project: project(),
            eval_run_id: run_id(id),
            success: true,
            error_message: None,
            subject_node_id: subject_node_id.map(str::to_owned),
            completed_at: ts,
        }),
    )
}

fn complete_failure(id: &str, error: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_fail_{id}"),
        RuntimeEvent::EvalRunCompleted(EvalRunCompleted {
            project: project(),
            eval_run_id: run_id(id),
            success: false,
            error_message: Some(error.to_owned()),
            subject_node_id: None,
            completed_at: ts,
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): EvalRunStarted projects into the read model in running state.
#[tokio::test]
async fn eval_run_started_shows_running_state() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[start_event("run_1", "prompt_release", 1_000)]).await.unwrap();

    let rec = EvalRunReadModel::get(store.as_ref(), &run_id("run_1"))
        .await.unwrap()
        .expect("eval run must exist after EvalRunStarted");

    assert_eq!(rec.eval_run_id, run_id("run_1"));
    assert_eq!(rec.project, project());
    assert_eq!(rec.subject_kind, "prompt_release");
    assert_eq!(rec.evaluator_type, "llm_judge");
    assert_eq!(rec.started_at, 1_000);
    assert!(rec.success.is_none(), "success must be None (run still in progress)");
    assert!(rec.error_message.is_none(), "no error while running");
    assert!(rec.completed_at.is_none(), "completed_at must be None while running");
}

/// (3) + (4): EvalRunCompleted(success=true) updates state with success and metrics.
///
/// "Metrics" at the store level: success flag, completed_at timestamp, and
/// subject_node_id linking to the evaluated artifact (e.g. prompt release ID).
#[tokio::test]
async fn eval_run_completed_success_persists_state() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        start_event("success_run", "prompt_release", 2_000),
        complete_success("success_run", Some("rel_v3"), 3_500),
    ]).await.unwrap();

    let rec = EvalRunReadModel::get(store.as_ref(), &run_id("success_run"))
        .await.unwrap().unwrap();

    // MUST: success flag set to true.
    assert_eq!(rec.success, Some(true), "success must be Some(true) after successful completion");
    // MUST: no error on success.
    assert!(rec.error_message.is_none(), "error_message must be None on success");
    // MUST: completed_at set.
    assert_eq!(rec.completed_at, Some(3_500), "completed_at must match EvalRunCompleted.completed_at");
    // MUST: duration is positive.
    assert!(rec.completed_at.unwrap() > rec.started_at,
        "completed_at must be after started_at");
}

/// (5): Failure path — EvalRunCompleted(success=false) records error message.
#[tokio::test]
async fn eval_run_failure_records_error_message() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        start_event("fail_run", "retrieval_policy", 4_000),
        complete_failure("fail_run", "scorer timed out after 30s", 5_000),
    ]).await.unwrap();

    let rec = EvalRunReadModel::get(store.as_ref(), &run_id("fail_run"))
        .await.unwrap().unwrap();

    assert_eq!(rec.success, Some(false), "success must be Some(false) on failure");
    assert!(
        rec.error_message.as_ref().map_or(false, |e| e.contains("timed out")),
        "error_message must contain the failure reason; got: {:?}",
        rec.error_message
    );
    assert_eq!(rec.completed_at, Some(5_000), "failed run must still set completed_at");
}

/// (6): list_by_project returns runs sorted by started_at (ascending).
#[tokio::test]
async fn list_by_project_returns_runs_in_order() {
    let store = Arc::new(InMemoryStore::new());

    // Three runs started at different times (appended out of order).
    store.append(&[
        start_event("run_c", "prompt_release", 3_000),
        start_event("run_a", "prompt_release", 1_000),
        start_event("run_b", "prompt_release", 2_000),
    ]).await.unwrap();

    // Complete two of them.
    store.append(&[
        complete_success("run_a", None, 1_800),
        complete_failure("run_c", "assertion failed", 3_900),
    ]).await.unwrap();

    // Append a run from a different project — must not appear.
    store.append(&[ev("evt_other", RuntimeEvent::EvalRunStarted(EvalRunStarted {
        project: other_project(),
        eval_run_id: EvalRunId::new("eval_other"),
        subject_kind: "prompt_release".to_owned(),
        evaluator_type: "auto".to_owned(),
        started_at: 500,
        prompt_asset_id: None,
        prompt_version_id: None,
        prompt_release_id: None,
        created_by: None,
    }))]).await.unwrap();

    let runs = EvalRunReadModel::list_by_project(store.as_ref(), &project(), 10, 0)
        .await.unwrap();

    // Must return exactly 3 runs (not the other-project run).
    assert_eq!(runs.len(), 3, "list_by_project must return exactly 3 runs for the project");

    // Must be sorted by started_at ascending.
    assert_eq!(runs[0].eval_run_id, run_id("run_a"), "first run must be run_a (started at 1_000)");
    assert_eq!(runs[1].eval_run_id, run_id("run_b"), "second run must be run_b (started at 2_000)");
    assert_eq!(runs[2].eval_run_id, run_id("run_c"), "third run must be run_c (started at 3_000)");

    // Completed runs reflect their state.
    assert_eq!(runs[0].success, Some(true),  "run_a succeeded");
    assert_eq!(runs[1].success, None,        "run_b still running");
    assert_eq!(runs[2].success, Some(false), "run_c failed");

    // Pagination: limit=2 offset=0 → first two.
    let page1 = EvalRunReadModel::list_by_project(store.as_ref(), &project(), 2, 0)
        .await.unwrap();
    assert_eq!(page1.len(), 2);
    let page2 = EvalRunReadModel::list_by_project(store.as_ref(), &project(), 2, 2)
        .await.unwrap();
    assert_eq!(page2.len(), 1);
}

/// (7): Eval run links to prompt_asset_id via subject_kind + subject_node_id.
///
/// RFC 004: eval runs that evaluate a prompt asset MUST set subject_kind to
/// "prompt_asset" and include the asset's ID as subject_node_id so operators
/// can trace which asset was evaluated.
#[tokio::test]
async fn eval_run_links_to_prompt_asset_id() {
    let store = Arc::new(InMemoryStore::new());
    let prompt_asset_id = PromptAssetId::new("prompt_assistant_v2");

    store.append(&[
        ev("evt_start_pa", RuntimeEvent::EvalRunStarted(EvalRunStarted {
            project: project(),
            eval_run_id: run_id("asset_eval"),
            subject_kind: "prompt_asset".to_owned(),   // ← links to prompt asset
            evaluator_type: "regression_suite".to_owned(),
            started_at: 10_000,
            prompt_asset_id: None,
            prompt_version_id: None,
            prompt_release_id: None,
            created_by: None,
        })),
        ev("evt_complete_pa", RuntimeEvent::EvalRunCompleted(EvalRunCompleted {
            project: project(),
            eval_run_id: run_id("asset_eval"),
            success: true,
            error_message: None,
            subject_node_id: Some(prompt_asset_id.as_str().to_owned()), // ← asset ID
            completed_at: 15_000,
        })),
    ]).await.unwrap();

    let rec = EvalRunReadModel::get(store.as_ref(), &run_id("asset_eval"))
        .await.unwrap().unwrap();

    // MUST: subject_kind identifies the type of artifact evaluated.
    assert_eq!(rec.subject_kind, "prompt_asset",
        "subject_kind must be 'prompt_asset' for prompt asset evaluations");

    // MUST: success is recorded.
    assert_eq!(rec.success, Some(true));

    // MUST: link is established through the event log — verify via read_stream.
    let events = EventLog::read_stream(store.as_ref(), None, 100).await.unwrap();
    let completion = events.iter().find(|e| {
        matches!(
            &e.envelope.payload,
            RuntimeEvent::EvalRunCompleted(c)
                if c.eval_run_id == run_id("asset_eval")
                && c.subject_node_id.as_deref() == Some(prompt_asset_id.as_str())
        )
    });
    assert!(
        completion.is_some(),
        "EvalRunCompleted must carry subject_node_id linking to the prompt asset"
    );
}

/// Project-scoped isolation: eval runs from other projects are excluded.
#[tokio::test]
async fn eval_run_list_is_project_scoped() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        start_event("proj_run", "prompt_release", 1_000),
        ev("evt_other", RuntimeEvent::EvalRunStarted(EvalRunStarted {
            project: other_project(),
            eval_run_id: EvalRunId::new("eval_other"),
            subject_kind: "prompt_release".to_owned(),
            evaluator_type: "auto".to_owned(),
            started_at: 500,
            prompt_asset_id: None,
            prompt_version_id: None,
            prompt_release_id: None,
            created_by: None,
        })),
    ]).await.unwrap();

    let project_runs = EvalRunReadModel::list_by_project(store.as_ref(), &project(), 10, 0)
        .await.unwrap();
    assert_eq!(project_runs.len(), 1, "only runs from project must be listed");
    assert_eq!(project_runs[0].eval_run_id, run_id("proj_run"));

    let other_runs = EvalRunReadModel::list_by_project(store.as_ref(), &other_project(), 10, 0)
        .await.unwrap();
    assert_eq!(other_runs.len(), 1);
    assert_ne!(other_runs[0].eval_run_id, run_id("proj_run"),
        "other_project must not see project's runs");
}
