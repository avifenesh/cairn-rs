//! Runtime-facing tool invocation service.
//!
//! Single entry point the runtime calls to execute a tool through the
//! full durable pipeline. Wraps permission checking, execution, record
//! lifecycle, and event emission in one clean interface.

use async_trait::async_trait;
use cairn_domain::ids::{RunId, SessionId, TaskId, ToolInvocationId};
use cairn_domain::policy::ExecutionClass;
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::tool_invocation::{ToolInvocationRecord, ToolInvocationTarget};
use serde::{Deserialize, Serialize};

/// Request from the runtime to invoke a tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeToolRequest {
    pub invocation_id: ToolInvocationId,
    pub plugin_id: Option<String>,
    pub project: ProjectKey,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub target: ToolInvocationTarget,
    pub execution_class: ExecutionClass,
    pub params: serde_json::Value,
    pub requested_at_ms: u64,
}

/// Result returned to the runtime after tool invocation completes.
#[derive(Clone, Debug)]
pub struct RuntimeToolResponse {
    /// All record snapshots produced during the invocation lifecycle.
    pub records: Vec<ToolInvocationRecord>,
    /// The terminal outcome classification.
    pub outcome: RuntimeToolOutcome,
    /// SSE-ready lifecycle output for `assistant_tool_call` shaping.
    pub lifecycle: ToolLifecycleOutput,
}

/// Consistent tool lifecycle fields for SSE payload shaping.
///
/// Worker 8's `assistant_tool_call` SSE payload needs `toolName`, `phase`,
/// `args`, `result`, and failure detail. This struct carries those fields
/// from the tool execution layer so the API layer doesn't need to
/// reverse-engineer them from records.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLifecycleOutput {
    pub tool_name: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
}

/// Runtime-level outcome that the runtime uses for control flow.
///
/// This stays downstream in `cairn-tools` on purpose instead of moving
/// into `cairn-domain`: it drives runtime/tool orchestration decisions
/// like pause-vs-fail and can carry richer execution-layer distinctions
/// (`PermissionDenied`, `HeldForApproval`) than the durable shared
/// `ToolInvocationOutcomeKind` contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeToolOutcome {
    /// Tool ran and produced output.
    Success,
    /// Tool ran but failed (retryable or permanent).
    Failed { retryable: bool, reason: String },
    /// Tool was canceled.
    Canceled,
    /// Tool timed out.
    Timeout,
    /// Permission denied before execution.
    PermissionDenied { reason: String },
    /// Held for operator approval — runtime should pause the task.
    HeldForApproval { reason: String },
}

impl RuntimeToolOutcome {
    pub fn should_pause_task(&self) -> bool {
        matches!(self, RuntimeToolOutcome::HeldForApproval { .. })
    }

    pub fn is_success(&self) -> bool {
        matches!(self, RuntimeToolOutcome::Success)
    }

    pub fn is_terminal_failure(&self) -> bool {
        matches!(
            self,
            RuntimeToolOutcome::Failed {
                retryable: false,
                ..
            } | RuntimeToolOutcome::PermissionDenied { .. }
        )
    }
}

impl ToolLifecycleOutput {
    pub fn started(tool_name: impl Into<String>, args: Option<serde_json::Value>) -> Self {
        Self {
            tool_name: tool_name.into(),
            phase: "start".to_owned(),
            args,
            result: None,
            error_detail: None,
        }
    }

    pub fn completed(tool_name: impl Into<String>, result: Option<serde_json::Value>) -> Self {
        Self {
            tool_name: tool_name.into(),
            phase: "completed".to_owned(),
            args: None,
            result,
            error_detail: None,
        }
    }

    pub fn failed(tool_name: impl Into<String>, error_detail: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            phase: "failed".to_owned(),
            args: None,
            result: None,
            error_detail: Some(error_detail.into()),
        }
    }
}

/// Service trait the runtime calls to execute tools.
///
/// Implementors wire this to the pipeline, permission gate, tool host,
/// and store. The runtime only needs to know about this trait.
#[async_trait]
pub trait RuntimeToolService: Send + Sync {
    /// Execute a tool through the full durable pipeline.
    ///
    /// The implementor is responsible for:
    /// 1. Permission checking
    /// 2. Record creation and persistence
    /// 3. Tool execution (builtin or plugin)
    /// 4. Record finalization
    /// 5. Event emission for graph/SSE consumers
    async fn invoke(
        &self,
        request: RuntimeToolRequest,
    ) -> Result<RuntimeToolResponse, Box<dyn std::error::Error + Send + Sync>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_for_approval_pauses_task() {
        let outcome = RuntimeToolOutcome::HeldForApproval {
            reason: "needs review".to_owned(),
        };
        assert!(outcome.should_pause_task());
        assert!(!outcome.is_success());
    }

    #[test]
    fn success_does_not_pause() {
        assert!(!RuntimeToolOutcome::Success.should_pause_task());
        assert!(RuntimeToolOutcome::Success.is_success());
    }

    #[test]
    fn permission_denied_is_terminal() {
        let outcome = RuntimeToolOutcome::PermissionDenied {
            reason: "blocked".to_owned(),
        };
        assert!(outcome.is_terminal_failure());
        assert!(!outcome.should_pause_task());
    }

    #[test]
    fn retryable_failure_is_not_terminal() {
        let outcome = RuntimeToolOutcome::Failed {
            retryable: true,
            reason: "transient".to_owned(),
        };
        assert!(!outcome.is_terminal_failure());
    }

    #[test]
    fn non_retryable_failure_is_terminal() {
        let outcome = RuntimeToolOutcome::Failed {
            retryable: false,
            reason: "bad input".to_owned(),
        };
        assert!(outcome.is_terminal_failure());
    }

    #[test]
    fn runtime_tool_request_construction() {
        let req = RuntimeToolRequest {
            invocation_id: ToolInvocationId::new("inv_1"),
            plugin_id: None,
            project: ProjectKey::new("t", "w", "p"),
            session_id: Some(SessionId::new("sess_1")),
            run_id: Some(RunId::new("run_1")),
            task_id: Some(TaskId::new("task_1")),
            target: ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            params: serde_json::json!({"path": "/tmp"}),
            requested_at_ms: 1000,
        };
        assert_eq!(req.invocation_id.as_str(), "inv_1");
        assert_eq!(req.session_id.unwrap().as_str(), "sess_1");
        assert_eq!(req.run_id.unwrap().as_str(), "run_1");
        assert_eq!(req.task_id.unwrap().as_str(), "task_1");
    }

    #[test]
    fn lifecycle_output_started_has_correct_phase() {
        let output =
            ToolLifecycleOutput::started("fs.read", Some(serde_json::json!({"path": "/tmp"})));
        assert_eq!(output.tool_name, "fs.read");
        assert_eq!(output.phase, "start");
        assert!(output.args.is_some());
        assert!(output.result.is_none());
        assert!(output.error_detail.is_none());
    }

    #[test]
    fn lifecycle_output_completed_has_result() {
        let output =
            ToolLifecycleOutput::completed("fs.read", Some(serde_json::json!({"text": "ok"})));
        assert_eq!(output.phase, "completed");
        assert!(output.result.is_some());
    }

    #[test]
    fn lifecycle_output_failed_has_error_detail() {
        let output = ToolLifecycleOutput::failed("fs.read", "file not found");
        assert_eq!(output.phase, "failed");
        assert_eq!(output.error_detail, Some("file not found".to_owned()));
    }

    #[test]
    fn lifecycle_output_serializes_to_camel_case() {
        let output = ToolLifecycleOutput::started("git.status", None);
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["toolName"], "git.status");
        assert_eq!(json["phase"], "start");
        // args should be omitted when None (skip_serializing_if)
        assert!(json.get("args").is_none());
    }

    #[test]
    fn lifecycle_payloads_are_idempotent_under_repeated_construction() {
        // Proves that constructing the same lifecycle output multiple times
        // always produces identical serialized JSON — safe under rapid churn.
        let args = serde_json::json!({"path": "/repo"});
        for _ in 0..10 {
            let a = ToolLifecycleOutput::started("git.status", Some(args.clone()));
            let b = ToolLifecycleOutput::started("git.status", Some(args.clone()));
            assert_eq!(
                serde_json::to_string(&a).unwrap(),
                serde_json::to_string(&b).unwrap(),
            );
        }

        for _ in 0..10 {
            let a = ToolLifecycleOutput::completed(
                "git.status",
                Some(serde_json::json!({"clean": true})),
            );
            let b = ToolLifecycleOutput::completed(
                "git.status",
                Some(serde_json::json!({"clean": true})),
            );
            assert_eq!(
                serde_json::to_string(&a).unwrap(),
                serde_json::to_string(&b).unwrap(),
            );
        }

        for _ in 0..10 {
            let a = ToolLifecycleOutput::failed("git.push", "rejected");
            let b = ToolLifecycleOutput::failed("git.push", "rejected");
            assert_eq!(
                serde_json::to_string(&a).unwrap(),
                serde_json::to_string(&b).unwrap(),
            );
        }
    }

    #[test]
    fn high_churn_mixed_outcomes_stay_coherent() {
        // Simulates rapid claim/complete churn: many tools, mixed outcomes,
        // interleaved construction. Every output must serialize identically
        // when constructed with the same inputs regardless of ordering.
        let tools = [
            "fs.read",
            "git.status",
            "db.query",
            "net.fetch",
            "shell.exec",
        ];

        for round in 0..100 {
            let tool = tools[round % tools.len()];
            let lifecycle = match round % 4 {
                0 => ToolLifecycleOutput::started(tool, Some(serde_json::json!({"round": round}))),
                1 => ToolLifecycleOutput::completed(tool, Some(serde_json::json!({"ok": true}))),
                2 => ToolLifecycleOutput::failed(tool, "error"),
                _ => ToolLifecycleOutput::started(tool, None),
            };

            let json = serde_json::to_string(&lifecycle).unwrap();

            // Reconstruct with same inputs — must be identical
            let lifecycle2 = match round % 4 {
                0 => ToolLifecycleOutput::started(tool, Some(serde_json::json!({"round": round}))),
                1 => ToolLifecycleOutput::completed(tool, Some(serde_json::json!({"ok": true}))),
                2 => ToolLifecycleOutput::failed(tool, "error"),
                _ => ToolLifecycleOutput::started(tool, None),
            };
            assert_eq!(
                json,
                serde_json::to_string(&lifecycle2).unwrap(),
                "round {round} tool {tool} not idempotent"
            );

            // Every JSON must parse back to valid structure
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed["toolName"], tool);
            assert!(["start", "completed", "failed"].contains(&parsed["phase"].as_str().unwrap()));
        }
    }
}
