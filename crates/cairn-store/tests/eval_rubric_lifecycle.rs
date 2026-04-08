//! Eval rubric and baseline lifecycle tests (RFC 002).
//!
//! Validates that eval rubrics and baselines are durably stored, their
//! lifecycle events are projected correctly, and the locked state prevents
//! further updates at the projection layer.
//!
//! Note on event schema gaps:
//!   EvalRubricCreated  — has rubric_id, name, created_at_ms but NO tenant_id.
//!                        Stored with sentinel tenant_id ""; list_by_tenant("") returns all.
//!   EvalBaselineSet    — carries baseline_id, metric, value, set_at_ms.
//!                        No tenant_id, name, or prompt_asset_id in the event.
//!   EvalBaselineLocked — sets locked=true on the baseline; projection enforces
//!                        immutability by ignoring EvalBaselineSet on locked baselines.

use cairn_domain::{
    EvalBaselineLocked, EvalBaselineSet, EvalRubricCreated, EventEnvelope, EventId, EventSource,
    RuntimeEvent, TenantId,
};
use cairn_store::{
    projections::{EvalBaselineReadModel, EvalRubricReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    use cairn_domain::OwnershipKey;
    EventEnvelope {
        event_id: EventId::new(id),
        source: EventSource::Runtime,
        ownership: OwnershipKey::System,
        causation_id: None,
        correlation_id: None,
        payload,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn create_rubric(
    evt_id: &str,
    rubric_id: &str,
    name: &str,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::EvalRubricCreated(EvalRubricCreated {
            rubric_id: rubric_id.to_owned(),
            name: name.to_owned(),
            created_at_ms: ts,
        }),
    )
}

fn set_baseline(
    evt_id: &str,
    baseline_id: &str,
    metric: &str,
    value: &str,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::EvalBaselineSet(EvalBaselineSet {
            baseline_id: baseline_id.to_owned(),
            metric: metric.to_owned(),
            value: value.to_owned(),
            set_at_ms: ts,
        }),
    )
}

fn lock_baseline(evt_id: &str, baseline_id: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::EvalBaselineLocked(EvalBaselineLocked {
            baseline_id: baseline_id.to_owned(),
            locked_at_ms: ts,
        }),
    )
}

// ── 1. EvalRubricCreated stores the record ────────────────────────────────────

#[tokio::test]
async fn eval_rubric_created_stores_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_rubric("e1", "rubric_001", "Accuracy Rubric", ts)])
        .await
        .unwrap();

    let rubric = EvalRubricReadModel::get_rubric(&store, "rubric_001")
        .await
        .unwrap()
        .expect("EvalRubric must exist after EvalRubricCreated");

    assert_eq!(rubric.rubric_id, "rubric_001");
    assert_eq!(rubric.name, "Accuracy Rubric");
    assert_eq!(rubric.created_at_ms, ts);
    assert!(
        rubric.dimensions.is_empty(),
        "new rubric has no dimensions from event"
    );
}

// ── 2. get_rubric returns None for unknown ID ──────────────────────────────────

#[tokio::test]
async fn get_rubric_returns_none_for_unknown_id() {
    let store = InMemoryStore::new();
    assert!(EvalRubricReadModel::get_rubric(&store, "ghost")
        .await
        .unwrap()
        .is_none());
}

// ── 3. EvalRubricCreated is idempotent (or_insert_with) ───────────────────────

#[tokio::test]
async fn eval_rubric_created_is_idempotent() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            create_rubric("e1", "rubric_idem", "Version 1", ts),
            create_rubric("e2", "rubric_idem", "Version 2", ts + 1),
        ])
        .await
        .unwrap();

    let rubric = EvalRubricReadModel::get_rubric(&store, "rubric_idem")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rubric.name, "Version 1",
        "first creation wins — subsequent events are idempotent"
    );
}

// ── 4. EvalBaselineSet stores a baseline record ───────────────────────────────

#[tokio::test]
async fn eval_baseline_set_stores_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[set_baseline(
            "e1",
            "baseline_001",
            "task_success_rate",
            "0.92",
            ts,
        )])
        .await
        .unwrap();

    let baseline = EvalBaselineReadModel::get_baseline(&store, "baseline_001")
        .await
        .unwrap()
        .expect("EvalBaseline must exist after EvalBaselineSet");

    assert_eq!(baseline.baseline_id, "baseline_001");
    assert!(!baseline.locked, "new baseline is unlocked");
}

// ── 5. EvalBaselineSet updates the record (metric name stored in name field) ──

#[tokio::test]
async fn eval_baseline_set_updates_metric() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            set_baseline("e1", "bl_metric", "task_success_rate", "0.85", ts),
            set_baseline("e2", "bl_metric", "latency_p50_ms", "200", ts + 1),
        ])
        .await
        .unwrap();

    let baseline = EvalBaselineReadModel::get_baseline(&store, "bl_metric")
        .await
        .unwrap()
        .unwrap();
    // Latest EvalBaselineSet's metric is reflected in the name.
    assert!(
        baseline.name.contains("latency_p50_ms"),
        "latest metric must be reflected in baseline record"
    );
    assert!(baseline.name.contains("200"));
}

// ── 6. EvalBaselineLocked sets locked=true ────────────────────────────────────

#[tokio::test]
async fn eval_baseline_locked_sets_locked_flag() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[set_baseline(
            "e1",
            "bl_lock",
            "task_success_rate",
            "0.90",
            ts,
        )])
        .await
        .unwrap();

    let before = EvalBaselineReadModel::get_baseline(&store, "bl_lock")
        .await
        .unwrap()
        .unwrap();
    assert!(!before.locked, "baseline starts unlocked");

    store
        .append(&[lock_baseline("e2", "bl_lock", ts + 1_000)])
        .await
        .unwrap();

    let after = EvalBaselineReadModel::get_baseline(&store, "bl_lock")
        .await
        .unwrap()
        .unwrap();
    assert!(after.locked, "EvalBaselineLocked must set locked=true");
}

// ── 7. Locked baseline ignores further EvalBaselineSet events ─────────────────

#[tokio::test]
async fn locked_baseline_ignores_subsequent_set_events() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            set_baseline("e1", "bl_immut", "task_success_rate", "0.90", ts),
            lock_baseline("e2", "bl_immut", ts + 1_000),
        ])
        .await
        .unwrap();

    let locked = EvalBaselineReadModel::get_baseline(&store, "bl_immut")
        .await
        .unwrap()
        .unwrap();
    let name_before_attempt = locked.name.clone();

    // Attempt to set another metric AFTER locking.
    store
        .append(&[set_baseline(
            "e3",
            "bl_immut",
            "latency_p50_ms",
            "999",
            ts + 2_000,
        )])
        .await
        .unwrap();

    let still_locked = EvalBaselineReadModel::get_baseline(&store, "bl_immut")
        .await
        .unwrap()
        .unwrap();
    assert!(still_locked.locked, "baseline must remain locked");
    assert_eq!(
        still_locked.name, name_before_attempt,
        "locked baseline must not be updated by subsequent EvalBaselineSet"
    );
}

// ── 8. EvalBaselineLocked on unknown baseline is a no-op ─────────────────────

#[tokio::test]
async fn locking_unknown_baseline_is_noop() {
    let store = InMemoryStore::new();
    store
        .append(&[lock_baseline("e1", "ghost_baseline", now_ms())])
        .await
        .unwrap();
    let result = EvalBaselineReadModel::get_baseline(&store, "ghost_baseline")
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "locking non-existent baseline must not create a record"
    );
}

// ── 9. Multiple rubrics tracked independently ─────────────────────────────────

#[tokio::test]
async fn multiple_rubrics_tracked_independently() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            create_rubric("e1", "rubric_a", "Safety Rubric", ts),
            create_rubric("e2", "rubric_b", "Quality Rubric", ts + 1),
            create_rubric("e3", "rubric_c", "Latency Rubric", ts + 2),
        ])
        .await
        .unwrap();

    for (id, name) in [
        ("rubric_a", "Safety Rubric"),
        ("rubric_b", "Quality Rubric"),
        ("rubric_c", "Latency Rubric"),
    ] {
        let r = EvalRubricReadModel::get_rubric(&store, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.name, name);
    }
}

// ── 10. Multiple baselines tracked independently ──────────────────────────────

#[tokio::test]
async fn multiple_baselines_tracked_independently() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            set_baseline("e1", "bl_prod", "task_success_rate", "0.95", ts),
            set_baseline("e2", "bl_staging", "task_success_rate", "0.88", ts + 1),
        ])
        .await
        .unwrap();

    let prod = EvalBaselineReadModel::get_baseline(&store, "bl_prod")
        .await
        .unwrap()
        .unwrap();
    let staging = EvalBaselineReadModel::get_baseline(&store, "bl_staging")
        .await
        .unwrap()
        .unwrap();

    assert!(prod.name.contains("0.95"));
    assert!(staging.name.contains("0.88"));
    assert!(!prod.locked);
    assert!(!staging.locked);
}

// ── 11. list_by_tenant with sentinel returns all rubrics ──────────────────────

#[tokio::test]
async fn list_by_tenant_sentinel_returns_all_rubrics() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            create_rubric("e1", "rub_list_a", "Rubric A", ts),
            create_rubric("e2", "rub_list_b", "Rubric B", ts + 1),
            create_rubric("e3", "rub_list_c", "Rubric C", ts + 2),
        ])
        .await
        .unwrap();

    // sentinel tenant "" returns all since event has no tenant_id
    let rubrics = EvalRubricReadModel::list_by_tenant(&store, &TenantId::new(""), 10, 0)
        .await
        .unwrap();
    assert_eq!(rubrics.len(), 3, "sentinel query returns all rubrics");

    // Sorted by rubric_id ascending.
    assert_eq!(rubrics[0].rubric_id, "rub_list_a");
    assert_eq!(rubrics[1].rubric_id, "rub_list_b");
    assert_eq!(rubrics[2].rubric_id, "rub_list_c");
}

// ── 12. list_by_tenant with sentinel returns all baselines ────────────────────

#[tokio::test]
async fn list_by_tenant_sentinel_returns_all_baselines() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            set_baseline("e1", "bl_list_a", "metric_1", "1.0", ts),
            set_baseline("e2", "bl_list_b", "metric_1", "0.9", ts + 1),
            lock_baseline("e3", "bl_list_a", ts + 2),
        ])
        .await
        .unwrap();

    let baselines = EvalBaselineReadModel::list_by_tenant(&store, &TenantId::new(""), 10, 0)
        .await
        .unwrap();
    assert_eq!(baselines.len(), 2);

    let locked_one = baselines
        .iter()
        .find(|b| b.baseline_id == "bl_list_a")
        .unwrap();
    let open_one = baselines
        .iter()
        .find(|b| b.baseline_id == "bl_list_b")
        .unwrap();
    assert!(locked_one.locked);
    assert!(!open_one.locked);
}

// ── 13. list_by_tenant pagination ─────────────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_rubrics_pagination() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u64..4 {
        store
            .append(&[create_rubric(
                &format!("e{i}"),
                &format!("rub_pg_{i:02}"),
                &format!("Rubric {i}"),
                ts + i,
            )])
            .await
            .unwrap();
    }

    let page1 = EvalRubricReadModel::list_by_tenant(&store, &TenantId::new(""), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].rubric_id, "rub_pg_00");
    assert_eq!(page1[1].rubric_id, "rub_pg_01");

    let page2 = EvalRubricReadModel::list_by_tenant(&store, &TenantId::new(""), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].rubric_id, "rub_pg_02");
}
