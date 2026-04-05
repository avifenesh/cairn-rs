//! Eval dataset lifecycle tests (RFC 004).
//!
//! Validates that eval datasets and their entries are durably stored via the
//! event-sourced projection and correctly queryable via EvalDatasetReadModel.
//!
//! Note on tenant scoping:
//!   EvalDatasetCreated does not carry a tenant_id (the event schema predates
//!   RFC 004 tenant attribution). The projection stores datasets with an empty
//!   sentinel tenant_id. Dataset isolation is therefore tested by dataset_id,
//!   and list_by_tenant("") returns all datasets for aggregate queries.
//!
//! Note on entry payload:
//!   EvalDatasetEntryAdded carries only entry_id and added_at_ms — no
//!   input/output content. The projection stores a minimal EvalDatasetEntry
//!   with the entry_id as a tag for deduplication and counting.

use cairn_domain::{
    EvalDatasetCreated, EvalDatasetEntryAdded, EventEnvelope, EventId, EventSource,
    RuntimeEvent, TenantId,
};
use cairn_store::{
    projections::EvalDatasetReadModel,
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    // EvalDatasetCreated/EntryAdded have no project key — use runtime source.
    // We construct the envelope directly since for_runtime_event expects a
    // project from the payload, which these events don't carry.
    use cairn_domain::{OwnershipKey, tenancy::ProjectKey, ProjectId, WorkspaceId};
    EventEnvelope {
        event_id:       EventId::new(id),
        source:         EventSource::Runtime,
        ownership:      OwnershipKey::System,
        causation_id:   None,
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

fn create_dataset(evt_id: &str, dataset_id: &str, name: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::EvalDatasetCreated(EvalDatasetCreated {
        dataset_id: dataset_id.to_owned(),
        name:       name.to_owned(),
        created_at_ms: ts,
    }))
}

fn add_entry(evt_id: &str, dataset_id: &str, entry_id: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::EvalDatasetEntryAdded(EvalDatasetEntryAdded {
        dataset_id: dataset_id.to_owned(),
        entry_id:   entry_id.to_owned(),
        added_at_ms: ts,
    }))
}

// ── 1. EvalDatasetCreated stores the record ───────────────────────────────────

#[tokio::test]
async fn eval_dataset_created_stores_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[create_dataset("e1", "ds_001", "QA Benchmark", ts)])
        .await.unwrap();

    let dataset = EvalDatasetReadModel::get_dataset(&store, "ds_001")
        .await.unwrap()
        .expect("EvalDataset must exist after EvalDatasetCreated");

    assert_eq!(dataset.dataset_id, "ds_001");
    assert_eq!(dataset.name, "QA Benchmark");
    assert_eq!(dataset.created_at_ms, ts);
    assert!(dataset.entries.is_empty(), "new dataset has no entries");
}

// ── 2. dataset name and metadata persist ─────────────────────────────────────

#[tokio::test]
async fn dataset_name_persists_correctly() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let long_name = "Comprehensive Prompt Safety Evaluation Dataset v2.1";
    store.append(&[create_dataset("e1", "ds_name", long_name, ts)])
        .await.unwrap();

    let dataset = EvalDatasetReadModel::get_dataset(&store, "ds_name")
        .await.unwrap().unwrap();

    assert_eq!(dataset.name, long_name, "full dataset name must persist");
    assert_eq!(dataset.created_at_ms, ts, "created_at_ms must persist");
}

// ── 3. EvalDatasetEntryAdded increments entry count ──────────────────────────

#[tokio::test]
async fn eval_dataset_entry_added_increments_count() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[create_dataset("e0", "ds_entries", "Entry Test", ts)])
        .await.unwrap();

    // No entries yet.
    let empty = EvalDatasetReadModel::get_dataset(&store, "ds_entries")
        .await.unwrap().unwrap();
    assert_eq!(empty.entries.len(), 0);

    // Add first entry.
    store.append(&[add_entry("e1", "ds_entries", "entry_001", ts + 1)])
        .await.unwrap();
    let after_one = EvalDatasetReadModel::get_dataset(&store, "ds_entries")
        .await.unwrap().unwrap();
    assert_eq!(after_one.entries.len(), 1, "one entry after first append");

    // Add two more.
    store.append(&[
        add_entry("e2", "ds_entries", "entry_002", ts + 2),
        add_entry("e3", "ds_entries", "entry_003", ts + 3),
    ]).await.unwrap();
    let after_three = EvalDatasetReadModel::get_dataset(&store, "ds_entries")
        .await.unwrap().unwrap();
    assert_eq!(after_three.entries.len(), 3, "three entries after three appends");
}

// ── 4. Entry count reflects multiple appends ──────────────────────────────────

#[tokio::test]
async fn entry_count_reflects_all_appended_entries() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[create_dataset("e0", "ds_count", "Count Test", ts)])
        .await.unwrap();

    let n = 10u32;
    for i in 1..=n {
        store.append(&[add_entry(&format!("e{i}"), "ds_count", &format!("entry_{i:03}"), ts + i as u64)])
            .await.unwrap();
    }

    let dataset = EvalDatasetReadModel::get_dataset(&store, "ds_count")
        .await.unwrap().unwrap();
    assert_eq!(dataset.entries.len(), n as usize,
        "entry count must equal the number of EvalDatasetEntryAdded events");
}

// ── 5. Duplicate entry_ids are deduplicated ───────────────────────────────────

#[tokio::test]
async fn duplicate_entry_ids_are_deduplicated() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[create_dataset("e0", "ds_dedup", "Dedup Test", ts)])
        .await.unwrap();

    // Append the same entry_id twice (idempotency).
    store.append(&[
        add_entry("e1", "ds_dedup", "entry_dup", ts + 1),
        add_entry("e2", "ds_dedup", "entry_dup", ts + 2), // duplicate
    ]).await.unwrap();

    let dataset = EvalDatasetReadModel::get_dataset(&store, "ds_dedup")
        .await.unwrap().unwrap();
    assert_eq!(dataset.entries.len(), 1,
        "duplicate entry_id must be deduplicated");
}

// ── 6. get_dataset returns None for unknown dataset ───────────────────────────

#[tokio::test]
async fn get_dataset_returns_none_for_unknown_id() {
    let store = InMemoryStore::new();
    let result = EvalDatasetReadModel::get_dataset(&store, "nonexistent")
        .await.unwrap();
    assert!(result.is_none());
}

// ── 7. Multiple datasets tracked independently ────────────────────────────────

#[tokio::test]
async fn multiple_datasets_tracked_independently() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[
        create_dataset("e1", "ds_a", "Dataset A", ts),
        create_dataset("e2", "ds_b", "Dataset B", ts + 1),
    ]).await.unwrap();

    // Add entries only to ds_a.
    store.append(&[
        add_entry("e3", "ds_a", "a_entry_1", ts + 2),
        add_entry("e4", "ds_a", "a_entry_2", ts + 3),
    ]).await.unwrap();

    let ds_a = EvalDatasetReadModel::get_dataset(&store, "ds_a").await.unwrap().unwrap();
    let ds_b = EvalDatasetReadModel::get_dataset(&store, "ds_b").await.unwrap().unwrap();

    assert_eq!(ds_a.name, "Dataset A");
    assert_eq!(ds_a.entries.len(), 2, "ds_a has 2 entries");

    assert_eq!(ds_b.name, "Dataset B");
    assert_eq!(ds_b.entries.len(), 0, "ds_b has no entries — they stay independent");
}

// ── 8. Dataset scoping: entries don't bleed across datasets ──────────────────

#[tokio::test]
async fn entries_are_scoped_to_their_dataset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[
        create_dataset("e1", "ds_scope_1", "Scope 1", ts),
        create_dataset("e2", "ds_scope_2", "Scope 2", ts + 1),
        add_entry("e3", "ds_scope_1", "entry_s1", ts + 2),
        add_entry("e4", "ds_scope_2", "entry_s2", ts + 3),
        add_entry("e5", "ds_scope_2", "entry_s3", ts + 4),
    ]).await.unwrap();

    let s1 = EvalDatasetReadModel::get_dataset(&store, "ds_scope_1").await.unwrap().unwrap();
    let s2 = EvalDatasetReadModel::get_dataset(&store, "ds_scope_2").await.unwrap().unwrap();

    assert_eq!(s1.entries.len(), 1, "scope_1 has 1 entry");
    assert_eq!(s2.entries.len(), 2, "scope_2 has 2 entries");

    // Entries from s2 don't appear in s1.
    assert!(!s1.entries.iter().any(|e| e.tags.contains(&"entry_s2".to_owned())));
    assert!(!s1.entries.iter().any(|e| e.tags.contains(&"entry_s3".to_owned())));
}

// ── 9. list_by_tenant returns all datasets (sentinel tenant) ──────────────────

#[tokio::test]
async fn list_by_tenant_returns_datasets_sorted_by_created_at() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Appended out-of-order timestamps.
    store.append(&[
        create_dataset("e1", "ds_lt_c", "Third",  ts + 200),
        create_dataset("e2", "ds_lt_a", "First",  ts),
        create_dataset("e3", "ds_lt_b", "Second", ts + 100),
    ]).await.unwrap();

    // The sentinel tenant_id used by the projection is "".
    let datasets = EvalDatasetReadModel::list_by_tenant(
        &store, &TenantId::new(""), 10, 0,
    ).await.unwrap();

    assert_eq!(datasets.len(), 3);
    // Sorted by created_at_ms ascending.
    assert_eq!(datasets[0].dataset_id, "ds_lt_a", "earliest first");
    assert_eq!(datasets[1].dataset_id, "ds_lt_b");
    assert_eq!(datasets[2].dataset_id, "ds_lt_c", "latest last");
}

// ── 10. list_by_tenant pagination ─────────────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_respects_limit_and_offset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u64..4 {
        store.append(&[create_dataset(
            &format!("e{i}"),
            &format!("ds_pg_{i:02}"),
            &format!("Dataset {i}"),
            ts + i * 10,
        )]).await.unwrap();
    }

    let page1 = EvalDatasetReadModel::list_by_tenant(&store, &TenantId::new(""), 2, 0)
        .await.unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].dataset_id, "ds_pg_00");
    assert_eq!(page1[1].dataset_id, "ds_pg_01");

    let page2 = EvalDatasetReadModel::list_by_tenant(&store, &TenantId::new(""), 2, 2)
        .await.unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].dataset_id, "ds_pg_02");
    assert_eq!(page2[1].dataset_id, "ds_pg_03");
}
