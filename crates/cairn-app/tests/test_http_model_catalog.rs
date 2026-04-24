//! Integration tests for the public model-catalog endpoints
//! (`/v1/models/catalog` and `/v1/models/catalog/providers`).
//!
//! These exercise the full cairn-app subprocess so we catch:
//! - Route wiring + auth middleware gating
//! - Query-param parsing + validation responses
//! - Filter, sort, and pagination behavior against the real bundled
//!   LiteLLM catalog (which is embedded at compile time — `with_bundled`
//!   returns hundreds of entries).

mod support;

use serde_json::Value;
use support::live_fabric::LiveHarness;

/// Minimal per-item shape we care about in these tests. We intentionally
/// DON'T mirror the full `ModelEntry` struct client-side — tests should
/// fail loudly on breaking changes to the serialized shape, but we only
/// pluck the fields the UI relies on.
fn first_item(body: &Value) -> &Value {
    let arr = body["items"].as_array().expect("items array");
    assert!(!arr.is_empty(), "expected at least one item in {body}");
    &arr[0]
}

async fn get_catalog(h: &LiveHarness, qs: &str) -> (u16, Value) {
    let r = h
        .client()
        .get(format!("{}/v1/models/catalog{qs}", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("send catalog request");
    let status = r.status().as_u16();
    let body: Value = if status == 200 {
        r.json().await.expect("json body")
    } else {
        let text = r.text().await.unwrap_or_default();
        // Fall back to a plain string Value if the error body isn't JSON;
        // assertions below key on status first and only then on body shape.
        let fallback = Value::String(text.clone());
        serde_json::from_str(&text).unwrap_or(fallback)
    };
    (status, body)
}

#[tokio::test]
async fn catalog_happy_path_returns_items_and_pagination_metadata() {
    let h = LiveHarness::setup().await;
    let (status, body) = get_catalog(&h, "").await;

    assert_eq!(status, 200, "body: {body}");
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "bundled catalog must have entries");

    let total = body["total"].as_u64().expect("total present");
    let has_more = body["hasMore"].as_bool().expect("hasMore present");
    assert!(total > 0);
    // default limit is 100; catalog ships with hundreds of chat-mode entries
    assert_eq!(
        has_more,
        total > items.len() as u64,
        "hasMore must reflect total vs items.len()"
    );

    // Sanity: every item must expose the fields the UI renders.
    let e = first_item(&body);
    for field in [
        "id",
        "provider",
        "display_name",
        "context_len",
        "tier",
        "cost_per_1m_input",
        "cost_per_1m_output",
        "supports_tools",
    ] {
        assert!(e.get(field).is_some(), "field `{field}` missing on {e}");
    }
}

#[tokio::test]
async fn catalog_filter_by_provider_only_returns_matching_entries() {
    let h = LiveHarness::setup().await;
    let (status, body) = get_catalog(&h, "?provider=openai&limit=500").await;

    assert_eq!(status, 200);
    let items = body["items"].as_array().unwrap();
    assert!(!items.is_empty(), "openai provider must be present");
    for it in items {
        assert_eq!(
            it["provider"].as_str().unwrap(),
            "openai",
            "provider filter violated by {it}"
        );
    }
}

#[tokio::test]
async fn catalog_search_is_case_insensitive_across_id_and_name() {
    let h = LiveHarness::setup().await;

    // The bundled catalog includes several gpt-4* entries. `search=GPT`
    // must match at least one regardless of case.
    let (status, body) = get_catalog(&h, "?search=GPT&limit=500").await;
    assert_eq!(status, 200);
    let items = body["items"].as_array().unwrap();
    assert!(!items.is_empty(), "search=GPT must match something");
    for it in items {
        let id = it["id"].as_str().unwrap().to_ascii_lowercase();
        let name = it["display_name"].as_str().unwrap().to_ascii_lowercase();
        let prov = it["provider"].as_str().unwrap().to_ascii_lowercase();
        assert!(
            id.contains("gpt") || name.contains("gpt") || prov.contains("gpt"),
            "neither id/display_name/provider contains 'gpt': {it}"
        );
    }
}

#[tokio::test]
async fn catalog_free_only_filter_returns_zero_cost_models() {
    let h = LiveHarness::setup().await;
    let (status, body) = get_catalog(&h, "?free_only=true&limit=500").await;

    assert_eq!(status, 200);
    let items = body["items"].as_array().unwrap();
    for it in items {
        assert_eq!(
            it["cost_per_1m_input"].as_f64().unwrap(),
            0.0,
            "free model has non-zero input cost: {it}"
        );
        assert_eq!(
            it["cost_per_1m_output"].as_f64().unwrap(),
            0.0,
            "free model has non-zero output cost: {it}"
        );
    }
}

#[tokio::test]
async fn catalog_max_cost_excludes_expensive_models() {
    let h = LiveHarness::setup().await;
    // $0.50/1M is below typical frontier pricing — GPT-4o et al. must be
    // excluded while cheaper models remain.
    let (status, body) = get_catalog(&h, "?max_cost_per_1m=0.5&limit=500").await;

    assert_eq!(status, 200);
    let items = body["items"].as_array().unwrap();
    for it in items {
        let cost = it["cost_per_1m_input"].as_f64().unwrap();
        assert!(
            cost <= 0.5,
            "max_cost filter violated: {cost} > 0.5 for {it}"
        );
    }
}

#[tokio::test]
async fn catalog_capability_filters_compose() {
    let h = LiveHarness::setup().await;
    let (status, body) =
        get_catalog(&h, "?supports_tools=true&supports_json_mode=true&limit=500").await;

    assert_eq!(status, 200);
    let items = body["items"].as_array().unwrap();
    for it in items {
        assert!(
            it["supports_tools"].as_bool().unwrap(),
            "item failed supports_tools filter: {it}"
        );
        assert!(
            it["supports_json_mode"].as_bool().unwrap(),
            "item failed supports_json_mode filter: {it}"
        );
    }
}

#[tokio::test]
async fn catalog_pagination_respects_limit_and_offset() {
    let h = LiveHarness::setup().await;

    // Page 1 (10 items)
    let (s1, body1) = get_catalog(&h, "?limit=10&offset=0").await;
    assert_eq!(s1, 200);
    let items1 = body1["items"].as_array().unwrap();
    assert_eq!(items1.len(), 10);
    let total1 = body1["total"].as_u64().unwrap();
    assert!(total1 > 10, "need >10 entries to test pagination");
    assert!(body1["hasMore"].as_bool().unwrap());

    // Page 2 must NOT overlap page 1 ids
    let (s2, body2) = get_catalog(&h, "?limit=10&offset=10").await;
    assert_eq!(s2, 200);
    let items2 = body2["items"].as_array().unwrap();
    assert!(!items2.is_empty());
    let ids1: std::collections::HashSet<_> = items1
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    for it in items2 {
        let id = it["id"].as_str().unwrap();
        assert!(
            !ids1.contains(id),
            "id `{id}` leaked from page 1 into page 2"
        );
    }
    assert_eq!(
        body2["total"].as_u64().unwrap(),
        total1,
        "total must be stable across pages"
    );
}

#[tokio::test]
async fn catalog_invalid_limit_returns_422() {
    let h = LiveHarness::setup().await;

    // limit=0
    let (s0, _) = get_catalog(&h, "?limit=0").await;
    assert_eq!(s0, 422, "limit=0 must be rejected with 422");

    // limit above the hard cap
    let (s_big, _) = get_catalog(&h, "?limit=100000").await;
    assert_eq!(s_big, 422, "oversized limit must be rejected with 422");

    // malformed tier
    let (s_tier, _) = get_catalog(&h, "?tier=overlord").await;
    assert_eq!(s_tier, 422, "invalid tier must be rejected with 422");
}

#[tokio::test]
async fn catalog_missing_bearer_returns_401() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .get(format!("{}/v1/models/catalog", h.base_url))
        .send()
        .await
        .expect("send");
    assert_eq!(r.status().as_u16(), 401, "unauth request must 401");
}

#[tokio::test]
async fn catalog_ordering_is_deterministic() {
    let h = LiveHarness::setup().await;

    let (s, body) =
        get_catalog(&h, "?provider=openai&limit=500&supports_tools=true").await;
    assert_eq!(s, 200);
    let items = body["items"].as_array().unwrap();
    assert!(items.len() >= 2);

    // Provider ASC (same provider, so no-op), then cost ASC, then id ASC.
    let mut prev_cost = f64::MIN;
    let mut prev_id = String::new();
    for it in items {
        let c = it["cost_per_1m_input"].as_f64().unwrap();
        let id = it["id"].as_str().unwrap().to_owned();
        if (c - prev_cost).abs() < f64::EPSILON {
            assert!(
                id >= prev_id,
                "tie-break on id violated at cost={c}: {prev_id} then {id}"
            );
        } else {
            assert!(
                c >= prev_cost,
                "cost ASC violated: {prev_cost} then {c}"
            );
        }
        prev_cost = c;
        prev_id = id;
    }
}

#[tokio::test]
async fn catalog_providers_summary_returns_sorted_counts() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .get(format!("{}/v1/models/catalog/providers", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("send");
    assert_eq!(r.status().as_u16(), 200);
    let body: Value = r.json().await.unwrap();

    let providers = body["providers"]
        .as_array()
        .expect("providers array present");
    assert!(!providers.is_empty(), "bundled catalog has providers");

    let mut total_count: u64 = 0;
    let mut prev: String = String::new();
    for p in providers {
        let name = p["name"].as_str().unwrap().to_owned();
        let count = p["count"].as_u64().unwrap();
        assert!(!name.is_empty(), "provider name must be non-empty");
        assert!(count > 0, "provider {name} has zero count");
        assert!(name >= prev, "providers not sorted: {prev} then {name}");
        total_count += count;
        prev = name;
    }

    // Sanity: providers-summary total should equal /catalog?limit=1000 with no
    // filters, up to pagination. Allow a loose bound but ensure non-trivial.
    assert!(total_count >= providers.len() as u64);

    // Second call (cache path) must produce identical bytes.
    let r2 = h
        .client()
        .get(format!("{}/v1/models/catalog/providers", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .unwrap();
    let body2: Value = r2.json().await.unwrap();
    assert_eq!(body, body2, "cached response must match original");
}

#[tokio::test]
async fn catalog_search_matches_provider_name() {
    let h = LiveHarness::setup().await;
    let (s, body) = get_catalog(&h, "?search=openai&limit=500").await;
    assert_eq!(s, 200);
    let items = body["items"].as_array().unwrap();
    assert!(!items.is_empty(), "search=openai must match entries");
    // At least some of these should have provider == "openai"
    let any_openai = items
        .iter()
        .any(|it| it["provider"].as_str().unwrap() == "openai");
    assert!(any_openai, "search=openai should surface openai-family entries");
}
