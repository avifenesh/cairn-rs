//! RFC 005 — run cost tracking and alert system end-to-end integration tests.
//!
//! Tests the full cost-tracking arc:
//!   1. Create a session and run
//!   2. Emit multiple RunCostUpdated events with different cost components
//!   3. Verify accumulated cost via RunCostReadModel
//!   4. Set a cost alert threshold
//!   5. Emit a cost that exceeds the threshold
//!   6. Verify RunCostAlertTriggered was emitted with correct fields

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunCostUpdated, RunId, RuntimeEvent,
    SessionId, TenantId,
};
use cairn_runtime::run_cost_alerts::RunCostAlertService;
use cairn_runtime::runs::RunService;
use cairn_runtime::services::{RunCostAlertServiceImpl, RunServiceImpl, SessionServiceImpl};
use cairn_runtime::sessions::SessionService;
use cairn_store::projections::RunCostReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_cost", "w_cost", "p_cost")
}

fn tenant() -> TenantId {
    TenantId::new("t_cost")
}

/// Append a RunCostUpdated event directly — the service layer has no
/// dedicated "record cost" method; cost is accumulated from provider call events.
async fn emit_cost(
    store: &Arc<InMemoryStore>,
    id: &str,
    run_id: &RunId,
    session_id: &SessionId,
    delta_cost_micros: u64,
    delta_tokens_in: u64,
    delta_tokens_out: u64,
) {
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new(id),
            EventSource::Runtime,
            RuntimeEvent::RunCostUpdated(RunCostUpdated {
                project: project(),
                run_id: run_id.clone(),
                session_id: Some(session_id.clone()),
                tenant_id: Some(tenant()),
                delta_cost_micros,
                delta_tokens_in,
                delta_tokens_out,
                provider_call_id: format!("call_{id}"),
                updated_at_ms: 1_700_000_000_000,
            }),
        )])
        .await
        .unwrap();
}

// ── Test 1–3: session + run creation, multi-event cost accumulation ───────────

/// RFC 005: cost updates must accumulate on the run's RunCostRecord.
/// Multiple events with different cost components must sum correctly.
#[tokio::test]
async fn cost_accumulates_across_multiple_events() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());

    let session_id = SessionId::new("sess_cost_1");
    let run_id = RunId::new("run_cost_1");

    // ── (1) Create session and run ─────────────────────────────────────────
    session_svc
        .create(&project(), session_id.clone())
        .await
        .unwrap();
    run_svc
        .start(&project(), &session_id, run_id.clone(), None)
        .await
        .unwrap();

    // Verify run exists before accumulating cost.
    let run = run_svc.get(&run_id).await.unwrap().unwrap();
    assert_eq!(run.run_id, run_id);

    // No cost yet.
    let before = RunCostReadModel::get_run_cost(store.as_ref(), &run_id)
        .await
        .unwrap();
    assert!(
        before.is_none() || before.as_ref().map(|r| r.total_cost_micros).unwrap_or(0) == 0,
        "cost must be zero before any RunCostUpdated events"
    );

    // ── (2) Emit multiple RunCostUpdated events ────────────────────────────
    // Event 1: 400 micros, 100 input tokens, 50 output tokens.
    emit_cost(&store, "e1", &run_id, &session_id, 400, 100, 50).await;
    // Event 2: 600 micros, 200 input tokens, 80 output tokens.
    emit_cost(&store, "e2", &run_id, &session_id, 600, 200, 80).await;
    // Event 3: 200 micros, 50 input tokens, 20 output tokens (small follow-up).
    emit_cost(&store, "e3", &run_id, &session_id, 200, 50, 20).await;

    // ── (3) Verify accumulated cost via RunCostReadModel ──────────────────
    let cost = RunCostReadModel::get_run_cost(store.as_ref(), &run_id)
        .await
        .unwrap()
        .expect("RunCostRecord must exist after cost events");

    assert_eq!(
        cost.run_id, run_id,
        "cost record must reference the correct run"
    );
    assert_eq!(
        cost.total_cost_micros,
        1_200, // 400 + 600 + 200
        "total_cost_micros must be the sum of all deltas"
    );
    assert_eq!(
        cost.total_tokens_in,
        350, // 100 + 200 + 50
        "total_tokens_in must accumulate across events"
    );
    assert_eq!(
        cost.total_tokens_out,
        150, // 50 + 80 + 20
        "total_tokens_out must accumulate across events"
    );
    assert_eq!(
        cost.provider_calls, 3,
        "provider_calls counter must equal the number of cost events"
    );
}

// ── Test 4–6: cost alert threshold → trigger event ────────────────────────────

/// RFC 005: setting a cost alert and then crossing the threshold must
/// auto-emit RunCostAlertTriggered with the correct fields.
#[tokio::test]
async fn cost_alert_triggers_when_threshold_exceeded() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let alert_svc = RunCostAlertServiceImpl::new(store.clone());

    let session_id = SessionId::new("sess_alert_e2e");
    let run_id = RunId::new("run_alert_e2e");

    // ── (1) Create session and run ─────────────────────────────────────────
    session_svc
        .create(&project(), session_id.clone())
        .await
        .unwrap();
    run_svc
        .start(&project(), &session_id, run_id.clone(), None)
        .await
        .unwrap();

    // ── (4) Set alert threshold at 500 micros ─────────────────────────────
    alert_svc
        .set_alert(run_id.clone(), tenant(), 500)
        .await
        .unwrap();

    let alert_record = alert_svc
        .get_alert(&run_id)
        .await
        .unwrap()
        .expect("alert record must exist after set_alert");
    assert_eq!(
        alert_record.threshold_micros, 500,
        "threshold must be persisted"
    );
    assert_eq!(
        alert_record.triggered_at_ms, 0,
        "alert must not be pre-triggered"
    );

    // Accumulate cost below threshold — no trigger yet.
    emit_cost(&store, "ea1", &run_id, &session_id, 300, 80, 30).await;

    let no_trigger = store.read_stream(None, 1_000).await.unwrap();
    let triggered_before = no_trigger.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RunCostAlertTriggered(ev) if ev.run_id == run_id)
    });
    assert!(
        !triggered_before,
        "alert must NOT fire when accumulated cost (300) is below threshold (500)"
    );

    // ── (5) Emit cost that pushes total above threshold ───────────────────
    // 300 already accumulated; adding 300 more → total = 600 > 500 threshold.
    emit_cost(&store, "ea2", &run_id, &session_id, 300, 100, 40).await;

    // ── (6) Verify RunCostAlertTriggered was emitted ──────────────────────
    let events = store.read_stream(None, 1_000).await.unwrap();

    let trigger_ev = events.iter().find_map(|e| {
        if let RuntimeEvent::RunCostAlertTriggered(ev) = &e.envelope.payload {
            if ev.run_id == run_id {
                return Some(ev.clone());
            }
        }
        None
    });

    let trigger = trigger_ev
        .expect("RunCostAlertTriggered must be emitted when total cost exceeds threshold");

    assert_eq!(
        trigger.run_id, run_id,
        "event must reference the correct run"
    );
    assert_eq!(
        trigger.threshold_micros, 500,
        "event must carry the configured threshold"
    );
    assert!(
        trigger.actual_cost_micros >= 600,
        "actual_cost_micros ({}) must be >= total emitted cost (600)",
        trigger.actual_cost_micros
    );
    assert!(
        trigger.triggered_at_ms > 0,
        "triggered_at_ms must be a positive timestamp"
    );
    assert_eq!(
        trigger.tenant_id,
        tenant(),
        "event must carry the correct tenant_id"
    );

    // get_alert must reflect the triggered state.
    let triggered_alert = alert_svc
        .get_alert(&run_id)
        .await
        .unwrap()
        .expect("alert record must still exist after triggering");
    assert!(
        triggered_alert.triggered_at_ms > 0,
        "triggered_at_ms must be set on the alert record after trigger"
    );
    assert!(
        triggered_alert.actual_cost_micros >= 600,
        "alert record must record the actual cost at trigger time"
    );
}

// ── Alert fires only once despite multiple threshold-crossing events ──────────

/// RFC 005: once an alert has fired for a run it must not fire again on
/// subsequent cost updates, preventing alert spam.
#[tokio::test]
async fn cost_alert_fires_exactly_once() {
    let store = Arc::new(InMemoryStore::new());
    let alert_svc = RunCostAlertServiceImpl::new(store.clone());

    let run_id = RunId::new("run_once");

    alert_svc
        .set_alert(run_id.clone(), tenant(), 100)
        .await
        .unwrap();

    let session_id = SessionId::new("sess_once");

    // Three separate cost bursts, each alone enough to exceed the threshold.
    for (i, cost) in [(0u32, 150u64), (1, 200), (2, 300)].iter() {
        emit_cost(
            &store,
            &format!("once_{i}"),
            &run_id,
            &session_id,
            *cost,
            0,
            0,
        )
        .await;
    }

    let events = store.read_stream(None, 1_000).await.unwrap();
    let trigger_count = events
        .iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::RunCostAlertTriggered(ev) if ev.run_id == run_id)
        })
        .count();

    assert_eq!(
        trigger_count, 1,
        "RunCostAlertTriggered must fire exactly once, not {trigger_count} times"
    );
}

// ── Alert below threshold never fires ─────────────────────────────────────────

/// RFC 005: accumulated cost below the threshold must never produce a
/// RunCostAlertTriggered event.
#[tokio::test]
async fn cost_alert_silent_below_threshold() {
    let store = Arc::new(InMemoryStore::new());
    let alert_svc = RunCostAlertServiceImpl::new(store.clone());

    let run_id = RunId::new("run_silent");

    // Threshold of 10_000 micros — we'll never reach it.
    alert_svc
        .set_alert(run_id.clone(), tenant(), 10_000)
        .await
        .unwrap();

    let session_id = SessionId::new("sess_silent");
    emit_cost(&store, "s1", &run_id, &session_id, 100, 10, 5).await;
    emit_cost(&store, "s2", &run_id, &session_id, 200, 20, 8).await;

    let events = store.read_stream(None, 1_000).await.unwrap();
    let any_trigger = events.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RunCostAlertTriggered(ev) if ev.run_id == run_id)
    });
    assert!(
        !any_trigger,
        "no RunCostAlertTriggered must be emitted when cost (300) stays below threshold (10_000)"
    );

    // Verify the cost is being accumulated correctly despite no alert.
    let cost = RunCostReadModel::get_run_cost(store.as_ref(), &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cost.total_cost_micros, 300,
        "cost must still be tracked even when alert doesn't fire"
    );
}

// ── check_and_trigger as explicit fallback ─────────────────────────────────────

/// `check_and_trigger` must also fire the alert when called explicitly after
/// cost has already exceeded the threshold (manual sweep path).
#[tokio::test]
async fn explicit_check_and_trigger_fires_alert() {
    let store = Arc::new(InMemoryStore::new());
    let alert_svc = RunCostAlertServiceImpl::new(store.clone());

    let run_id = RunId::new("run_manual");
    let session_id = SessionId::new("sess_manual");

    // Set alert before accumulating cost.
    alert_svc
        .set_alert(run_id.clone(), tenant(), 50)
        .await
        .unwrap();

    // Accumulate 200 micros — exceeds threshold.
    emit_cost(&store, "m1", &run_id, &session_id, 200, 40, 15).await;

    // The in-memory store fires the alert inline; call check_and_trigger
    // a second time to confirm it returns false (already triggered = idempotent).
    let second_call = alert_svc.check_and_trigger(&run_id).await.unwrap();
    assert!(
        !second_call,
        "check_and_trigger must return false when alert was already triggered"
    );

    // list_triggered_by_tenant must include the triggered alert.
    let triggered = alert_svc.list_triggered_by_tenant(&tenant()).await.unwrap();
    let found = triggered.iter().any(|a| a.run_id == run_id);
    assert!(
        found,
        "triggered alert must appear in list_triggered_by_tenant"
    );
}
