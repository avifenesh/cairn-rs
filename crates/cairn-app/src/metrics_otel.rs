//! OTLP HTTP/protobuf transport + event-log tap for the `metrics-otel`
//! feature.
//!
//! The data-shape layer (`OtlpExporter`, `ExportableSpan`,
//! `SpanExportSink`) lives in `cairn-runtime/src/telemetry.rs` — see
//! RFC 021. This module adds:
//!
//! - [`HttpProtoSink`] — concrete `SpanExportSink` that serialises
//!   `ExportableSpan` → OTLP `Span` protobuf and POSTs batches to an
//!   OTLP/HTTP collector. Every major backend (Langfuse, Grafana
//!   Tempo, Jaeger, Datadog, Phoenix, Honeycomb) accepts this format.
//! - [`BatchingSink`] — wraps an inner sink with size/time batching
//!   so the tap doesn't hit the network per event.
//! - [`OtlpTap`] — background task that subscribes to the
//!   `InMemoryStore` broadcast and routes each `RuntimeEvent`
//!   through `OtlpExporter::export_event`.
//! - [`otlp_config_from_env`] — parses `CAIRN_OTLP_*` env vars into
//!   an `OtlpConfig`.

#![cfg(feature = "metrics-otel")]

use std::sync::Arc;
use std::time::Duration;

use cairn_domain::protocols::{OtlpConfig, OtlpProtocol};
use cairn_runtime::telemetry::{ExportableSpan, OtlpExporter, SpanAttributeValue, SpanExportSink};
use cairn_store::InMemoryStore;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{
    any_value::Value as AnyValueInner, AnyValue, InstrumentationScope, KeyValue,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{
    span::SpanKind, status::StatusCode, ResourceSpans, ScopeSpans, Span, Status,
};
use prost::Message;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

// ── HttpProtoSink ───────────────────────────────────────────────────

/// OTLP HTTP/protobuf transport. POSTs batches to
/// `{endpoint}/v1/traces` with `Content-Type: application/x-protobuf`.
///
/// Endpoint convention follows the OTLP/HTTP spec: operators set the
/// base URL (e.g. `http://localhost:4318`), we append `/v1/traces`.
/// Paths already ending in `/v1/traces` are accepted as-is so copy-paste
/// from collector docs (which sometimes include the path) works.
pub struct HttpProtoSink {
    client: reqwest::Client,
    endpoint: String,
    service_name: String,
}

impl HttpProtoSink {
    pub fn new(endpoint: &str, service_name: &str) -> Self {
        Self {
            // 10 s timeout covers slow collectors; batches fire
            // asynchronously so a sluggish backend doesn't stall
            // request-path code.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            endpoint: normalize_endpoint(endpoint),
            service_name: service_name.to_owned(),
        }
    }
}

fn normalize_endpoint(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.ends_with("/v1/traces") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/v1/traces")
    }
}

#[async_trait::async_trait]
impl SpanExportSink for HttpProtoSink {
    async fn export(&self, spans: &[ExportableSpan]) -> Result<(), String> {
        if spans.is_empty() {
            return Ok(());
        }
        let request = build_export_request(spans, &self.service_name);
        let mut body = Vec::with_capacity(request.encoded_len());
        request
            .encode(&mut body)
            .map_err(|e| format!("OTLP encode: {e}"))?;
        let resp = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/x-protobuf")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("OTLP POST to {}: {e}", self.endpoint))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!(
                "OTLP endpoint {} returned {}",
                self.endpoint, status
            ));
        }
        Ok(())
    }
}

fn build_export_request(spans: &[ExportableSpan], service_name: &str) -> ExportTraceServiceRequest {
    let proto_spans: Vec<Span> = spans.iter().map(to_proto_span).collect();
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "service.name".into(),
                    value: Some(AnyValue {
                        value: Some(AnyValueInner::StringValue(service_name.to_owned())),
                    }),
                }],
                dropped_attributes_count: 0,
                entity_refs: vec![],
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(InstrumentationScope {
                    name: "cairn-rs".into(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    attributes: vec![],
                    dropped_attributes_count: 0,
                }),
                spans: proto_spans,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    }
}

fn to_proto_span(span: &ExportableSpan) -> Span {
    let attributes = span
        .attributes
        .iter()
        .map(|(k, v)| KeyValue {
            key: k.clone(),
            value: Some(AnyValue {
                value: Some(match v {
                    SpanAttributeValue::String(s) => AnyValueInner::StringValue(s.clone()),
                    SpanAttributeValue::Int(i) => AnyValueInner::IntValue(*i),
                    SpanAttributeValue::Float(f) => AnyValueInner::DoubleValue(*f),
                    SpanAttributeValue::Bool(b) => AnyValueInner::BoolValue(*b),
                }),
            }),
        })
        .collect();

    let kind = match span.kind.as_str() {
        "server" => SpanKind::Server as i32,
        "client" => SpanKind::Client as i32,
        "internal" => SpanKind::Internal as i32,
        "producer" => SpanKind::Producer as i32,
        "consumer" => SpanKind::Consumer as i32,
        _ => SpanKind::Unspecified as i32,
    };
    let status_code = match span.status.as_str() {
        "ok" => StatusCode::Ok as i32,
        "error" => StatusCode::Error as i32,
        _ => StatusCode::Unset as i32,
    };

    Span {
        // OTLP protocol requires 16 hex chars (8 bytes) for span_id,
        // 32 hex chars (16 bytes) for trace_id. ExportableSpan formats
        // them that way via `{:016x}` / `{:032x}`; decode to bytes.
        trace_id: hex_to_bytes(&span.trace_id, 16),
        span_id: hex_to_bytes(&span.span_id, 8),
        trace_state: String::new(),
        parent_span_id: span
            .parent_span_id
            .as_ref()
            .map(|p| hex_to_bytes(p, 8))
            .unwrap_or_default(),
        flags: 0,
        name: span.name.clone(),
        kind,
        start_time_unix_nano: span.start_time_ns,
        end_time_unix_nano: span.end_time_ns,
        attributes,
        dropped_attributes_count: 0,
        events: vec![],
        dropped_events_count: 0,
        links: vec![],
        dropped_links_count: 0,
        status: Some(Status {
            message: String::new(),
            code: status_code,
        }),
    }
}

/// Decode a hex string to a big-endian byte array of exactly
/// `target_len` bytes. Odd-length, non-hex, or too-short inputs are
/// padded/truncated rather than failing — the OTLP spec rejects
/// empty/invalid IDs, and we'd rather emit a slightly-off ID than
/// drop the span entirely. In practice ExportableSpan always
/// produces the exact widths.
fn hex_to_bytes(hex: &str, target_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; target_len];
    let bytes = hex.as_bytes();
    let nibble = |c: u8| -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        }
    };
    for (i, out_byte) in out.iter_mut().enumerate().take(target_len) {
        let hi_idx = i * 2;
        let lo_idx = hi_idx + 1;
        if lo_idx >= bytes.len() {
            break;
        }
        *out_byte = (nibble(bytes[hi_idx]) << 4) | nibble(bytes[lo_idx]);
    }
    out
}

// ── BatchingSink ───────────────────────────────────────────────────

/// Buffers incoming spans and forwards them to an inner sink in
/// batches. Two flush triggers:
///   1. Buffer reaches `max_batch_size`.
///   2. `max_delay` elapses since the first span in the current batch.
///
/// Trigger 2 runs on a companion tokio task so a trickle of spans
/// still reaches the backend within bounded latency; trigger 1 runs
/// on the `export` path so bursty traffic doesn't wait for the timer.
pub struct BatchingSink {
    inner: Arc<dyn SpanExportSink>,
    buffer: Arc<Mutex<Vec<ExportableSpan>>>,
    max_batch_size: usize,
    cancel: CancellationToken,
    /// `Mutex<Option<_>>` so `shutdown` can `take()` and `await` the
    /// handle — otherwise the final-flush block inside the timer
    /// task races against process teardown and buffered spans go
    /// missing on restart.
    timer_handle: Mutex<Option<JoinHandle<()>>>,
}

impl BatchingSink {
    pub fn new(inner: Arc<dyn SpanExportSink>, max_batch_size: usize, max_delay: Duration) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::<ExportableSpan>::with_capacity(
            max_batch_size,
        )));
        let cancel = CancellationToken::new();
        let timer_buffer = buffer.clone();
        let timer_inner = inner.clone();
        let timer_cancel = cancel.clone();
        let timer_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = timer_cancel.cancelled() => break,
                    _ = tokio::time::sleep(max_delay) => {}
                }
                let mut buf = timer_buffer.lock().await;
                if buf.is_empty() {
                    continue;
                }
                let batch = std::mem::take(&mut *buf);
                drop(buf);
                if let Err(e) = timer_inner.export(&batch).await {
                    tracing::warn!(
                        error = %e,
                        count = batch.len(),
                        "OTLP batching sink: timer-triggered flush failed"
                    );
                }
            }
            // Final flush on shutdown.
            let mut buf = timer_buffer.lock().await;
            if !buf.is_empty() {
                let batch = std::mem::take(&mut *buf);
                drop(buf);
                if let Err(e) = timer_inner.export(&batch).await {
                    tracing::warn!(
                        error = %e,
                        count = batch.len(),
                        "OTLP batching sink: shutdown flush failed"
                    );
                }
            }
        });
        Self {
            inner,
            buffer,
            max_batch_size,
            cancel,
            timer_handle: Mutex::new(Some(timer_handle)),
        }
    }

    /// Cancel the timer and await its termination. The timer task
    /// runs the final-flush block before returning, so when this
    /// future resolves any buffered spans have been flushed to the
    /// inner sink. Idempotent.
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        if let Some(handle) = self.timer_handle.lock().await.take() {
            if let Err(e) = handle.await {
                tracing::warn!(error = %e, "OTLP batching sink: timer task panicked");
            }
        }
    }
}

#[async_trait::async_trait]
impl SpanExportSink for BatchingSink {
    async fn export(&self, spans: &[ExportableSpan]) -> Result<(), String> {
        let ready_batch = {
            let mut buf = self.buffer.lock().await;
            buf.extend_from_slice(spans);
            if buf.len() >= self.max_batch_size {
                Some(std::mem::take(&mut *buf))
            } else {
                None
            }
        };
        if let Some(batch) = ready_batch {
            self.inner.export(&batch).await?;
        }
        Ok(())
    }
}

// ── OtlpTap ────────────────────────────────────────────────────────

/// Event-log tap: subscribes to `InMemoryStore` broadcasts, routes
/// each `RuntimeEvent` to `OtlpExporter::export_event`, which in turn
/// calls the configured sink (typically a `BatchingSink` wrapping an
/// `HttpProtoSink`).
#[derive(Clone)]
pub struct OtlpTap {
    inner: Arc<OtlpTapInner>,
}

struct OtlpTapInner {
    handle: Mutex<Option<JoinHandle<()>>>,
    cancel: CancellationToken,
}

impl OtlpTap {
    pub fn spawn(store: Arc<InMemoryStore>, exporter: Arc<OtlpExporter>) -> Self {
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        // Subscribe before the spawn (same race-fix as MetricsTap).
        let mut rx = store.subscribe();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = worker_cancel.cancelled() => break,
                    msg = rx.recv() => match msg {
                        Ok(ev) => {
                            if let Err(e) = exporter.export_event(&ev.envelope.payload).await {
                                tracing::warn!(error = %e, "OTLP tap: export failed");
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            tracing::warn!(
                                dropped = n,
                                "OTLP tap: broadcast lag, {n} events missed"
                            );
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            }
            tracing::info!("OTLP tap stopped");
        });
        tracing::info!("OTLP tap started");
        Self {
            inner: Arc::new(OtlpTapInner {
                handle: Mutex::new(Some(handle)),
                cancel,
            }),
        }
    }

    pub async fn shutdown(&self) {
        self.inner.cancel.cancel();
        let handle = self.inner.handle.lock().await.take();
        if let Some(h) = handle {
            if let Err(e) = h.await {
                tracing::warn!(error = %e, "OTLP tap task panicked");
            }
        }
    }
}

// ── Config ────────────────────────────────────────────────────────

/// Parse `CAIRN_OTLP_*` env vars into an `OtlpConfig`. Returns a
/// disabled config when `CAIRN_OTLP_ENABLED` is unset or not truthy.
pub fn otlp_config_from_env() -> OtlpConfig {
    let enabled = std::env::var("CAIRN_OTLP_ENABLED")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    if !enabled {
        return OtlpConfig::default();
    }
    let endpoint =
        std::env::var("CAIRN_OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4318".to_owned());
    let protocol = match std::env::var("CAIRN_OTLP_PROTOCOL").ok().as_deref() {
        Some("grpc") => OtlpProtocol::Grpc,
        Some("http/json") | Some("http_json") => OtlpProtocol::HttpJson,
        _ => OtlpProtocol::HttpBinary,
    };
    let redact_content = std::env::var("CAIRN_OTLP_REDACT_CONTENT")
        .ok()
        .map(|v| !matches!(v.as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true);
    let service_name =
        std::env::var("CAIRN_OTLP_SERVICE_NAME").unwrap_or_else(|_| "cairn-rs".to_owned());
    OtlpConfig {
        enabled: true,
        endpoint,
        protocol,
        redact_content,
        service_name,
    }
}
