//! Signal ingestion contract tests (RFC 012).
//!
//! Validates that every ingested signal is durably stored with its full
//! payload intact and correctly scoped to its project.
//!
//! Projection contract:
//!   SignalIngested → SignalRecord { id, project, source, payload, timestamp_ms }
//!
//! list_by_project:
//!   - filtered by full ProjectKey equality
//!   - sorted by timestamp_ms ascending
//!   - supports limit/offset pagination

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, SignalId,
    SignalIngested, TenantId, WorkspaceId,
};
use cairn_store::{projections::SignalReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str, workspace: &str, proj: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
        project_id: ProjectId::new(proj),
    }
}

fn default_project() -> ProjectKey {
    project("t_sig", "w_sig", "p_sig")
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

fn ingest(
    evt_id: &str,
    signal_id: &str,
    proj: ProjectKey,
    source: &str,
    payload: serde_json::Value,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::SignalIngested(SignalIngested {
            project: proj,
            signal_id: SignalId::new(signal_id),
            source: source.to_owned(),
            payload,
            timestamp_ms: ts,
        }),
    )
}

// ── 1. SignalIngested stores record with all fields ───────────────────────────

#[tokio::test]
async fn signal_ingested_stores_all_fields() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let signal_id = SignalId::new("sig_001");

    store
        .append(&[ingest(
            "e1",
            "sig_001",
            default_project(),
            "webhook:github",
            serde_json::json!({ "event": "push", "ref": "refs/heads/main", "commits": 3 }),
            ts,
        )])
        .await
        .unwrap();

    let record = SignalReadModel::get(&store, &signal_id)
        .await
        .unwrap()
        .expect("SignalRecord must exist after SignalIngested");

    assert_eq!(record.id, signal_id);
    assert_eq!(record.project, default_project());
    assert_eq!(record.source, "webhook:github");
    assert_eq!(record.timestamp_ms, ts);
    assert_eq!(record.payload["event"], "push");
    assert_eq!(record.payload["ref"], "refs/heads/main");
    assert_eq!(record.payload["commits"], 3);
}

// ── 2. list_by_project returns only project-scoped signals ────────────────────

#[tokio::test]
async fn list_by_project_returns_only_matching_project_signals() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let proj_a = project("ta", "wa", "pa");
    let proj_b = project("tb", "wb", "pb");

    store
        .append(&[
            ingest(
                "e1",
                "sig_a1",
                proj_a.clone(),
                "src_a",
                serde_json::json!({"n": 1}),
                ts,
            ),
            ingest(
                "e2",
                "sig_a2",
                proj_a.clone(),
                "src_a",
                serde_json::json!({"n": 2}),
                ts + 1,
            ),
            ingest(
                "e3",
                "sig_b1",
                proj_b.clone(),
                "src_b",
                serde_json::json!({"n": 3}),
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    let a_sigs = SignalReadModel::list_by_project(&store, &proj_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(a_sigs.len(), 2, "proj_a has 2 signals");
    assert!(a_sigs.iter().all(|s| s.project == proj_a));
    let a_ids: Vec<_> = a_sigs.iter().map(|s| s.id.as_str()).collect();
    assert!(a_ids.contains(&"sig_a1"));
    assert!(a_ids.contains(&"sig_a2"));
    assert!(
        !a_ids.contains(&"sig_b1"),
        "proj_b signal must not appear in proj_a"
    );

    let b_sigs = SignalReadModel::list_by_project(&store, &proj_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(b_sigs.len(), 1);
    assert_eq!(b_sigs[0].id.as_str(), "sig_b1");

    // Unknown project returns empty.
    let empty = SignalReadModel::list_by_project(&store, &project("tx", "wx", "px"), 10, 0)
        .await
        .unwrap();
    assert!(empty.is_empty());
}

// ── 3. serde_json::Value payload round-trips without data loss ────────────────

#[tokio::test]
async fn payload_round_trips_primitive_types() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let payload = serde_json::json!({
        "string_field":  "hello world",
        "int_field":     42,
        "float_field":   3.14,
        "bool_true":     true,
        "bool_false":    false,
        "null_field":    null
    });

    store
        .append(&[ingest(
            "e1",
            "sig_prim",
            default_project(),
            "test",
            payload.clone(),
            ts,
        )])
        .await
        .unwrap();

    let record = SignalReadModel::get(&store, &SignalId::new("sig_prim"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.payload["string_field"], "hello world");
    assert_eq!(record.payload["int_field"], 42);
    assert!((record.payload["float_field"].as_f64().unwrap() - 3.14).abs() < 1e-9);
    assert_eq!(record.payload["bool_true"], true);
    assert_eq!(record.payload["bool_false"], false);
    assert!(record.payload["null_field"].is_null());
}

#[tokio::test]
async fn payload_round_trips_nested_objects() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let payload = serde_json::json!({
        "repository": {
            "name": "cairn-rs",
            "owner": { "login": "acme-corp" }
        },
        "commits": [
            { "id": "abc123", "message": "fix: patch", "author": "alice" },
            { "id": "def456", "message": "feat: new",  "author": "bob"   }
        ],
        "tags": ["backend", "rust"]
    });

    store
        .append(&[ingest(
            "e1",
            "sig_nested",
            default_project(),
            "github",
            payload.clone(),
            ts,
        )])
        .await
        .unwrap();

    let record = SignalReadModel::get(&store, &SignalId::new("sig_nested"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        record.payload, payload,
        "nested payload must round-trip exactly"
    );
    assert_eq!(record.payload["repository"]["name"], "cairn-rs");
    assert_eq!(record.payload["repository"]["owner"]["login"], "acme-corp");
    assert_eq!(record.payload["commits"].as_array().unwrap().len(), 2);
    assert_eq!(record.payload["commits"][0]["id"], "abc123");
    assert_eq!(record.payload["tags"][1], "rust");
}

#[tokio::test]
async fn empty_payload_object_is_valid() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[ingest(
            "e1",
            "sig_empty",
            default_project(),
            "scheduler",
            serde_json::json!({}),
            ts,
        )])
        .await
        .unwrap();

    let record = SignalReadModel::get(&store, &SignalId::new("sig_empty"))
        .await
        .unwrap()
        .unwrap();
    assert!(record.payload.is_object());
    assert_eq!(
        record.payload.as_object().unwrap().len(),
        0,
        "empty payload object round-trips as empty"
    );
}

#[tokio::test]
async fn array_payload_is_valid() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let payload = serde_json::json!(["item1", "item2", "item3"]);
    store
        .append(&[ingest(
            "e1",
            "sig_arr",
            default_project(),
            "batch",
            payload.clone(),
            ts,
        )])
        .await
        .unwrap();

    let record = SignalReadModel::get(&store, &SignalId::new("sig_arr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.payload, payload);
    assert_eq!(record.payload.as_array().unwrap().len(), 3);
    assert_eq!(record.payload[0], "item1");
}

// ── 4. Timestamp ordering ─────────────────────────────────────────────────────

#[tokio::test]
async fn list_by_project_sorted_by_timestamp_ms_ascending() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Append in reverse timestamp order to prove sort is not insertion-order.
    store
        .append(&[
            ingest(
                "e1",
                "sig_ts3",
                default_project(),
                "s",
                serde_json::json!({"i":3}),
                ts + 200,
            ),
            ingest(
                "e2",
                "sig_ts1",
                default_project(),
                "s",
                serde_json::json!({"i":1}),
                ts,
            ),
            ingest(
                "e3",
                "sig_ts2",
                default_project(),
                "s",
                serde_json::json!({"i":2}),
                ts + 100,
            ),
        ])
        .await
        .unwrap();

    let signals = SignalReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();

    assert_eq!(signals.len(), 3);
    assert_eq!(
        signals[0].id.as_str(),
        "sig_ts1",
        "earliest timestamp first"
    );
    assert_eq!(signals[1].id.as_str(), "sig_ts2");
    assert_eq!(signals[2].id.as_str(), "sig_ts3", "latest timestamp last");

    // Timestamps are strictly ascending.
    for window in signals.windows(2) {
        assert!(
            window[0].timestamp_ms < window[1].timestamp_ms,
            "timestamps must be strictly ascending in sorted result"
        );
    }
}

#[tokio::test]
async fn signals_with_same_timestamp_all_returned() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Batch event — all signals share the exact same timestamp.
    store
        .append(&[
            ingest(
                "e1",
                "sig_same_1",
                default_project(),
                "batch",
                serde_json::json!({"seq":1}),
                ts,
            ),
            ingest(
                "e2",
                "sig_same_2",
                default_project(),
                "batch",
                serde_json::json!({"seq":2}),
                ts,
            ),
            ingest(
                "e3",
                "sig_same_3",
                default_project(),
                "batch",
                serde_json::json!({"seq":3}),
                ts,
            ),
        ])
        .await
        .unwrap();

    let signals = SignalReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        signals.len(),
        3,
        "all 3 signals with same timestamp are returned"
    );
}

// ── 5. Pagination on large signal sets ────────────────────────────────────────

#[tokio::test]
async fn pagination_on_large_signal_set() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Ingest 10 signals with sequential timestamps.
    for i in 0u32..10 {
        store
            .append(&[ingest(
                &format!("e{i:02}"),
                &format!("sig_pg_{i:02}"),
                default_project(),
                "paginator",
                serde_json::json!({ "seq": i }),
                ts + i as u64 * 10,
            )])
            .await
            .unwrap();
    }

    // First page of 3.
    let page1 = SignalReadModel::list_by_project(&store, &default_project(), 3, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);
    assert_eq!(page1[0].id.as_str(), "sig_pg_00");
    assert_eq!(page1[2].id.as_str(), "sig_pg_02");

    // Second page of 3.
    let page2 = SignalReadModel::list_by_project(&store, &default_project(), 3, 3)
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].id.as_str(), "sig_pg_03");
    assert_eq!(page2[2].id.as_str(), "sig_pg_05");

    // Third page of 3.
    let page3 = SignalReadModel::list_by_project(&store, &default_project(), 3, 6)
        .await
        .unwrap();
    assert_eq!(page3.len(), 3);
    assert_eq!(page3[0].id.as_str(), "sig_pg_06");

    // Last page (partial — only 1 signal left).
    let page4 = SignalReadModel::list_by_project(&store, &default_project(), 3, 9)
        .await
        .unwrap();
    assert_eq!(page4.len(), 1);
    assert_eq!(page4[0].id.as_str(), "sig_pg_09");

    // Offset beyond end returns empty.
    let past_end = SignalReadModel::list_by_project(&store, &default_project(), 3, 10)
        .await
        .unwrap();
    assert!(past_end.is_empty(), "offset past end returns empty");
}

// ── 6. Source string variety ──────────────────────────────────────────────────

#[tokio::test]
async fn source_field_accepts_various_formats() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let sources = [
        ("sig_src_1", "webhook:github"),
        ("sig_src_2", "schedule:daily"),
        ("sig_src_3", "external:crm"),
        ("sig_src_4", "internal:monitor"),
    ];

    for (i, (sig_id, source)) in sources.iter().enumerate() {
        store
            .append(&[ingest(
                &format!("e{i}"),
                sig_id,
                default_project(),
                source,
                serde_json::json!({}),
                ts + i as u64,
            )])
            .await
            .unwrap();
    }

    for (sig_id, expected_source) in &sources {
        let record = SignalReadModel::get(&store, &SignalId::new(*sig_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.source, *expected_source,
            "source field must round-trip for {sig_id}"
        );
    }
}

// ── 7. get() returns None for unknown signal ──────────────────────────────────

#[tokio::test]
async fn get_returns_none_for_unknown_signal() {
    let store = InMemoryStore::new();
    let result = SignalReadModel::get(&store, &SignalId::new("ghost_signal"))
        .await
        .unwrap();
    assert!(result.is_none());
}
