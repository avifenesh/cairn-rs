//! Control plane protocol types (RFC 021).
//!
//! SQ/EQ (Submission Queue / Event Queue), A2A (Agent-to-Agent),
//! and OTLP configuration types.

use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

// ── SQ/EQ Protocol Types ─────────────────────────────────────────────────────

/// Client information sent during SQ/EQ Initialize.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqEqClientInfo {
    pub name: String,
    pub version: String,
}

/// Subscription preferences for the SQ/EQ event stream.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqEqSubscriptions {
    /// Event type glob patterns (e.g. "run.*", "task.*", "decision.*").
    #[serde(default)]
    pub event_types: Vec<String>,
    /// Whether to include reasoning chains in events.
    #[serde(default)]
    pub include_reasoning: Option<String>,
    /// Whether to exclude internal-only events.
    #[serde(default)]
    pub exclude_internal: bool,
}

/// `POST /v1/sqeq/initialize` request body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqEqInitializeRequest {
    pub protocol_versions: Vec<String>,
    #[serde(default)]
    pub client: Option<SqEqClientInfo>,
    pub scope: ProjectKey,
    #[serde(default)]
    pub subscriptions: SqEqSubscriptions,
}

/// `POST /v1/sqeq/initialize` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqEqInitializeResponse {
    pub negotiated_version: String,
    pub sqeq_session_id: String,
    pub bound_scope: ProjectKey,
    pub include_reasoning: String,
    pub capabilities: SqEqCapabilities,
}

/// Server capabilities reported in the Initialize response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqEqCapabilities {
    pub supported_commands: Vec<String>,
    pub supported_events: Vec<String>,
    pub supports_replay: bool,
    pub max_event_buffer: u64,
}

/// `POST /v1/sqeq/submit` request body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqEqSubmission {
    /// Command method name (e.g. "start_run", "pause_run", "resolve_approval").
    pub method: String,
    /// Client-generated correlation ID for cause-to-effect tracing.
    pub correlation_id: String,
    /// Command parameters (shape depends on method).
    pub params: serde_json::Value,
}

/// `POST /v1/sqeq/submit` synchronous response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqEqSubmissionAck {
    pub accepted: bool,
    pub correlation_id: String,
    /// The event sequence number where downstream effects will appear.
    pub projected_event_seq: Option<u64>,
    /// Error details if not accepted.
    pub error: Option<String>,
}

// ── A2A Types ────────────────────────────────────────────────────────────────

/// A2A Agent Card (served at `GET /.well-known/agent.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aAgentCard {
    pub a2a_version: String,
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub endpoints: A2aEndpoints,
    pub auth: A2aAuth,
    pub capabilities: A2aCapabilities,
    pub accepted_task_kinds: Vec<String>,
    pub supported_input_formats: Vec<String>,
    pub supported_output_formats: Vec<String>,
    pub transport: Vec<String>,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aEndpoints {
    pub task_submission: String,
    pub task_status: String,
    pub task_cancel: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aAuth {
    #[serde(rename = "type")]
    pub auth_type: String,
    pub docs_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aCapabilities {
    pub accepts_tasks: bool,
    pub delegates_tasks: bool,
    pub supports_streaming: bool,
    pub supports_push_notifications: bool,
}

/// A2A task submission request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aTaskSubmission {
    pub task: A2aTask,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aTask {
    pub kind: String,
    pub input: A2aTaskInput,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aTaskInput {
    pub content_type: String,
    pub content: String,
}

/// A2A task submission response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct A2aTaskResponse {
    pub task_id: String,
    pub status: String,
    pub status_url: String,
}

// ── OTLP Configuration ──────────────────────────────────────────────────────

/// OTLP export protocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtlpProtocol {
    Grpc,
    HttpBinary,
    HttpJson,
}

/// OTLP exporter configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OtlpConfig {
    /// Whether OTLP export is enabled.
    pub enabled: bool,
    /// OTLP endpoint URL.
    pub endpoint: String,
    /// Export protocol.
    pub protocol: OtlpProtocol,
    /// Whether to redact message content, tool args, and model responses.
    /// `true` by default (privacy-safe).
    pub redact_content: bool,
    /// Service name reported in spans.
    pub service_name: String,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:4317".to_owned(),
            protocol: OtlpProtocol::Grpc,
            redact_content: true,
            service_name: "cairn".to_owned(),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqeq_initialize_request_serde() {
        let req = SqEqInitializeRequest {
            protocol_versions: vec!["1.0".into()],
            client: Some(SqEqClientInfo {
                name: "test-client".into(),
                version: "0.1.0".into(),
            }),
            scope: ProjectKey::new("t", "w", "p"),
            subscriptions: SqEqSubscriptions {
                event_types: vec!["run.*".into(), "task.*".into()],
                include_reasoning: Some("requested".into()),
                exclude_internal: true,
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SqEqInitializeRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn sqeq_submission_serde() {
        let sub = SqEqSubmission {
            method: "start_run".into(),
            correlation_id: "corr_1".into(),
            params: serde_json::json!({"run_id": "run_1", "mode": "direct"}),
        };
        let json = serde_json::to_string(&sub).unwrap();
        let parsed: SqEqSubmission = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, sub);
    }

    #[test]
    fn a2a_agent_card_serializes() {
        let card = A2aAgentCard {
            a2a_version: "0.3".into(),
            agent_id: "urn:cairn:self-hosted:tenant:acme".into(),
            name: "Cairn Control Plane".into(),
            description: "test".into(),
            endpoints: A2aEndpoints {
                task_submission: "/v1/a2a/tasks".into(),
                task_status: "/v1/a2a/tasks/{task_id}".into(),
                task_cancel: "/v1/a2a/tasks/{task_id}/cancel".into(),
            },
            auth: A2aAuth {
                auth_type: "bearer".into(),
                docs_url: "https://docs.cairn.dev/a2a/auth".into(),
            },
            capabilities: A2aCapabilities {
                accepts_tasks: true,
                delegates_tasks: true,
                supports_streaming: true,
                supports_push_notifications: false,
            },
            accepted_task_kinds: vec!["research".into()],
            supported_input_formats: vec!["text/markdown".into()],
            supported_output_formats: vec!["text/markdown".into()],
            transport: vec!["https".into()],
            version: "0.1.0".into(),
        };
        let json = serde_json::to_string_pretty(&card).unwrap();
        assert!(json.contains("a2a_version"));
        assert!(json.contains("Cairn Control Plane"));
    }

    #[test]
    fn otlp_config_defaults() {
        let cfg = OtlpConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.redact_content);
        assert_eq!(cfg.protocol, OtlpProtocol::Grpc);
    }
}
