use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 request envelope per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: String,
    pub method: String,
    pub params: serde_json::Value,
}

/// JSON-RPC 2.0 success response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: String,
    pub result: serde_json::Value,
}

/// JSON-RPC 2.0 error response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub jsonrpc: String,
    pub id: String,
    pub error: JsonRpcErrorBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcErrorBody {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 notification (no id, no response expected).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcRequest {
    pub fn new(
        id: impl Into<String>,
        method: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

impl JsonRpcResponse {
    pub fn new(id: impl Into<String>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id: id.into(),
            result,
        }
    }
}

/// Canonical host -> plugin method names per RFC 007.
pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const SHUTDOWN: &str = "shutdown";
    pub const HEALTH_CHECK: &str = "health.check";
    pub const TOOLS_LIST: &str = "tools.list";
    pub const TOOLS_INVOKE: &str = "tools.invoke";
    pub const SIGNALS_POLL: &str = "signals.poll";
    pub const CHANNELS_DELIVER: &str = "channels.deliver";
    pub const HOOKS_POST_TURN: &str = "hooks.post_turn";
    pub const POLICY_EVALUATE: &str = "policy.evaluate";
    pub const EVAL_SCORE: &str = "eval.score";
    pub const CANCEL: &str = "cancel";
}

/// Canonical plugin -> host notification methods per RFC 007.
pub mod notifications {
    pub const LOG_EMIT: &str = "log.emit";
    pub const PROGRESS_UPDATE: &str = "progress.update";
    pub const EVENT_EMIT: &str = "event.emit";
}

/// `initialize` request params.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub host: HostInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub name: String,
    pub version: String,
}

/// `initialize` response result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub plugin: PluginInfo,
    pub capabilities: Vec<serde_json::Value>,
    #[serde(default)]
    pub limits: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// `tools.invoke` request params.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsInvokeParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub input: serde_json::Value,
    pub scope: ScopeWire,
    #[serde(default)]
    pub actor: Option<ActorWire>,
    #[serde(default)]
    pub runtime: Option<RuntimeLinkageWire>,
    #[serde(default)]
    pub grants: Vec<String>,
}

/// `tools.invoke` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsInvokeResult {
    pub status: String,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub events: Vec<serde_json::Value>,
}

/// Scope object per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeWire {
    #[serde(rename = "tenantId")]
    pub tenant_id: String,
    #[serde(rename = "workspaceId", default)]
    pub workspace_id: Option<String>,
    #[serde(rename = "projectId", default)]
    pub project_id: Option<String>,
}

/// Actor object per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorWire {
    #[serde(rename = "operatorId")]
    pub operator_id: String,
}

/// Runtime linkage per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeLinkageWire {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "taskId", default)]
    pub task_id: Option<String>,
}

/// `tools.list` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsListResult {
    pub tools: Vec<ToolDescriptorWire>,
}

/// Tool descriptor as returned by the plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescriptorWire {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// `signals.poll` request params per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalsPollParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    pub source: serde_json::Value,
    pub scope: ScopeWire,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// `signals.poll` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalsPollResult {
    pub status: String,
    #[serde(default)]
    pub events: Vec<serde_json::Value>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// `channels.deliver` request params per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelsDeliverParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    pub channel: serde_json::Value,
    pub message: serde_json::Value,
    #[serde(default)]
    pub recipients: Vec<serde_json::Value>,
    pub scope: ScopeWire,
}

/// `channels.deliver` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelsDeliverResult {
    pub status: String,
    #[serde(rename = "deliveryIds", default)]
    pub delivery_ids: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// `hooks.post_turn` request params per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HooksPostTurnParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    pub scope: ScopeWire,
    #[serde(default)]
    pub runtime: Option<RuntimeLinkageWire>,
    pub turn: serde_json::Value,
}

/// `hooks.post_turn` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HooksPostTurnResult {
    pub status: String,
    #[serde(default)]
    pub findings: Vec<serde_json::Value>,
    #[serde(default)]
    pub patches: Vec<serde_json::Value>,
}

/// `policy.evaluate` request params per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEvaluateParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    pub scope: ScopeWire,
    #[serde(default)]
    pub actor: Option<ActorWire>,
    pub action: serde_json::Value,
    #[serde(default)]
    pub context: serde_json::Value,
}

/// `policy.evaluate` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEvaluateResult {
    pub decision: String,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(rename = "appliedPolicies", default)]
    pub applied_policies: Vec<serde_json::Value>,
}

/// `eval.score` request params per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalScoreParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    pub scope: ScopeWire,
    pub target: serde_json::Value,
    #[serde(default)]
    pub dataset: Option<serde_json::Value>,
    #[serde(default)]
    pub samples: Vec<serde_json::Value>,
}

/// `eval.score` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalScoreResult {
    pub status: String,
    #[serde(default)]
    pub scores: Vec<serde_json::Value>,
    #[serde(default)]
    pub summary: serde_json::Value,
}

/// `cancel` request params per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
}

/// `cancel` success result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelResult {
    pub status: String,
}

// --- Plugin -> Host notification payloads (RFC 007) ---

/// Typed payload for `log.emit` notifications.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEmitParams {
    pub level: String,
    pub message: String,
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    #[serde(default)]
    pub fields: Option<serde_json::Value>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

/// Typed payload for `progress.update` notifications.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressUpdateParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    pub message: String,
    #[serde(default)]
    pub percent: Option<u32>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(rename = "etaMs", default)]
    pub eta_ms: Option<u64>,
}

/// Typed payload for `event.emit` notifications.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEmitParams {
    #[serde(rename = "invocationId")]
    pub invocation_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: serde_json::Value,
    #[serde(rename = "externalId", default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

/// Typed wrapper for all plugin -> host notifications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginNotification {
    LogEmit(LogEmitParams),
    ProgressUpdate(ProgressUpdateParams),
    EventEmit(EventEmitParams),
}

impl PluginNotification {
    /// Parse a raw JsonRpcNotification into a typed PluginNotification.
    pub fn from_raw(notification: &JsonRpcNotification) -> Option<Self> {
        match notification.method.as_str() {
            notifications::LOG_EMIT => serde_json::from_value(notification.params.clone())
                .ok()
                .map(PluginNotification::LogEmit),
            notifications::PROGRESS_UPDATE => {
                serde_json::from_value(notification.params.clone())
                    .ok()
                    .map(PluginNotification::ProgressUpdate)
            }
            notifications::EVENT_EMIT => serde_json::from_value(notification.params.clone())
                .ok()
                .map(PluginNotification::EventEmit),
            _ => None,
        }
    }

    /// Get the invocation ID from any notification variant.
    pub fn invocation_id(&self) -> &str {
        match self {
            PluginNotification::LogEmit(p) => &p.invocation_id,
            PluginNotification::ProgressUpdate(p) => &p.invocation_id,
            PluginNotification::EventEmit(p) => &p.invocation_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonrpc_request_construction() {
        let req = JsonRpcRequest::new("req_1", methods::TOOLS_INVOKE, serde_json::json!({}));
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "tools.invoke");
    }

    #[test]
    fn tools_invoke_params_roundtrip() {
        let params = ToolsInvokeParams {
            invocation_id: "inv_1".to_owned(),
            tool_name: "git.status".to_owned(),
            input: serde_json::json!({}),
            scope: ScopeWire {
                tenant_id: "t1".to_owned(),
                workspace_id: Some("w1".to_owned()),
                project_id: Some("p1".to_owned()),
            },
            actor: Some(ActorWire {
                operator_id: "u1".to_owned(),
            }),
            runtime: Some(RuntimeLinkageWire {
                session_id: "s1".to_owned(),
                run_id: "r1".to_owned(),
                task_id: Some("t1".to_owned()),
            }),
            grants: vec!["fs.read".to_owned(), "process.exec".to_owned()],
        };

        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["toolName"], "git.status");
        assert_eq!(json["scope"]["tenantId"], "t1");

        let back: ToolsInvokeParams = serde_json::from_value(json).unwrap();
        assert_eq!(back.grants.len(), 2);
    }

    #[test]
    fn initialize_roundtrip() {
        let params = InitializeParams {
            protocol_version: "1.0".to_owned(),
            host: HostInfo {
                name: "cairn".to_owned(),
                version: "0.1.0".to_owned(),
            },
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["protocolVersion"], "1.0");
        assert_eq!(json["host"]["name"], "cairn");
    }
}
