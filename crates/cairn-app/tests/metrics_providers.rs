//! Tests for the metrics-providers feature: the metrics tap derives
//! LLM provider call counters, duration histograms, and token
//! counters from `ProviderCallCompleted` events.
//!
//! Same isolation pattern as metrics_core.rs — append events directly
//! to an `InMemoryStore` with a live `MetricsTap` attached. The events
//! are the same shape any production provider emits.

#![cfg(all(feature = "metrics-providers", feature = "in-memory-runtime"))]

use std::sync::Arc;
use std::time::Duration;

use cairn_app::metrics::AppMetrics;
use cairn_app::metrics_tap::MetricsTap;
use cairn_domain::providers::{OperationKind, ProviderCallStatus};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, ProviderBindingId, ProviderCallCompleted,
    ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId, RouteDecisionId,
    RuntimeEvent,
};
use cairn_store::event_log::EventLog;
use cairn_store::InMemoryStore;

async fn setup() -> (Arc<InMemoryStore>, Arc<AppMetrics>, MetricsTap) {
    let store = Arc::new(InMemoryStore::new());
    let metrics = Arc::new(AppMetrics::default());
    let tap = MetricsTap::spawn(store.clone(), metrics.clone());
    (store, metrics, tap)
}

/// 200ms drain window — same budget as metrics_core. Broadcast delivery
/// is typically sub-millisecond in-process; the margin absorbs CI noise.
async fn drain_tap() {
    tokio::time::sleep(Duration::from_millis(200)).await;
}

fn project() -> ProjectKey {
    ProjectKey::new("t1", "w1", "p1")
}

#[allow(clippy::too_many_arguments)]
fn provider_call(
    id: &str,
    connection: &str,
    model: &str,
    operation: OperationKind,
    status: ProviderCallStatus,
    latency_ms: Option<u64>,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("test_{id}")),
        EventSource::Runtime,
        RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
            project: project(),
            provider_call_id: ProviderCallId::new(id),
            route_decision_id: RouteDecisionId::new("route_1"),
            route_attempt_id: RouteAttemptId::new("attempt_1"),
            provider_binding_id: ProviderBindingId::new("binding_1"),
            provider_connection_id: ProviderConnectionId::new(connection),
            provider_model_id: ProviderModelId::new(model),
            operation_kind: operation,
            status,
            latency_ms,
            input_tokens,
            output_tokens,
            cost_micros: None,
            completed_at: 0,
            session_id: None,
            run_id: None,
            error_class: None,
            raw_error_message: None,
            retry_count: 0,
            task_id: None,
            prompt_release_id: None,
            fallback_position: 0,
            started_at: 0,
            finished_at: 0,
        }),
    )
}

#[tokio::test]
async fn succeeded_call_bumps_counter_and_histogram_and_tokens() {
    let (store, metrics, tap) = setup().await;

    store
        .append(&[provider_call(
            "call_1",
            "openai",
            "gpt-4o",
            OperationKind::Generate,
            ProviderCallStatus::Succeeded,
            Some(840),
            Some(1000),
            Some(500),
        )])
        .await
        .unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();

    // Counter
    assert!(
        output.contains(
            r#"cairn_provider_calls_total{provider_connection="openai",model="gpt-4o",operation_kind="generate",status="succeeded"} 1"#
        ),
        "missing calls counter row:\n{output}"
    );

    // Histogram — 840ms falls in the le=1000 bucket and everything above.
    // Buckets: 100, 250, 500, 1000, 2500, 5000, 10000, 30000, 60000, 120000.
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_bucket{provider_connection="openai",model="gpt-4o",operation_kind="generate",le="1000"} 1"#
    ));
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_bucket{provider_connection="openai",model="gpt-4o",operation_kind="generate",le="500"} 0"#
    ));
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_sum{provider_connection="openai",model="gpt-4o",operation_kind="generate"} 840"#
    ));
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_count{provider_connection="openai",model="gpt-4o",operation_kind="generate"} 1"#
    ));

    // Token counters
    assert!(output.contains(
        r#"cairn_provider_tokens_total{provider_connection="openai",model="gpt-4o",kind="input"} 1000"#
    ));
    assert!(output.contains(
        r#"cairn_provider_tokens_total{provider_connection="openai",model="gpt-4o",kind="output"} 500"#
    ));

    tap.shutdown().await;
}

#[tokio::test]
async fn failed_call_without_latency_bumps_counter_but_not_histogram() {
    // Some failures surface before the call returns — e.g. admission
    // denial at the router. There's no latency to report. Counter
    // still bumps; histogram stays at zero count.
    let (store, metrics, tap) = setup().await;

    store
        .append(&[provider_call(
            "call_fail",
            "anthropic",
            "claude-3.5-sonnet",
            OperationKind::Generate,
            ProviderCallStatus::Failed,
            None,
            None,
            None,
        )])
        .await
        .unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(output.contains(
        r#"cairn_provider_calls_total{provider_connection="anthropic",model="claude-3.5-sonnet",operation_kind="generate",status="failed"} 1"#
    ));
    assert!(
        !output.contains(r#"cairn_provider_call_duration_ms_count{provider_connection="anthropic"#),
        "histogram should not emit for a no-latency failure:\n{output}"
    );

    tap.shutdown().await;
}

#[tokio::test]
async fn embed_operation_separates_from_generate_in_labels() {
    // Embedding calls hit a different duration profile than
    // generation; dashboards need the split. Confirm both live as
    // distinct series under the same connection+model.
    let (store, metrics, tap) = setup().await;

    store
        .append(&[
            provider_call(
                "gen_1",
                "openai",
                "gpt-4o",
                OperationKind::Generate,
                ProviderCallStatus::Succeeded,
                Some(1500),
                Some(500),
                Some(200),
            ),
            provider_call(
                "emb_1",
                "openai",
                "text-embedding-3-small",
                OperationKind::Embed,
                ProviderCallStatus::Succeeded,
                Some(120),
                Some(8000),
                None,
            ),
        ])
        .await
        .unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(output.contains(
        r#"cairn_provider_calls_total{provider_connection="openai",model="gpt-4o",operation_kind="generate",status="succeeded"} 1"#
    ));
    assert!(output.contains(
        r#"cairn_provider_calls_total{provider_connection="openai",model="text-embedding-3-small",operation_kind="embed",status="succeeded"} 1"#
    ));
    // Embedding latency should land in the le=250 bucket; generate in le=2500.
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_bucket{provider_connection="openai",model="text-embedding-3-small",operation_kind="embed",le="250"} 1"#
    ));
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_bucket{provider_connection="openai",model="gpt-4o",operation_kind="generate",le="2500"} 1"#
    ));

    tap.shutdown().await;
}

#[tokio::test]
async fn multiple_calls_accumulate_in_counter_and_histogram() {
    let (store, metrics, tap) = setup().await;

    let mut events = Vec::new();
    for (i, latency) in [150u64, 800, 2_200].iter().enumerate() {
        events.push(provider_call(
            &format!("call_{i}"),
            "openai",
            "gpt-4o",
            OperationKind::Generate,
            ProviderCallStatus::Succeeded,
            Some(*latency),
            Some(100),
            Some(50),
        ));
    }
    store.append(&events).await.unwrap();

    drain_tap().await;

    let output = metrics.render_prometheus();
    assert!(output.contains(
        r#"cairn_provider_calls_total{provider_connection="openai",model="gpt-4o",operation_kind="generate",status="succeeded"} 3"#
    ));
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_count{provider_connection="openai",model="gpt-4o",operation_kind="generate"} 3"#
    ));
    // sum = 150 + 800 + 2200 = 3150
    assert!(output.contains(
        r#"cairn_provider_call_duration_ms_sum{provider_connection="openai",model="gpt-4o",operation_kind="generate"} 3150"#
    ));
    // tokens: 100*3 = 300 input, 50*3 = 150 output
    assert!(output.contains(
        r#"cairn_provider_tokens_total{provider_connection="openai",model="gpt-4o",kind="input"} 300"#
    ));
    assert!(output.contains(
        r#"cairn_provider_tokens_total{provider_connection="openai",model="gpt-4o",kind="output"} 150"#
    ));

    tap.shutdown().await;
}

#[tokio::test]
async fn render_surface_present_without_any_events() {
    // TYPE lines must be present on a cold cairn — Prometheus scrapers
    // rely on the schema line to know the metric exists.
    let (_store, metrics, tap) = setup().await;

    let output = metrics.render_prometheus();
    for line in &[
        "# TYPE cairn_provider_calls_total counter",
        "# TYPE cairn_provider_call_duration_ms histogram",
        "# TYPE cairn_provider_tokens_total counter",
    ] {
        assert!(
            output.contains(line),
            "render_prometheus missing TYPE line `{line}`:\n{output}"
        );
    }

    tap.shutdown().await;
}
