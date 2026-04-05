//! LLM observability / traces end-to-end integration tests.
//!
//! Tests the full trace recording and query arc:
//!   1. Record an LLM call trace with model_id, tokens, latency, cost
//!   2. Verify the trace is retrievable by session via list_by_session
//!   3. Record multiple traces for a session
//!   4. List traces and verify ordering (most-recent first)
//!   5. Verify cost aggregation across traces
//!
//! Additional coverage:
//!   - list_all returns traces across sessions in most-recent-first order
//!   - Session isolation: list_by_session scopes results to one session
//!   - Pagination via limit is respected
//!   - Error traces are tracked separately from successes
//!   - latency_percentiles and error_rate computed correctly

use std::sync::Arc;

use cairn_domain::{LlmCallTrace, RunId, SessionId};
use cairn_runtime::observability::LlmObservabilityService;
use cairn_runtime::services::LlmObservabilityServiceImpl;
use cairn_store::InMemoryStore;

fn session(id: &str) -> SessionId {
    SessionId::new(id)
}

fn trace(
    trace_id: &str,
    model: &str,
    session_id: &SessionId,
    prompt_tokens: u32,
    completion_tokens: u32,
    latency_ms: u64,
    cost_micros: u64,
    created_at_ms: u64,
    is_error: bool,
) -> LlmCallTrace {
    LlmCallTrace {
        trace_id: trace_id.to_owned(),
        model_id: model.to_owned(),
        prompt_tokens,
        completion_tokens,
        latency_ms,
        cost_micros,
        session_id: Some(session_id.clone()),
        run_id: None,
        created_at_ms,
        is_error,
    }
}

// ── Test 1+2: record trace, verify retrievable by session ────────────────────

/// Record one LLM call trace with all fields populated, then verify
/// list_by_session returns it with identical field values.
#[tokio::test]
async fn record_trace_and_retrieve_by_session() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let sess = session("sess_obs_1");

    // ── (1) Record a trace ────────────────────────────────────────────────
    svc.record(LlmCallTrace {
        trace_id: "trace_obs_1".to_owned(),
        model_id: "claude-sonnet-4-6".to_owned(),
        prompt_tokens: 512,
        completion_tokens: 128,
        latency_ms: 350,
        cost_micros: 2_400,
        session_id: Some(sess.clone()),
        run_id: Some(RunId::new("run_obs_1")),
        created_at_ms: 1_700_000_001_000,
        is_error: false,
    })
    .await
    .unwrap();

    // ── (2) Verify retrievable by session ─────────────────────────────────
    let traces = svc.list_by_session(&sess, 10).await.unwrap();

    assert_eq!(traces.len(), 1, "one trace must be returned for the session");

    let t = &traces[0];
    assert_eq!(t.trace_id, "trace_obs_1");
    assert_eq!(t.model_id, "claude-sonnet-4-6");
    assert_eq!(t.prompt_tokens, 512);
    assert_eq!(t.completion_tokens, 128);
    assert_eq!(t.latency_ms, 350);
    assert_eq!(t.cost_micros, 2_400);
    assert_eq!(t.session_id.as_ref().unwrap(), &sess);
    assert!(!t.is_error, "success trace must have is_error=false");
    assert!(t.run_id.is_some());
}

// ── Tests 3+4: multiple traces, ordering ─────────────────────────────────────

/// Record three traces for the same session with ascending timestamps.
/// list_by_session must return them in most-recent-first order.
#[tokio::test]
async fn multiple_traces_ordered_most_recent_first() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let sess = session("sess_order");
    let base_ms = 1_700_000_000_000u64;

    // ── (3) Record three traces ────────────────────────────────────────────
    for (i, model) in [
        (0u64, "gpt-4"),
        (1,    "claude-haiku-4-5"),
        (2,    "claude-sonnet-4-6"),
    ] {
        svc.record(trace(
            &format!("t_order_{i}"),
            model,
            &sess,
            100,
            50,
            100 + i * 10,   // 100, 110, 120 ms
            1_000 + i * 100, // 1000, 1100, 1200 micros
            base_ms + i,    // ascending timestamps
            false,
        ))
        .await
        .unwrap();
    }

    // ── (4) Verify most-recent-first ordering ──────────────────────────────
    let traces = svc.list_by_session(&sess, 10).await.unwrap();
    assert_eq!(traces.len(), 3, "all three traces must be returned");

    // Most recent (largest created_at_ms) first.
    assert!(
        traces[0].created_at_ms >= traces[1].created_at_ms,
        "first trace must be most recent; got order: {:?}",
        traces.iter().map(|t| t.created_at_ms).collect::<Vec<_>>()
    );
    assert!(
        traces[1].created_at_ms >= traces[2].created_at_ms,
        "second trace must be more recent than third"
    );

    // The most recent trace is index 2 (highest timestamp).
    assert_eq!(traces[0].trace_id, "t_order_2", "most recent trace must be t_order_2");
    assert_eq!(traces[2].trace_id, "t_order_0", "oldest trace must be last");
}

// ── Test 5: cost aggregation across traces ────────────────────────────────────

/// Cost aggregation: sum cost_micros across all traces for a session.
/// The service doesn't have a dedicated aggregate method; callers sum the list.
#[tokio::test]
async fn cost_aggregation_across_traces() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let sess = session("sess_cost");
    let base_ms = 1_700_000_000_000u64;

    // ── (5) Record traces with known costs ────────────────────────────────
    let costs = [500u64, 1_200, 800, 2_500]; // total = 5_000 micros
    for (i, cost) in costs.iter().enumerate() {
        svc.record(trace(
            &format!("t_cost_{i}"),
            "gpt-4o",
            &sess,
            200,
            80,
            200,
            *cost,
            base_ms + i as u64,
            false,
        ))
        .await
        .unwrap();
    }

    let traces = svc.list_by_session(&sess, 10).await.unwrap();
    assert_eq!(traces.len(), 4);

    let total_cost: u64 = traces.iter().map(|t| t.cost_micros).sum();
    assert_eq!(
        total_cost,
        5_000,
        "aggregated cost must equal sum of all individual trace costs"
    );

    // Total tokens.
    let total_prompt: u32 = traces.iter().map(|t| t.prompt_tokens).sum();
    let total_completion: u32 = traces.iter().map(|t| t.completion_tokens).sum();
    assert_eq!(total_prompt, 800,      "4 × 200 = 800 prompt tokens");
    assert_eq!(total_completion, 320,  "4 × 80 = 320 completion tokens");

    // Average latency.
    let avg_latency: u64 = traces.iter().map(|t| t.latency_ms).sum::<u64>() / traces.len() as u64;
    assert_eq!(avg_latency, 200, "all traces have 200ms latency; avg = 200");
}

// ── Session isolation ──────────────────────────────────────────────────────────

/// list_by_session must only return traces for the queried session.
#[tokio::test]
async fn session_isolation_in_list_by_session() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let sess_a = session("sess_iso_a");
    let sess_b = session("sess_iso_b");
    let base_ms = 1_700_000_000_000u64;

    svc.record(trace("ta1", "gpt-4", &sess_a, 100, 40, 100, 1_000, base_ms,     false)).await.unwrap();
    svc.record(trace("ta2", "gpt-4", &sess_a, 100, 40, 120, 1_100, base_ms + 1, false)).await.unwrap();
    svc.record(trace("tb1", "gpt-4", &sess_b, 100, 40, 150, 1_200, base_ms + 2, false)).await.unwrap();

    let a_traces = svc.list_by_session(&sess_a, 10).await.unwrap();
    let b_traces = svc.list_by_session(&sess_b, 10).await.unwrap();

    assert_eq!(a_traces.len(), 2, "session A must see only its 2 traces");
    assert_eq!(b_traces.len(), 1, "session B must see only its 1 trace");

    let a_ids: Vec<&str> = a_traces.iter().map(|t| t.trace_id.as_str()).collect();
    assert!(a_ids.contains(&"ta1") && a_ids.contains(&"ta2"));
    assert!(!a_ids.contains(&"tb1"), "session B's trace must not appear in session A's list");
}

// ── list_all returns traces across sessions ───────────────────────────────────

#[tokio::test]
async fn list_all_returns_traces_across_sessions() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let s1 = session("sess_all_1");
    let s2 = session("sess_all_2");
    let base_ms = 1_700_000_000_000u64;

    for (i, sess) in [(0u64, &s1), (1, &s2), (2, &s1)] {
        svc.record(trace(&format!("tall_{i}"), "model-x", sess, 50, 20, 80, 500, base_ms + i, false))
            .await
            .unwrap();
    }

    let all = svc.list_all(10).await.unwrap();
    assert_eq!(all.len(), 3, "list_all must return all 3 traces across both sessions");

    // Most-recent first.
    assert!(all[0].created_at_ms >= all[1].created_at_ms);
    assert!(all[1].created_at_ms >= all[2].created_at_ms);
}

// ── Pagination via limit ───────────────────────────────────────────────────────

#[tokio::test]
async fn list_by_session_respects_limit() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let sess = session("sess_limit");
    let base_ms = 1_700_000_000_000u64;

    for i in 0u64..5 {
        svc.record(trace(&format!("tl_{i}"), "m", &sess, 10, 5, 50, 100, base_ms + i, false))
            .await
            .unwrap();
    }

    let limited = svc.list_by_session(&sess, 3).await.unwrap();
    assert_eq!(limited.len(), 3, "limit=3 must return at most 3 traces");

    // The 3 most recent must be returned.
    for t in &limited {
        assert!(
            t.created_at_ms >= base_ms + 2,
            "only the 3 most recent traces (ms >= base+2) must be returned"
        );
    }
}

// ── Error traces tracked correctly ───────────────────────────────────────────

#[tokio::test]
async fn error_traces_tracked_separately() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let sess = session("sess_err");
    let base_ms = 1_700_000_000_000u64;

    // 3 successes + 2 errors.
    for i in 0u64..3 {
        svc.record(trace(&format!("ok_{i}"), "m", &sess, 100, 50, 100, 500, base_ms + i, false))
            .await
            .unwrap();
    }
    for i in 0u64..2 {
        svc.record(trace(&format!("err_{i}"), "m", &sess, 100, 0, 50, 0, base_ms + 3 + i, true))
            .await
            .unwrap();
    }

    let all = svc.list_by_session(&sess, 10).await.unwrap();
    assert_eq!(all.len(), 5);

    let error_count = all.iter().filter(|t| t.is_error).count();
    let success_count = all.iter().filter(|t| !t.is_error).count();
    assert_eq!(error_count, 2, "2 error traces must be present");
    assert_eq!(success_count, 3, "3 success traces must be present");

    // Error traces have cost_micros = 0 (failed calls aren't billed).
    let error_cost: u64 = all.iter().filter(|t| t.is_error).map(|t| t.cost_micros).sum();
    assert_eq!(error_cost, 0, "error traces must carry zero cost");
}

// ── latency_percentiles computed correctly ────────────────────────────────────

#[tokio::test]
async fn latency_percentiles_over_session_traces() {
    let store = Arc::new(InMemoryStore::new());
    let svc = LlmObservabilityServiceImpl::new(store);

    let sess = session("sess_perc");
    // Use a far-future base so all traces fall within any window.
    let far_future = u64::MAX / 2;

    // Latencies: 10, 20, 30, 40, 50, 60, 70, 80, 90, 100
    // Sorted ascending. p50 idx = 5 → 60; p95 idx = 9 → 100.
    for i in 1u64..=10 {
        svc.record(LlmCallTrace {
            trace_id: format!("tp_{i}"),
            model_id: "m".to_owned(),
            prompt_tokens: 10,
            completion_tokens: 5,
            latency_ms: i * 10,
            cost_micros: 100,
            session_id: Some(sess.clone()),
            run_id: None,
            created_at_ms: far_future + i,
            is_error: false,
        })
        .await
        .unwrap();
    }

    let stats = svc.latency_percentiles(u64::MAX / 2).await.unwrap();
    assert_eq!(stats.sample_count, 10);
    assert_eq!(stats.p50_ms, 60, "p50 must be 60 ms for [10..100] sorted");
    assert_eq!(stats.p95_ms, 100, "p95 must be 100 ms for [10..100] sorted");
}
