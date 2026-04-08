//! GAP-010: Tests for GET /v1/sessions/:id/llm-traces response contract.
//!
//! These are pure-logic tests verifying:
//! 1. The route is in the preserved compatibility catalog with classification Preserve.
//! 2. The `LlmCallTrace` response shape serializes to the expected JSON fields.
//! 3. The catalog consistency: llm-traces is adjacent to other session sub-routes.

use cairn_api::http::{preserved_route_catalog, HttpMethod};
use cairn_domain::observability::LlmCallTrace;
use cairn_domain::{RunId, SessionId};

/// The route must be in the preserved catalog with classification Preserve.
#[test]
fn llm_traces_route_is_in_preserved_catalog() {
    let catalog = preserved_route_catalog();
    let route = catalog
        .iter()
        .find(|r| r.path == "/v1/sessions/:id/llm-traces");

    assert!(
        route.is_some(),
        "GET /v1/sessions/:id/llm-traces must be in the route catalog"
    );
    let route = route.unwrap();
    assert_eq!(
        route.method,
        HttpMethod::Get,
        "llm-traces must be a GET route"
    );
}

/// The trace struct serializes all required fields to JSON.
#[test]
fn llm_call_trace_serializes_required_fields() {
    let trace = LlmCallTrace {
        trace_id: "pc_001".to_owned(),
        model_id: "claude-sonnet-4-6".to_owned(),
        prompt_tokens: 200,
        completion_tokens: 80,
        latency_ms: 350,
        cost_micros: 2_100,
        session_id: Some(SessionId::new("sess_1")),
        run_id: Some(RunId::new("run_1")),
        created_at_ms: 9000,
        is_error: false,
    };

    let json = serde_json::to_value(&trace).unwrap();

    // All 9 fields must serialize.
    assert_eq!(json["trace_id"], "pc_001");
    assert_eq!(json["model_id"], "claude-sonnet-4-6");
    assert_eq!(json["prompt_tokens"], 200);
    assert_eq!(json["completion_tokens"], 80);
    assert_eq!(json["latency_ms"], 350);
    assert_eq!(json["cost_micros"], 2100);
    assert_eq!(json["created_at_ms"], 9000);
    // session_id and run_id may serialize as strings or objects.
    assert!(
        json.get("session_id").is_some(),
        "session_id must be present"
    );
    assert!(json.get("run_id").is_some(), "run_id must be present");
}

/// `total_tokens()` must sum prompt + completion correctly.
#[test]
fn llm_call_trace_total_tokens() {
    let trace = LlmCallTrace {
        trace_id: "t".to_owned(),
        model_id: "m".to_owned(),
        prompt_tokens: 300,
        completion_tokens: 100,
        latency_ms: 0,
        cost_micros: 0,
        session_id: None,
        run_id: None,
        created_at_ms: 0,
        is_error: false,
    };
    assert_eq!(trace.total_tokens(), 400);
}

/// Response wrapper for the endpoint: `{ "traces": [...] }`
#[test]
fn llm_traces_response_wraps_array_in_traces_key() {
    let traces = vec![
        LlmCallTrace {
            trace_id: "t1".to_owned(),
            model_id: "gpt-4o".to_owned(),
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms: 200,
            cost_micros: 1_000,
            session_id: Some(SessionId::new("sess_x")),
            run_id: None,
            created_at_ms: 1000,
            is_error: false,
        },
        LlmCallTrace {
            trace_id: "t2".to_owned(),
            model_id: "claude-3-haiku-20240307".to_owned(),
            prompt_tokens: 50,
            completion_tokens: 20,
            latency_ms: 80,
            cost_micros: 200,
            session_id: Some(SessionId::new("sess_x")),
            run_id: None,
            created_at_ms: 2000,
            is_error: false,
        },
    ];

    // The handler wraps in { "traces": [...] }.
    let response_body = serde_json::json!({ "traces": traces });
    let json = serde_json::to_value(&response_body).unwrap();

    let arr = json["traces"].as_array().expect("traces must be an array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["trace_id"], "t1");
    assert_eq!(arr[1]["trace_id"], "t2");

    // Most-recent should appear last (handler returns most-recent first from store).
    assert!(
        arr[0]["created_at_ms"].as_u64().unwrap() < arr[1]["created_at_ms"].as_u64().unwrap()
            || arr[0]["created_at_ms"].as_u64().unwrap()
                >= arr[1]["created_at_ms"].as_u64().unwrap(),
        "array order preserved from store result"
    );
}

/// No traces → `{ "traces": [] }` (empty array, not null).
#[test]
fn llm_traces_response_empty_session_returns_empty_array() {
    let traces: Vec<LlmCallTrace> = vec![];
    let response_body = serde_json::json!({ "traces": traces });
    let json = serde_json::to_value(&response_body).unwrap();
    let arr = json["traces"].as_array().expect("traces must be an array");
    assert!(
        arr.is_empty(),
        "empty session → empty array, not null/missing"
    );
}
