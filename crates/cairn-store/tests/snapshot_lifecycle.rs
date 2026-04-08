//! RFC 002 — Snapshot lifecycle tests.
//!
//! Validates point-in-time snapshot management through the event log and
//! synchronous projection:
//!
//! - `SnapshotCreated` populates `SnapshotReadModel` with the snapshot record.
//! - `list_by_tenant` returns all snapshots in ascending `created_at_ms` order.
//! - `get_latest` returns the most recent snapshot for the tenant.
//! - Multiple snapshots for the same tenant are all stored and ordered correctly.
//! - Cross-tenant isolation: each tenant's snapshots are private.

use cairn_domain::{
    events::SnapshotCreated, tenancy::OwnershipKey, EventEnvelope, EventId, EventSource,
    RuntimeEvent, TenantId,
};
use cairn_store::{projections::SnapshotReadModel, EventLog, InMemoryStore};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tenant_ownership(tenant_id: &str) -> OwnershipKey {
    OwnershipKey::Tenant(cairn_domain::tenancy::TenantKey::new(TenantId::new(
        tenant_id,
    )))
}

async fn create_snapshot(
    store: &InMemoryStore,
    event_id: &str,
    snapshot_id: &str,
    tenant_id: &str,
    event_position: u64,
    created_at_ms: u64,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        tenant_ownership(tenant_id),
        RuntimeEvent::SnapshotCreated(SnapshotCreated {
            snapshot_id: snapshot_id.to_owned(),
            created_at_ms,
            tenant_id: TenantId::new(tenant_id),
            event_position,
        }),
    );
    store.append(&[env]).await.unwrap();
}

// ── 1. SnapshotCreated stores the record ──────────────────────────────────────

#[tokio::test]
async fn snapshot_created_appears_in_read_model() {
    let store = InMemoryStore::new();

    create_snapshot(&store, "e1", "snap_1", "tenant_a", 42, 5_000).await;

    let snapshots = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_a"))
        .await
        .unwrap();

    assert_eq!(snapshots.len(), 1);
    let s = &snapshots[0];
    assert_eq!(s.snapshot_id, "snap_1");
    assert_eq!(s.tenant_id.as_str(), "tenant_a");
    assert_eq!(s.event_position, 42);
    assert_eq!(s.created_at_ms, 5_000);
}

#[tokio::test]
async fn list_by_tenant_returns_empty_for_unknown_tenant() {
    let store = InMemoryStore::new();
    let result = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("ghost"))
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn get_latest_returns_none_for_tenant_with_no_snapshots() {
    let store = InMemoryStore::new();
    let result = SnapshotReadModel::get_latest(&store, &TenantId::new("no_snaps"))
        .await
        .unwrap();
    assert!(result.is_none());
}

// ── 3. Multiple snapshots for same tenant are all stored ─────────────────────

#[tokio::test]
async fn multiple_snapshots_are_all_stored() {
    let store = InMemoryStore::new();

    for (i, at) in [(1u64, 1_000u64), (2, 2_000), (3, 3_000)] {
        create_snapshot(
            &store,
            &format!("e{i}"),
            &format!("snap_{i}"),
            "tenant_b",
            i * 10,
            at,
        )
        .await;
    }

    let snapshots = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_b"))
        .await
        .unwrap();

    assert_eq!(snapshots.len(), 3, "all 3 snapshots must be stored");
}

// ── 4. Snapshots ordered by created_at_ms ────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_returns_snapshots_in_ascending_order() {
    let store = InMemoryStore::new();

    // Append in non-chronological order.
    create_snapshot(&store, "e1", "snap_late", "tenant_ord", 30, 3_000).await;
    create_snapshot(&store, "e2", "snap_early", "tenant_ord", 10, 1_000).await;
    create_snapshot(&store, "e3", "snap_middle", "tenant_ord", 20, 2_000).await;

    let snapshots = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_ord"))
        .await
        .unwrap();

    assert_eq!(snapshots.len(), 3);
    assert_eq!(
        snapshots[0].snapshot_id, "snap_early",
        "earliest must come first"
    );
    assert_eq!(
        snapshots[1].snapshot_id, "snap_middle",
        "middle must come second"
    );
    assert_eq!(
        snapshots[2].snapshot_id, "snap_late",
        "latest must come last"
    );

    // Timestamps must be strictly increasing.
    for w in snapshots.windows(2) {
        assert!(w[0].created_at_ms < w[1].created_at_ms);
    }
}

#[tokio::test]
async fn event_positions_preserved_per_snapshot() {
    let store = InMemoryStore::new();

    create_snapshot(&store, "e1", "snap_pos_a", "tenant_pos", 100, 1_000).await;
    create_snapshot(&store, "e2", "snap_pos_b", "tenant_pos", 250, 2_000).await;

    let snapshots = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_pos"))
        .await
        .unwrap();

    assert_eq!(
        snapshots[0].event_position, 100,
        "first snapshot event_position must be 100"
    );
    assert_eq!(
        snapshots[1].event_position, 250,
        "second snapshot event_position must be 250"
    );
}

// ── 5. get_latest returns the most recent snapshot ────────────────────────────

#[tokio::test]
async fn get_latest_returns_snapshot_with_highest_created_at_ms() {
    let store = InMemoryStore::new();

    create_snapshot(&store, "e1", "snap_v1", "tenant_latest", 10, 1_000).await;
    create_snapshot(&store, "e2", "snap_v2", "tenant_latest", 20, 3_000).await;
    create_snapshot(&store, "e3", "snap_v3", "tenant_latest", 15, 2_000).await;

    let latest = SnapshotReadModel::get_latest(&store, &TenantId::new("tenant_latest"))
        .await
        .unwrap()
        .expect("latest must exist");

    assert_eq!(
        latest.snapshot_id, "snap_v2",
        "snap_v2 has highest created_at_ms (3_000) and must be the latest"
    );
    assert_eq!(latest.created_at_ms, 3_000);
}

#[tokio::test]
async fn get_latest_with_single_snapshot_returns_it() {
    let store = InMemoryStore::new();
    create_snapshot(&store, "e1", "snap_only", "tenant_single", 5, 9_999).await;

    let latest = SnapshotReadModel::get_latest(&store, &TenantId::new("tenant_single"))
        .await
        .unwrap()
        .expect("single snapshot must be returned as latest");

    assert_eq!(latest.snapshot_id, "snap_only");
}

#[tokio::test]
async fn get_latest_advances_with_each_new_snapshot() {
    let store = InMemoryStore::new();

    for (i, at) in [(1u64, 1_000u64), (2, 5_000), (3, 3_000)] {
        create_snapshot(
            &store,
            &format!("e{i}"),
            &format!("s{i}"),
            "tenant_adv",
            i,
            at,
        )
        .await;

        let latest = SnapshotReadModel::get_latest(&store, &TenantId::new("tenant_adv"))
            .await
            .unwrap()
            .unwrap();

        // After each append, the latest must be the one with the max created_at_ms so far.
        let expected_max = [(1, 1_000), (2, 5_000), (3, 5_000)][i as usize - 1].1;
        assert_eq!(
            latest.created_at_ms, expected_max,
            "get_latest must track the highest created_at_ms after {i} appends"
        );
    }
}

// ── 6. Cross-tenant isolation ─────────────────────────────────────────────────

#[tokio::test]
async fn snapshots_are_scoped_to_their_tenant() {
    let store = InMemoryStore::new();

    create_snapshot(&store, "e1", "snap_ta1", "tenant_x", 10, 1_000).await;
    create_snapshot(&store, "e2", "snap_ta2", "tenant_x", 20, 2_000).await;
    create_snapshot(&store, "e3", "snap_tb1", "tenant_y", 30, 3_000).await;

    let x_snaps = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_x"))
        .await
        .unwrap();
    let y_snaps = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_y"))
        .await
        .unwrap();

    assert_eq!(x_snaps.len(), 2, "tenant_x must have 2 snapshots");
    assert_eq!(y_snaps.len(), 1, "tenant_y must have 1 snapshot");

    let x_ids: Vec<&str> = x_snaps.iter().map(|s| s.snapshot_id.as_str()).collect();
    assert!(x_ids.contains(&"snap_ta1") && x_ids.contains(&"snap_ta2"));
    assert_eq!(y_snaps[0].snapshot_id, "snap_tb1");
}

#[tokio::test]
async fn get_latest_scoped_to_requesting_tenant() {
    let store = InMemoryStore::new();

    create_snapshot(&store, "e1", "snap_p", "tenant_p", 1, 1_000).await;
    create_snapshot(&store, "e2", "snap_q", "tenant_q", 2, 9_000).await; // much newer

    // tenant_p should see its own latest, not tenant_q's newer snapshot.
    let p_latest = SnapshotReadModel::get_latest(&store, &TenantId::new("tenant_p"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        p_latest.snapshot_id, "snap_p",
        "tenant_p must see only its own latest snapshot"
    );
    assert_eq!(p_latest.created_at_ms, 1_000);
}

#[tokio::test]
async fn updating_one_tenant_snapshot_does_not_affect_another() {
    let store = InMemoryStore::new();

    create_snapshot(&store, "e1", "snap_m", "tenant_m", 5, 1_000).await;
    create_snapshot(&store, "e2", "snap_n", "tenant_n", 6, 2_000).await;

    // Add second snapshot to tenant_m.
    create_snapshot(&store, "e3", "snap_m2", "tenant_m", 10, 5_000).await;

    let m_count = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_m"))
        .await
        .unwrap()
        .len();
    let n_count = SnapshotReadModel::list_by_tenant(&store, &TenantId::new("tenant_n"))
        .await
        .unwrap()
        .len();

    assert_eq!(m_count, 2, "tenant_m must have 2 snapshots");
    assert_eq!(n_count, 1, "tenant_n must still have only 1 snapshot");
}

// ── 7. Event log completeness ─────────────────────────────────────────────────

#[tokio::test]
async fn snapshot_events_are_written_to_log() {
    let store = InMemoryStore::new();

    create_snapshot(&store, "e1", "snap_log_1", "tenant_log", 100, 1_000).await;
    create_snapshot(&store, "e2", "snap_log_2", "tenant_log", 200, 2_000).await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 2);

    assert!(
        matches!(&all[0].envelope.payload, RuntimeEvent::SnapshotCreated(e)
        if e.snapshot_id == "snap_log_1" && e.event_position == 100)
    );
    assert!(
        matches!(&all[1].envelope.payload, RuntimeEvent::SnapshotCreated(e)
        if e.snapshot_id == "snap_log_2" && e.event_position == 200)
    );
}
