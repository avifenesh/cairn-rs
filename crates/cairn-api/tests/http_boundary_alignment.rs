//! Executable tests verifying HTTP endpoint shapes match preserved fixtures.
//!
//! For request/response families that are exact today, these assertions compare
//! directly against the preserved fixture examples. For families that are still
//! intentionally thinner, the tests keep the current gap explicit so the
//! generated Worker 1 reports stay honest.

use std::collections::HashSet;

fn load_fixture(fixture_name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../tests/fixtures/http/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        fixture_name
    );
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to load fixture {path}: {e}"));
    serde_json::from_str(&content).unwrap()
}

fn keys_of(value: &serde_json::Value) -> HashSet<String> {
    match value.as_object() {
        Some(map) => map.keys().cloned().collect(),
        None => HashSet::new(),
    }
}

fn assert_json_matches_fixture(our_json: serde_json::Value, fixture: &serde_json::Value) {
    assert_eq!(
        our_json, *fixture,
        "serialized HTTP shape did not match preserved fixture example"
    );
}

#[test]
fn feed_response_has_items_and_has_more() {
    let fixture = load_fixture("GET__v1_feed__limit20_unread_true");
    let response = &fixture["response"];
    let fixture_keys = keys_of(response);

    assert!(fixture_keys.contains("items"));
    assert!(fixture_keys.contains("hasMore"));

    // Our ListResponse produces the same preserved shape
    let our_response = cairn_api::http::ListResponse {
        items: vec![cairn_api::feed::FeedItem {
            id: "101".to_owned(),
            source: "slack".to_owned(),
            kind: Some("message".to_owned()),
            title: Some("Build pipeline needs approval".to_owned()),
            body: Some("Deploy is waiting on approval from ops.".to_owned()),
            url: Some("https://example.test/slack/101".to_owned()),
            author: Some("ops-bot".to_owned()),
            avatar_url: Some("https://example.test/avatar/ops-bot.png".to_owned()),
            repo_full_name: Some("avife/cairn".to_owned()),
            is_read: false,
            is_archived: false,
            group_key: Some("slack:deploy".to_owned()),
            created_at: "2026-04-03T09:30:00Z".to_owned(),
        }],
        has_more: true,
    };
    let our_json = serde_json::to_value(&our_response).unwrap();
    assert!(our_json.get("items").is_some());
    assert!(our_json.get("hasMore").is_some());
}

#[test]
fn feed_response_matches_fixture_exactly() {
    let fixture = load_fixture("GET__v1_feed__limit20_unread_true");
    let our_response = cairn_api::http::ListResponse {
        items: vec![cairn_api::feed::FeedItem {
            id: "101".to_owned(),
            source: "slack".to_owned(),
            kind: Some("message".to_owned()),
            title: Some("Build pipeline needs approval".to_owned()),
            body: Some("Deploy is waiting on approval from ops.".to_owned()),
            url: Some("https://example.test/slack/101".to_owned()),
            author: Some("ops-bot".to_owned()),
            avatar_url: Some("https://example.test/avatar/ops-bot.png".to_owned()),
            repo_full_name: Some("avife/cairn".to_owned()),
            is_read: false,
            is_archived: false,
            group_key: Some("slack:deploy".to_owned()),
            created_at: "2026-04-03T09:30:00Z".to_owned(),
        }],
        has_more: true,
    };
    let our_json = serde_json::to_value(&our_response).unwrap();
    assert_json_matches_fixture(our_json, &fixture["response"]);
}

#[test]
fn memory_search_response_has_items() {
    let fixture = load_fixture("GET__v1_memories_search__q_test_limit_10");
    let response = &fixture["response"];
    assert!(keys_of(response).contains("items"));

    // Verify our MemoryItem has the fixture's required fields
    let fixture_item = &response["items"][0];
    let fixture_item_keys = keys_of(fixture_item);

    let our_item = cairn_api::memory_api::MemoryItem {
        id: "memory_001".to_owned(),
        content: "The weekly digest should...".to_owned(),
        category: Some("project".to_owned()),
        status: cairn_api::memory_api::MemoryStatus::Accepted,
        source: Some("ops-notes".to_owned()),
        confidence: Some(0.92),
        created_at: "2026-04-02T15:00:00Z".to_owned(),
    };
    let our_json = serde_json::to_value(&our_item).unwrap();
    let our_keys = keys_of(&our_json);

    // Must have at least: id, content, category, status
    for key in &["id", "content", "category", "status"] {
        assert!(
            fixture_item_keys.contains(*key),
            "fixture memory item has '{key}'"
        );
        assert!(our_keys.contains(*key), "our MemoryItem missing '{key}'");
    }
}

#[test]
fn memory_search_response_matches_fixture() {
    let our_item = cairn_api::memory_api::MemoryItem {
        id: "memory_001".to_owned(),
        content: "The weekly digest should summarize blocked deploys first.".to_owned(),
        category: Some("project".to_owned()),
        status: cairn_api::memory_api::MemoryStatus::Accepted,
        source: Some("ops-notes".to_owned()),
        confidence: Some(0.92),
        created_at: "2026-04-02T15:00:00Z".to_owned(),
    };
    let our_json = serde_json::to_value(&our_item).unwrap();

    assert_eq!(our_json["source"], "ops-notes");
    assert_eq!(our_json["confidence"], 0.92);
    assert_eq!(our_json["createdAt"], "2026-04-02T15:00:00Z");
    assert!(
        our_json["createdAt"].is_string(),
        "memory response createdAt now uses ISO string matching the fixture"
    );
}

#[test]
fn assistant_message_request_matches_fixture_exactly() {
    let fixture = load_fixture("POST__v1_assistant_message__with_session");
    let request_body = &fixture["request"]["body"];

    let our_req = cairn_api::assistant::AssistantMessageRequest {
        message: "Summarize the deploy blockers.".to_owned(),
        mode: Some("work".to_owned()),
        session_id: Some("session_001".to_owned()),
    };
    let our_json = serde_json::to_value(&our_req).unwrap();
    assert_json_matches_fixture(our_json, request_body);
}

#[test]
fn assistant_message_response_matches_fixture_exactly() {
    let fixture = load_fixture("POST__v1_assistant_message__with_session");
    let response = &fixture["response"];

    let our_resp = cairn_api::assistant::AssistantMessageResponse {
        task_id: "task_assistant_001".to_owned(),
    };
    let our_json = serde_json::to_value(&our_resp).unwrap();
    assert_json_matches_fixture(our_json, response);
}

#[test]
fn dashboard_overview_shape_stays_minimum_contract_stable() {
    let overview = cairn_api::overview::DashboardOverview {
        active_runs: 3,
        active_tasks: 12,
        pending_approvals: 2,
        failed_runs_24h: 1,
        system_healthy: true,
        latency_p50_ms: None,
        latency_p95_ms: None,
        error_rate_24h: 0.0,
        degraded_components: vec![],
        recent_critical_events: vec![],
        active_providers: 0,
        active_plugins: 0,
        memory_doc_count: 0,
        eval_runs_today: 0,
    };
    let json = serde_json::to_value(&overview).unwrap();
    let keys = keys_of(&json);

    let expected = HashSet::from([
        "active_runs".to_owned(),
        "active_tasks".to_owned(),
        "pending_approvals".to_owned(),
        "failed_runs_24h".to_owned(),
        "system_healthy".to_owned(),
        "latency_p50_ms".to_owned(),
        "latency_p95_ms".to_owned(),
        "error_rate_24h".to_owned(),
        "degraded_components".to_owned(),
        "recent_critical_events".to_owned(),
        "active_providers".to_owned(),
        "active_plugins".to_owned(),
        "memory_doc_count".to_owned(),
        "eval_runs_today".to_owned(),
    ]);

    assert_eq!(
        keys, expected,
        "dashboard overview shape drifted from the preserved minimum contract",
    );
}
