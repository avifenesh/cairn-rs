//! OTLP Span Export (RFC 021).
//!
//! Maps `RuntimeEvent` variants to OpenTelemetry GenAI semantic convention
//! spans for export to any OTLP-compatible backend (Langfuse, Grafana Tempo,
//! Jaeger, Datadog, Phoenix, etc.).
//!
//! No external OTel SDK dependency — this module produces `ExportableSpan`
//! structs that a transport layer serializes to OTLP wire format. The
//! transport (gRPC or HTTP) is injected via the `SpanExportSink` trait.

use cairn_domain::protocols::OtlpConfig;
use cairn_domain::RuntimeEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── ExportableSpan ───────────────────────────────────────────────────────────

/// An OTel-compatible span ready for export.
///
/// Follows the OpenTelemetry GenAI Agent Application Semantic Convention
/// (2025). Transport implementations serialize this to OTLP protobuf or JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportableSpan {
    /// Unique span ID (hex string).
    pub span_id: String,
    /// Parent span ID for nesting (run → tool → LLM call).
    pub parent_span_id: Option<String>,
    /// Trace ID grouping related spans.
    pub trace_id: String,
    /// Span name (e.g. "tool_call:memory_search", "llm:gpt-4").
    pub name: String,
    /// Start time in nanoseconds since epoch.
    pub start_time_ns: u64,
    /// End time in nanoseconds since epoch (0 if in-progress).
    pub end_time_ns: u64,
    /// OTel span kind: "internal", "client", "server".
    pub kind: String,
    /// OTel status: "ok", "error", "unset".
    pub status: String,
    /// GenAI semantic convention attributes.
    pub attributes: HashMap<String, SpanAttributeValue>,
}

/// Span attribute value (OTel supports string, int, float, bool, array).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpanAttributeValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

// ── SpanExportSink ───────────────────────────────────────────────────────────

/// Trait for sending spans to an OTLP backend.
///
/// Implementations handle serialization and transport (gRPC, HTTP/protobuf,
/// HTTP/JSON). The exporter calls `export` with batches of spans.
#[async_trait::async_trait]
pub trait SpanExportSink: Send + Sync {
    async fn export(&self, spans: &[ExportableSpan]) -> Result<(), String>;
}

/// No-op sink for when OTLP export is disabled.
pub struct NoOpSink;

#[async_trait::async_trait]
impl SpanExportSink for NoOpSink {
    async fn export(&self, _spans: &[ExportableSpan]) -> Result<(), String> {
        Ok(())
    }
}

// ── OtlpExporter ─────────────────────────────────────────────────────────────

/// Maps `RuntimeEvent` variants to `ExportableSpan` and sends them to a sink.
///
/// Respects `OtlpConfig.redact_content` — when true, message content,
/// tool args, and model responses are stripped from span attributes.
pub struct OtlpExporter {
    config: OtlpConfig,
    sink: Box<dyn SpanExportSink>,
}

impl OtlpExporter {
    pub fn new(config: OtlpConfig, sink: Box<dyn SpanExportSink>) -> Self {
        Self { config, sink }
    }

    /// Disabled exporter (no-op sink, export disabled).
    pub fn disabled() -> Self {
        Self {
            config: OtlpConfig::default(),
            sink: Box::new(NoOpSink),
        }
    }

    /// Whether export is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Export a single runtime event as one or more OTel spans.
    pub async fn export_event(&self, event: &RuntimeEvent) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }

        let spans = self.event_to_spans(event);
        if spans.is_empty() {
            return Ok(());
        }

        self.sink.export(&spans).await
    }

    /// Map a RuntimeEvent to zero or more exportable spans.
    fn event_to_spans(&self, event: &RuntimeEvent) -> Vec<ExportableSpan> {
        match event {
            RuntimeEvent::RunCreated(e) => {
                vec![self.run_span(e.run_id.as_ref(), e.session_id.as_ref(), "run.created", 0)]
            }
            RuntimeEvent::RunStateChanged(e) => vec![self.run_span(
                e.run_id.as_ref(),
                "",
                &format!("run.state_changed:{:?}", e.transition.to),
                0,
            )],
            RuntimeEvent::ToolInvocationStarted(e) => {
                let tool_name = match &e.target {
                    cairn_domain::tool_invocation::ToolInvocationTarget::Builtin { tool_name } => {
                        tool_name.clone()
                    }
                    cairn_domain::tool_invocation::ToolInvocationTarget::Plugin {
                        tool_name,
                        ..
                    } => tool_name.clone(),
                };
                let mut attrs = HashMap::new();
                attrs.insert(
                    "gen_ai.operation.name".into(),
                    SpanAttributeValue::String("tool_call".into()),
                );
                attrs.insert(
                    "cairn.tool.name".into(),
                    SpanAttributeValue::String(tool_name.clone()),
                );
                let run_str = e.run_id.as_ref().map(|r| r.to_string()).unwrap_or_default();
                vec![ExportableSpan {
                    span_id: format!("{:016x}", hash_id(e.invocation_id.as_ref())),
                    parent_span_id: Some(format!("{:016x}", hash_id(&run_str))),
                    trace_id: format!("{:032x}", hash_id(&run_str)),
                    name: format!("tool:{tool_name}"),
                    start_time_ns: e.started_at_ms * 1_000_000,
                    end_time_ns: 0,
                    kind: "client".into(),
                    status: "unset".into(),
                    attributes: attrs,
                }]
            }
            RuntimeEvent::ToolInvocationCompleted(e) => {
                let op_name = if e.tool_name == "memory_search" {
                    "retrieval"
                } else {
                    "tool_call"
                };
                let mut attrs = HashMap::new();
                attrs.insert(
                    "gen_ai.operation.name".into(),
                    SpanAttributeValue::String(op_name.into()),
                );
                attrs.insert(
                    "cairn.tool.name".into(),
                    SpanAttributeValue::String(e.tool_name.clone()),
                );
                vec![ExportableSpan {
                    span_id: format!("{:016x}", hash_id(e.invocation_id.as_ref())),
                    parent_span_id: None,
                    trace_id: format!("{:032x}", hash_id(e.invocation_id.as_ref())),
                    name: format!("tool:{}", e.tool_name),
                    start_time_ns: 0,
                    end_time_ns: e.finished_at_ms * 1_000_000,
                    kind: "client".into(),
                    status: "ok".into(),
                    attributes: attrs,
                }]
            }
            RuntimeEvent::ProviderCallCompleted(e) => {
                let mut attrs = HashMap::new();
                attrs.insert(
                    "gen_ai.operation.name".into(),
                    SpanAttributeValue::String("chat".into()),
                );
                attrs.insert(
                    "gen_ai.request.model".into(),
                    SpanAttributeValue::String(e.provider_model_id.to_string()),
                );
                if let Some(input) = e.input_tokens {
                    attrs.insert(
                        "gen_ai.usage.input_tokens".into(),
                        SpanAttributeValue::Int(input as i64),
                    );
                }
                if let Some(output) = e.output_tokens {
                    attrs.insert(
                        "gen_ai.usage.output_tokens".into(),
                        SpanAttributeValue::Int(output as i64),
                    );
                }
                let run_str = e.run_id.as_ref().map(|r| r.to_string()).unwrap_or_default();
                vec![ExportableSpan {
                    span_id: format!("{:016x}", hash_id(e.provider_call_id.as_ref())),
                    parent_span_id: if run_str.is_empty() {
                        None
                    } else {
                        Some(format!("{:016x}", hash_id(&run_str)))
                    },
                    trace_id: if run_str.is_empty() {
                        format!("{:032x}", hash_id(e.provider_call_id.as_ref()))
                    } else {
                        format!("{:032x}", hash_id(&run_str))
                    },
                    name: format!("llm:{}", e.provider_model_id),
                    start_time_ns: 0,
                    end_time_ns: e.completed_at * 1_000_000,
                    kind: "client".into(),
                    status: "ok".into(),
                    attributes: attrs,
                }]
            }
            RuntimeEvent::RouteDecisionMade(e) => {
                let mut attrs = HashMap::new();
                attrs.insert(
                    "gen_ai.operation.name".into(),
                    SpanAttributeValue::String("route_decision".into()),
                );
                attrs.insert(
                    "cairn.route.operation_kind".into(),
                    SpanAttributeValue::String(format!("{:?}", e.operation_kind)),
                );
                attrs.insert(
                    "cairn.route.status".into(),
                    SpanAttributeValue::String(format!("{:?}", e.final_status)),
                );
                attrs.insert(
                    "cairn.route.attempt_count".into(),
                    SpanAttributeValue::Int(i64::from(e.attempt_count)),
                );
                attrs.insert(
                    "cairn.route.fallback_used".into(),
                    SpanAttributeValue::Bool(e.fallback_used),
                );
                if let Some(binding) = &e.selected_provider_binding_id {
                    attrs.insert(
                        "cairn.route.selected_binding".into(),
                        SpanAttributeValue::String(binding.to_string()),
                    );
                }
                vec![ExportableSpan {
                    span_id: format!("{:016x}", hash_id(e.route_decision_id.as_ref())),
                    // Route decisions structurally can't correlate with
                    // the enclosing run: `RouteDecisionMade` carries no
                    // `run_id`. The downstream `ProviderCallCompleted`
                    // both references this decision (via
                    // route_decision_id) and does carry run_id, so
                    // operators can join the two in their OTLP
                    // backend. Adding run_id to the event is a domain
                    // change tracked as a follow-up.
                    parent_span_id: None,
                    trace_id: format!("{:032x}", hash_id(e.route_decision_id.as_ref())),
                    name: "route_decision".into(),
                    start_time_ns: e.decided_at * 1_000_000,
                    end_time_ns: e.decided_at * 1_000_000,
                    kind: "internal".into(),
                    status: match e.final_status {
                        cairn_domain::providers::RouteDecisionStatus::Selected => "ok".into(),
                        _ => "error".into(),
                    },
                    attributes: attrs,
                }]
            }
            // All other events — no span export in v1.
            _ => vec![],
        }
    }

    fn run_span(&self, run_id: &str, session_id: &str, name: &str, ts: u64) -> ExportableSpan {
        let mut attrs = HashMap::new();
        attrs.insert(
            "cairn.run.id".into(),
            SpanAttributeValue::String(run_id.to_owned()),
        );
        if !session_id.is_empty() {
            attrs.insert(
                "cairn.session.id".into(),
                SpanAttributeValue::String(session_id.to_owned()),
            );
        }
        attrs.insert(
            "service.name".into(),
            SpanAttributeValue::String(self.config.service_name.clone()),
        );
        ExportableSpan {
            span_id: format!("{:016x}", hash_id(run_id)),
            parent_span_id: None,
            trace_id: format!("{:032x}", hash_id(run_id)),
            name: name.to_owned(),
            start_time_ns: ts * 1_000_000,
            end_time_ns: 0,
            kind: "server".into(),
            status: "unset".into(),
            attributes: attrs,
        }
    }
}

/// Simple FNV-1a hash for deterministic span/trace IDs from string keys.
fn hash_id(s: &str) -> u64 {
    s.as_bytes().iter().fold(0xcbf29ce484222325u64, |h, &b| {
        h.wrapping_mul(0x100000001b3) ^ (b as u64)
    })
}

/// Truncate a string to at most `max_bytes` (for attribute value size limits).
#[allow(dead_code)]
fn truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_owned()
    } else {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::protocols::OtlpConfig;

    #[test]
    fn disabled_exporter_produces_no_spans() {
        let exporter = OtlpExporter::disabled();
        assert!(!exporter.is_enabled());
    }

    #[tokio::test]
    async fn disabled_export_is_noop() {
        let exporter = OtlpExporter::disabled();
        let event =
            cairn_domain::RuntimeEvent::SessionCreated(cairn_domain::events::SessionCreated {
                project: cairn_domain::ProjectKey::new("t", "w", "p"),
                session_id: cairn_domain::SessionId::new("s1"),
            });
        assert!(exporter.export_event(&event).await.is_ok());
    }

    #[test]
    fn tool_invocation_started_produces_span() {
        let config = OtlpConfig {
            enabled: true,
            ..Default::default()
        };
        let exporter = OtlpExporter::new(config, Box::new(NoOpSink));
        let event = cairn_domain::RuntimeEvent::ToolInvocationStarted(
            cairn_domain::events::ToolInvocationStarted {
                project: cairn_domain::ProjectKey::new("t", "w", "p"),
                invocation_id: cairn_domain::ToolInvocationId::new("inv1"),
                session_id: Some(cairn_domain::SessionId::new("s1")),
                run_id: Some(cairn_domain::RunId::new("run1")),
                task_id: None,
                target: cairn_domain::tool_invocation::ToolInvocationTarget::Builtin {
                    tool_name: "memory_search".to_owned(),
                },
                execution_class: cairn_domain::policy::ExecutionClass::SupervisedProcess,
                prompt_release_id: None,
                requested_at_ms: 1700000000000,
                started_at_ms: 1700000000000,
            },
        );
        let spans = exporter.event_to_spans(&event);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "tool:memory_search");
        assert_eq!(spans[0].kind, "client");
    }

    #[test]
    fn memory_search_gets_retrieval_operation_name() {
        let config = OtlpConfig {
            enabled: true,
            ..Default::default()
        };
        let exporter = OtlpExporter::new(config, Box::new(NoOpSink));
        let event = cairn_domain::RuntimeEvent::ToolInvocationCompleted(
            cairn_domain::events::ToolInvocationCompleted {
                project: cairn_domain::ProjectKey::new("t", "w", "p"),
                invocation_id: cairn_domain::ToolInvocationId::new("inv1"),
                task_id: None,
                tool_name: "memory_search".to_owned(),
                finished_at_ms: 1700000000042,
                outcome: cairn_domain::tool_invocation::ToolInvocationOutcomeKind::Success,
                tool_call_id: None,
                result_json: None,
            },
        );
        let spans = exporter.event_to_spans(&event);
        assert_eq!(spans.len(), 1);
        match spans[0].attributes.get("gen_ai.operation.name") {
            Some(SpanAttributeValue::String(s)) => assert_eq!(s, "retrieval"),
            other => panic!("expected retrieval, got {:?}", other),
        }
    }

    #[test]
    fn provider_call_produces_genai_span() {
        let config = OtlpConfig {
            enabled: true,
            ..Default::default()
        };
        let exporter = OtlpExporter::new(config, Box::new(NoOpSink));
        let event = cairn_domain::RuntimeEvent::ProviderCallCompleted(
            cairn_domain::events::ProviderCallCompleted {
                project: cairn_domain::ProjectKey::new("t", "w", "p"),
                provider_call_id: cairn_domain::ProviderCallId::new("call1"),
                route_decision_id: cairn_domain::RouteDecisionId::new("rd1"),
                route_attempt_id: cairn_domain::RouteAttemptId::new("ra1"),
                provider_binding_id: cairn_domain::ProviderBindingId::new("pb1"),
                provider_connection_id: cairn_domain::ProviderConnectionId::new("pc1"),
                provider_model_id: cairn_domain::ProviderModelId::new("gpt-4"),
                operation_kind: cairn_domain::providers::OperationKind::Generate,
                status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                latency_ms: Some(500),
                input_tokens: Some(500),
                output_tokens: Some(200),
                cost_micros: Some(150),
                completed_at: 1700000000500,
                session_id: None,
                run_id: Some(cairn_domain::RunId::new("run1")),
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                task_id: None,
                prompt_release_id: None,
                fallback_position: 0,
                started_at: 1700000000000,
                finished_at: 1700000000500,
            },
        );
        let spans = exporter.event_to_spans(&event);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "llm:gpt-4");
        assert_eq!(spans[0].status, "ok");
        match spans[0].attributes.get("gen_ai.usage.input_tokens") {
            Some(SpanAttributeValue::Int(n)) => assert_eq!(*n, 500),
            other => panic!("expected 500, got {:?}", other),
        }
    }

    #[test]
    fn unhandled_event_produces_no_spans() {
        let config = OtlpConfig {
            enabled: true,
            ..Default::default()
        };
        let exporter = OtlpExporter::new(config, Box::new(NoOpSink));
        let event =
            cairn_domain::RuntimeEvent::SessionCreated(cairn_domain::events::SessionCreated {
                project: cairn_domain::ProjectKey::new("t", "w", "p"),
                session_id: cairn_domain::SessionId::new("s1"),
            });
        assert!(exporter.event_to_spans(&event).is_empty());
    }

    #[test]
    fn hash_id_is_deterministic() {
        assert_eq!(hash_id("run_1"), hash_id("run_1"));
        assert_ne!(hash_id("run_1"), hash_id("run_2"));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "hello 🌍 world";
        let t = truncate(s, 8);
        assert!(t.len() <= 12); // 8 bytes + "…" (3 bytes)
        assert!(t.ends_with('…'));
    }
}
