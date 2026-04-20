//! End-to-end tests for the metrics-otel feature.
//!
//! Spins up an axum HTTP server that impersonates an OTLP collector
//! (POST /v1/traces, protobuf body), wires an `OtlpTap` against it,
//! appends events to an `InMemoryStore`, and asserts the spans land
//! on the wire with the right attributes.

#![cfg(all(feature = "metrics-otel", feature = "in-memory-runtime"))]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use cairn_app::metrics_otel::{BatchingSink, HttpProtoSink, OtlpTap};
use cairn_domain::protocols::{OtlpConfig, OtlpProtocol};
use cairn_domain::providers::{OperationKind, ProviderCallStatus};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, ProviderBindingId, ProviderCallCompleted,
    ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId, RouteDecisionId,
    RunCreated, RunId, RuntimeEvent, SessionId,
};
use cairn_runtime::telemetry::OtlpExporter;
use cairn_store::event_log::EventLog;
use cairn_store::InMemoryStore;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;
use tokio::sync::Mutex;

/// Shared state for the mock OTLP collector — captures every
/// incoming OTLP request so tests can assert what was exported.
#[derive(Default, Clone)]
struct CollectorState {
    received: Arc<Mutex<Vec<ExportTraceServiceRequest>>>,
}

async fn collector_handler(State(state): State<CollectorState>, body: Bytes) -> StatusCode {
    let req = match ExportTraceServiceRequest::decode(body.as_ref()) {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    state.received.lock().await.push(req);
    StatusCode::OK
}

/// Start the mock collector on a random port; return the bound addr
/// + the shared captured-request vec.
async fn start_mock_collector() -> (SocketAddr, Arc<Mutex<Vec<ExportTraceServiceRequest>>>) {
    let state = CollectorState::default();
    let received = state.received.clone();
    let app = Router::new()
        .route("/v1/traces", post(collector_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, received)
}

fn project() -> ProjectKey {
    ProjectKey::new("t1", "w1", "p1")
}

fn make_exporter(endpoint: &str) -> Arc<OtlpExporter> {
    let cfg = OtlpConfig {
        enabled: true,
        endpoint: endpoint.to_owned(),
        protocol: OtlpProtocol::HttpBinary,
        redact_content: true,
        service_name: "cairn-rs-test".to_owned(),
    };
    let transport: Arc<dyn cairn_runtime::telemetry::SpanExportSink> =
        Arc::new(HttpProtoSink::new(endpoint, "cairn-rs-test"));
    // 1-span batch + 100 ms timer → fast-flush for tests.
    let batched = Arc::new(BatchingSink::new(transport, 1, Duration::from_millis(100)));
    // Shim: OtlpExporter::new takes Box<dyn Sink>, we have an Arc<dyn>.
    struct SinkArc(Arc<dyn cairn_runtime::telemetry::SpanExportSink>);
    #[async_trait::async_trait]
    impl cairn_runtime::telemetry::SpanExportSink for SinkArc {
        async fn export(
            &self,
            spans: &[cairn_runtime::telemetry::ExportableSpan],
        ) -> Result<(), String> {
            self.0.export(spans).await
        }
    }
    Arc::new(OtlpExporter::new(cfg, Box::new(SinkArc(batched))))
}

async fn wait_for(received: &Arc<Mutex<Vec<ExportTraceServiceRequest>>>, min_spans: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let total: usize = received
            .lock()
            .await
            .iter()
            .map(|r| {
                r.resource_spans
                    .iter()
                    .flat_map(|rs| &rs.scope_spans)
                    .map(|ss| ss.spans.len())
                    .sum::<usize>()
            })
            .sum();
        if total >= min_spans {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timeout waiting for {min_spans} spans; got {total}. \
                 Captured requests: {:?}",
                received.lock().await.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn run_created_emits_span_to_collector() {
    let (addr, received) = start_mock_collector().await;
    let endpoint = format!("http://{addr}");

    let store = Arc::new(InMemoryStore::new());
    let exporter = make_exporter(&endpoint);
    let tap = OtlpTap::spawn(store.clone(), exporter);

    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_1"),
            EventSource::Runtime,
            RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                run_id: RunId::new("run_1"),
                session_id: SessionId::new("sess_1"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
        )])
        .await
        .unwrap();

    wait_for(&received, 1).await;

    let requests = received.lock().await;
    let span = &requests[0].resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(span.name, "run.created");
    // service.name is on the Resource, not the Span.
    let svc_attr = requests[0].resource_spans[0]
        .resource
        .as_ref()
        .unwrap()
        .attributes
        .iter()
        .find(|kv| kv.key == "service.name")
        .unwrap();
    if let Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) =
        svc_attr.value.as_ref().unwrap().value.as_ref()
    {
        assert_eq!(s, "cairn-rs-test");
    } else {
        panic!("service.name value was not a string");
    }

    tap.shutdown().await;
}

#[tokio::test]
async fn provider_call_span_carries_genai_attributes() {
    let (addr, received) = start_mock_collector().await;
    let endpoint = format!("http://{addr}");

    let store = Arc::new(InMemoryStore::new());
    let exporter = make_exporter(&endpoint);
    let tap = OtlpTap::spawn(store.clone(), exporter);

    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_2"),
            EventSource::Runtime,
            RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                project: project(),
                provider_call_id: ProviderCallId::new("call_1"),
                route_decision_id: RouteDecisionId::new("rd_1"),
                route_attempt_id: RouteAttemptId::new("ra_1"),
                provider_binding_id: ProviderBindingId::new("binding_1"),
                provider_connection_id: ProviderConnectionId::new("openai"),
                provider_model_id: ProviderModelId::new("gpt-4o"),
                operation_kind: OperationKind::Generate,
                status: ProviderCallStatus::Succeeded,
                latency_ms: Some(420),
                input_tokens: Some(500),
                output_tokens: Some(200),
                cost_micros: None,
                completed_at: 1_000_000,
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
        )])
        .await
        .unwrap();

    wait_for(&received, 1).await;

    let requests = received.lock().await;
    let span = &requests[0].resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(span.name, "llm:gpt-4o");

    let attr = |k: &str| -> Option<String> {
        span.attributes
            .iter()
            .find(|kv| kv.key == k)
            .and_then(|kv| match kv.value.as_ref()?.value.as_ref()? {
                opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => {
                    Some(s.clone())
                }
                opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(i) => {
                    Some(i.to_string())
                }
                _ => None,
            })
    };
    assert_eq!(attr("gen_ai.operation.name").as_deref(), Some("chat"));
    assert_eq!(attr("gen_ai.request.model").as_deref(), Some("gpt-4o"));
    assert_eq!(attr("gen_ai.usage.input_tokens").as_deref(), Some("500"));
    assert_eq!(attr("gen_ai.usage.output_tokens").as_deref(), Some("200"));

    tap.shutdown().await;
}

#[tokio::test]
async fn batching_sink_flushes_on_timer() {
    // Batch size is 1 in the test setup so this already proves a
    // one-span flush. This test covers the under-batch-size path:
    // set batch size to 10, send 1 span, wait past the timer, assert
    // it arrived.
    let (addr, received) = start_mock_collector().await;
    let endpoint = format!("http://{addr}");

    // Custom exporter with larger batch — force the timer path.
    let cfg = OtlpConfig {
        enabled: true,
        endpoint: endpoint.clone(),
        protocol: OtlpProtocol::HttpBinary,
        redact_content: true,
        service_name: "cairn-rs-test".to_owned(),
    };
    let transport: Arc<dyn cairn_runtime::telemetry::SpanExportSink> =
        Arc::new(HttpProtoSink::new(&endpoint, "cairn-rs-test"));
    let batched = Arc::new(BatchingSink::new(
        transport,
        10, // won't trigger on size
        Duration::from_millis(200),
    ));
    struct SinkArc(Arc<dyn cairn_runtime::telemetry::SpanExportSink>);
    #[async_trait::async_trait]
    impl cairn_runtime::telemetry::SpanExportSink for SinkArc {
        async fn export(
            &self,
            spans: &[cairn_runtime::telemetry::ExportableSpan],
        ) -> Result<(), String> {
            self.0.export(spans).await
        }
    }
    let exporter = Arc::new(OtlpExporter::new(cfg, Box::new(SinkArc(batched))));
    let store = Arc::new(InMemoryStore::new());
    let tap = OtlpTap::spawn(store.clone(), exporter);

    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_3"),
            EventSource::Runtime,
            RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                run_id: RunId::new("run_3"),
                session_id: SessionId::new("sess_3"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
        )])
        .await
        .unwrap();

    // Wait longer than the timer — the single span should flush even
    // though we're below the batch-size threshold.
    wait_for(&received, 1).await;

    tap.shutdown().await;
}

#[tokio::test]
async fn batching_sink_shutdown_awaits_final_flush() {
    // Regression for the Cursor/Gemini review finding: the old
    // shutdown() returned before the final-flush block completed,
    // so buffered spans were silently lost on process exit. Now
    // shutdown() awaits the timer task, which runs the final flush
    // before returning.
    let (addr, received) = start_mock_collector().await;
    let endpoint = format!("http://{addr}");

    // Batch size 100 (never fires), timer 60 s (never fires during
    // this test). The only way the single enqueued span reaches the
    // collector is via the shutdown final-flush path.
    let transport: Arc<dyn cairn_runtime::telemetry::SpanExportSink> =
        Arc::new(HttpProtoSink::new(&endpoint, "cairn-rs-test"));
    let batched = Arc::new(BatchingSink::new(transport, 100, Duration::from_secs(60)));

    // Enqueue one span directly via the SpanExportSink contract.
    let span = cairn_runtime::telemetry::ExportableSpan {
        span_id: "0000000000000001".into(),
        parent_span_id: None,
        trace_id: "00000000000000000000000000000001".into(),
        name: "shutdown_flush_probe".into(),
        start_time_ns: 1_000_000,
        end_time_ns: 2_000_000,
        kind: "internal".into(),
        status: "ok".into(),
        attributes: std::collections::HashMap::new(),
    };
    use cairn_runtime::telemetry::SpanExportSink;
    batched.export(std::slice::from_ref(&span)).await.unwrap();

    // Asserting collector silence BEFORE shutdown, then non-silence
    // AFTER, would be correct but racy on slow CI — instead we rely
    // on the batch config (100 / 60s) to make pre-shutdown arrival
    // structurally impossible within the test timeout.
    batched.shutdown().await;

    wait_for(&received, 1).await;
}
