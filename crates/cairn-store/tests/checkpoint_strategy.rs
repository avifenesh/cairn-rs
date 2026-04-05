//! Checkpoint strategy lifecycle tests (RFC 002).
//!
//! Validates the checkpoint strategy pipeline: setting a strategy for a run,
//! updating it (upsert semantics), cross-run isolation, and the checkpoint
//! record dispositions (Latest/Superseded) that strategies govern.
//!
//! CheckpointStrategyReadModel provides:
//!   get_by_run(run_id) — returns the current strategy for a run (if set)
//!
//! Note: the trait has no list method; "strategies in order" is verified via
//! sequential get_by_run calls after each update.
//!
//! Note on event fields: CheckpointStrategySet carries strategy_id, description,
//! set_at_ms, and optional run_id. Fields like interval_ms and max_checkpoints
//! are on the CheckpointStrategy struct but not the event; they default to 0/10.

use cairn_domain::{
    CheckpointId, CheckpointRecorded, CheckpointRestored, CheckpointStrategySet, EventEnvelope,
    EventId, EventSource, ProjectId, ProjectKey, RunId, RuntimeEvent, SessionCreated, SessionId,
    TenantId, WorkspaceId,
};
use cairn_domain::lifecycle::CheckpointDisposition;
use cairn_store::{
    projections::{CheckpointReadModel, CheckpointStrategyReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id:    TenantId::new("t_cpstrat"),
        workspace_id: WorkspaceId::new("w_cpstrat"),
        project_id:   ProjectId::new("p_cpstrat"),
    }
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

fn set_strategy(
    evt_id:      &str,
    strategy_id: &str,
    run_id:      &str,
    description: &str,
    ts:          u64,
) -> EventEnvelope<RuntimeEvent> {
    // CheckpointStrategySet has no project field (system-level event).
    // Use a raw envelope since for_runtime_event requires a project.
    use cairn_domain::{OwnershipKey, tenancy::ProjectKey as PK};
    EventEnvelope {
        event_id:       EventId::new(evt_id),
        source:         EventSource::Runtime,
        ownership:      OwnershipKey::System,
        causation_id:   None,
        correlation_id: None,
        payload: RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
            strategy_id:  strategy_id.to_owned(),
            description:  description.to_owned(),
            set_at_ms:    ts,
            run_id:       Some(RunId::new(run_id)),
        }),
    }
}

fn record_checkpoint(
    evt_id:  &str,
    run_id:  &str,
    cp_id:   &str,
    disposition: CheckpointDisposition,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
        project:        project(),
        run_id:         RunId::new(run_id),
        checkpoint_id:  CheckpointId::new(cp_id),
        disposition,
        data:           Some(serde_json::json!({ "step": cp_id })),
    }))
}

// ── 1. CheckpointStrategySet stores the record ────────────────────────────────

#[tokio::test]
async fn checkpoint_strategy_set_stores_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[set_strategy("e1", "strat_001", "run_strat_1", "interval=60s", ts)])
        .await.unwrap();

    let strategy = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new("run_strat_1"))
        .await.unwrap()
        .expect("CheckpointStrategy must exist after CheckpointStrategySet");

    assert_eq!(strategy.strategy_id, "strat_001");
    assert_eq!(strategy.run_id.as_str(), "run_strat_1");
}

// ── 2. Strategy with no run_id is not stored ──────────────────────────────────

#[tokio::test]
async fn strategy_without_run_id_is_not_stored() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Event without run_id — cannot index by run.
    use cairn_domain::OwnershipKey;
    store.append(&[EventEnvelope {
        event_id:       EventId::new("e1"),
        source:         EventSource::Runtime,
        ownership:      OwnershipKey::System,
        causation_id:   None,
        correlation_id: None,
        payload: RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
            strategy_id:  "strat_no_run".to_owned(),
            description:  "no run".to_owned(),
            set_at_ms:    ts,
            run_id:       None,
        }),
    }]).await.unwrap();

    // Cannot query by run_id since there is none — returns None for any run.
    let none = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new("any_run"))
        .await.unwrap();
    assert!(none.is_none());
}

// ── 3. Strategy updates replace the previous (upsert by run_id) ───────────────

#[tokio::test]
async fn strategy_update_replaces_previous() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = "run_update";

    // Initial strategy.
    store.append(&[set_strategy("e1", "strat_v1", run_id, "version 1", ts)])
        .await.unwrap();
    let v1 = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new(run_id))
        .await.unwrap().unwrap();
    assert_eq!(v1.strategy_id, "strat_v1");

    // Updated strategy.
    store.append(&[set_strategy("e2", "strat_v2", run_id, "version 2", ts + 1_000)])
        .await.unwrap();
    let v2 = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new(run_id))
        .await.unwrap().unwrap();
    assert_eq!(v2.strategy_id, "strat_v2",
        "second CheckpointStrategySet must replace the first");

    // Third update.
    store.append(&[set_strategy("e3", "strat_v3", run_id, "version 3", ts + 2_000)])
        .await.unwrap();
    let v3 = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new(run_id))
        .await.unwrap().unwrap();
    assert_eq!(v3.strategy_id, "strat_v3");
}

// ── 4. Sequential strategy updates track progression ─────────────────────────

#[tokio::test]
async fn sequential_strategy_updates_progress_correctly() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = "run_seq";

    let names = ["alpha", "beta", "gamma", "delta", "epsilon"];

    for (i, name) in names.iter().enumerate() {
        store.append(&[set_strategy(
            &format!("e{i}"), name, run_id,
            &format!("strategy {name}"), ts + i as u64 * 500,
        )]).await.unwrap();

        // After each update, verify the current strategy is the latest.
        let current = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new(run_id))
            .await.unwrap().unwrap();
        assert_eq!(current.strategy_id, *name,
            "after setting {name}: get_by_run must return {name}");
    }

    // Final state is the last set strategy.
    let final_strat = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new(run_id))
        .await.unwrap().unwrap();
    assert_eq!(final_strat.strategy_id, "epsilon");
}

// ── 5. CheckpointDisposition::Latest persists ─────────────────────────────────

#[tokio::test]
async fn checkpoint_latest_disposition_persists() {
    let store = InMemoryStore::new();

    // First checkpoint in a run starts as Latest.
    store.append(&[
        evt("e_s", RuntimeEvent::SessionCreated(SessionCreated {
            project: project(), session_id: SessionId::new("sess_disp"),
        })),
        record_checkpoint("e1", "run_disp", "cp_1", CheckpointDisposition::Latest),
    ]).await.unwrap();

    let cp = CheckpointReadModel::get(&store, &CheckpointId::new("cp_1"))
        .await.unwrap().unwrap();
    assert_eq!(cp.disposition, CheckpointDisposition::Latest);
    assert_eq!(cp.run_id.as_str(), "run_disp");
}

// ── 6. CheckpointDisposition::Superseded when a new Latest is set ─────────────

#[tokio::test]
async fn second_latest_supersedes_first() {
    let store = InMemoryStore::new();

    store.append(&[
        evt("e_s", RuntimeEvent::SessionCreated(SessionCreated {
            project: project(), session_id: SessionId::new("sess_sup"),
        })),
        record_checkpoint("e1", "run_sup", "cp_first",  CheckpointDisposition::Latest),
    ]).await.unwrap();

    // First is Latest.
    let cp1_before = CheckpointReadModel::get(&store, &CheckpointId::new("cp_first"))
        .await.unwrap().unwrap();
    assert_eq!(cp1_before.disposition, CheckpointDisposition::Latest);

    // Append a second Latest for the same run — first becomes Superseded.
    store.append(&[
        record_checkpoint("e2", "run_sup", "cp_second", CheckpointDisposition::Latest),
    ]).await.unwrap();

    let cp1_after = CheckpointReadModel::get(&store, &CheckpointId::new("cp_first"))
        .await.unwrap().unwrap();
    assert_eq!(cp1_after.disposition, CheckpointDisposition::Superseded,
        "first checkpoint must be Superseded when a newer Latest is recorded");

    let cp2 = CheckpointReadModel::get(&store, &CheckpointId::new("cp_second"))
        .await.unwrap().unwrap();
    assert_eq!(cp2.disposition, CheckpointDisposition::Latest,
        "second checkpoint must remain Latest");
}

// ── 7. Both dispositions can coexist for the same run ─────────────────────────

#[tokio::test]
async fn latest_and_superseded_coexist_in_same_run() {
    let store = InMemoryStore::new();

    store.append(&[
        evt("e_s", RuntimeEvent::SessionCreated(SessionCreated {
            project: project(), session_id: SessionId::new("sess_coex"),
        })),
        record_checkpoint("e1", "run_coex", "cp_a", CheckpointDisposition::Latest),
        record_checkpoint("e2", "run_coex", "cp_b", CheckpointDisposition::Latest),
        record_checkpoint("e3", "run_coex", "cp_c", CheckpointDisposition::Latest),
    ]).await.unwrap();

    let cp_a = CheckpointReadModel::get(&store, &CheckpointId::new("cp_a")).await.unwrap().unwrap();
    let cp_b = CheckpointReadModel::get(&store, &CheckpointId::new("cp_b")).await.unwrap().unwrap();
    let cp_c = CheckpointReadModel::get(&store, &CheckpointId::new("cp_c")).await.unwrap().unwrap();

    assert_eq!(cp_a.disposition, CheckpointDisposition::Superseded);
    assert_eq!(cp_b.disposition, CheckpointDisposition::Superseded);
    assert_eq!(cp_c.disposition, CheckpointDisposition::Latest, "only the last is Latest");
}

// ── 8. Cross-run isolation: strategies don't bleed across runs ────────────────

#[tokio::test]
async fn cross_run_strategy_isolation() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[
        set_strategy("e1", "strat_run_a", "run_iso_a", "for run A", ts),
        set_strategy("e2", "strat_run_b", "run_iso_b", "for run B", ts + 1),
    ]).await.unwrap();

    let strat_a = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new("run_iso_a"))
        .await.unwrap().unwrap();
    assert_eq!(strat_a.strategy_id, "strat_run_a");
    assert_eq!(strat_a.run_id.as_str(), "run_iso_a");

    let strat_b = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new("run_iso_b"))
        .await.unwrap().unwrap();
    assert_eq!(strat_b.strategy_id, "strat_run_b");
    assert_eq!(strat_b.run_id.as_str(), "run_iso_b");

    // run_a's strategy doesn't leak into run_b and vice versa.
    assert_ne!(strat_a.strategy_id, strat_b.strategy_id);

    // Run with no strategy returns None.
    let none = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new("run_no_strategy"))
        .await.unwrap();
    assert!(none.is_none());
}

// ── 9. Checkpoint RestoreEvent changes nothing on the strategy ────────────────

#[tokio::test]
async fn checkpoint_restored_does_not_affect_strategy() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[
        evt("e_s", RuntimeEvent::SessionCreated(SessionCreated {
            project: project(), session_id: SessionId::new("sess_restore"),
        })),
        record_checkpoint("e1", "run_restore", "cp_restore", CheckpointDisposition::Latest),
        set_strategy("e2", "strat_restore", "run_restore", "restore test", ts),
    ]).await.unwrap();

    // Restoring a checkpoint.
    store.append(&[evt("e3", RuntimeEvent::CheckpointRestored(CheckpointRestored {
        project:       project(),
        run_id:        RunId::new("run_restore"),
        checkpoint_id: CheckpointId::new("cp_restore"),
    }))]).await.unwrap();

    // Strategy is unchanged after restore.
    let strat = CheckpointStrategyReadModel::get_by_run(&store, &RunId::new("run_restore"))
        .await.unwrap().unwrap();
    assert_eq!(strat.strategy_id, "strat_restore",
        "strategy must not change after CheckpointRestored");
}
