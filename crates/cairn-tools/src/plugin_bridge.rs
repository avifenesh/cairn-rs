use cairn_domain::tenancy::ProjectKey;
use cairn_plugin_proto::wire::{
    methods, HostInfo, InitializeParams, JsonRpcRequest, ScopeWire, ToolsInvokeParams,
    ToolsInvokeResult,
};

use crate::builtin::ToolOutcome;

/// Builds the `initialize` JSON-RPC request for a plugin handshake.
pub fn build_initialize_request(request_id: &str) -> JsonRpcRequest {
    let params = InitializeParams {
        protocol_version: "1.0".to_owned(),
        host: HostInfo {
            name: "cairn".to_owned(),
            version: "0.1.0".to_owned(),
        },
    };
    JsonRpcRequest::new(
        request_id,
        methods::INITIALIZE,
        serde_json::to_value(&params).unwrap_or_default(),
    )
}

/// Builds the `shutdown` JSON-RPC request.
pub fn build_shutdown_request(request_id: &str) -> JsonRpcRequest {
    JsonRpcRequest::new(request_id, methods::SHUTDOWN, serde_json::json!({}))
}

/// Builds a `tools.list` JSON-RPC request.
pub fn build_tools_list_request(request_id: &str) -> JsonRpcRequest {
    JsonRpcRequest::new(request_id, methods::TOOLS_LIST, serde_json::json!({}))
}

/// Builds a `tools.invoke` JSON-RPC request from host-side types.
pub fn build_tools_invoke_request(
    request_id: &str,
    invocation_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    project: &ProjectKey,
    grants: &[String],
) -> JsonRpcRequest {
    let params = ToolsInvokeParams {
        invocation_id: invocation_id.to_owned(),
        tool_name: tool_name.to_owned(),
        input,
        scope: ScopeWire {
            tenant_id: project.tenant_id.to_string(),
            workspace_id: Some(project.workspace_id.to_string()),
            project_id: Some(project.project_id.to_string()),
        },
        actor: None,
        runtime: None,
        grants: grants.to_vec(),
    };
    JsonRpcRequest::new(
        request_id,
        methods::TOOLS_INVOKE,
        serde_json::to_value(&params).unwrap_or_default(),
    )
}

/// Converts a plugin `tools.invoke` result to a host-side `ToolOutcome`.
pub fn invoke_result_to_outcome(result: &ToolsInvokeResult) -> ToolOutcome {
    match result.status.as_str() {
        "success" => ToolOutcome::Success {
            output: result.output.clone().unwrap_or(serde_json::Value::Null),
        },
        "timeout" => ToolOutcome::Timeout,
        "canceled" => ToolOutcome::Canceled,
        other => ToolOutcome::PermanentFailure {
            reason: format!("plugin returned status: {other}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_plugin_proto::wire::{methods, ToolsInvokeResult};

    #[test]
    fn initialize_request_shape() {
        let req = build_initialize_request("req_1");
        assert_eq!(req.method, methods::INITIALIZE);
        assert_eq!(req.params["protocolVersion"], "1.0");
        assert_eq!(req.params["host"]["name"], "cairn");
    }

    #[test]
    fn tools_invoke_request_carries_scope() {
        let req = build_tools_invoke_request(
            "req_2",
            "inv_1",
            "git.status",
            serde_json::json!({}),
            &ProjectKey::new("t1", "w1", "p1"),
            &["fs.read".to_owned()],
        );
        assert_eq!(req.method, methods::TOOLS_INVOKE);
        assert_eq!(req.params["scope"]["tenantId"], "t1");
        assert_eq!(req.params["toolName"], "git.status");
    }

    #[test]
    fn invoke_result_success_to_outcome() {
        let result = ToolsInvokeResult {
            status: "success".to_owned(),
            output: Some(serde_json::json!({"text": "clean"})),
            events: vec![],
        };
        let outcome = invoke_result_to_outcome(&result);
        assert!(outcome.is_success());
    }

    #[test]
    fn invoke_result_timeout_to_outcome() {
        let result = ToolsInvokeResult {
            status: "timeout".to_owned(),
            output: None,
            events: vec![],
        };
        let outcome = invoke_result_to_outcome(&result);
        assert!(matches!(outcome, ToolOutcome::Timeout));
    }

    #[test]
    fn invoke_result_unknown_status_to_permanent_failure() {
        let result = ToolsInvokeResult {
            status: "unknown_error".to_owned(),
            output: None,
            events: vec![],
        };
        let outcome = invoke_result_to_outcome(&result);
        assert!(outcome.is_terminal_failure());
    }
}
