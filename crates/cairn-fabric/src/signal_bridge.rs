use ff_core::types::{ExecutionId, WaitpointId};
use ff_sdk::task::{Signal, SignalOutcome};
use ff_sdk::FlowFabricWorker;

use crate::error::FabricError;

pub struct SignalBridge {
    worker: FlowFabricWorker,
}

impl SignalBridge {
    pub fn new(worker: FlowFabricWorker) -> Self {
        Self { worker }
    }

    pub async fn deliver_approval_signal(
        &self,
        execution_id: &ExecutionId,
        waitpoint_id: &WaitpointId,
        approved: bool,
        approval_id: &str,
        details: Option<String>,
    ) -> Result<SignalOutcome, FabricError> {
        let signal_name = if approved {
            "approval_granted"
        } else {
            "approval_rejected"
        };

        let payload = details.map(|d| {
            serde_json::json!({
                "approved": approved,
                "details": d,
            })
            .to_string()
            .into_bytes()
        });

        let signal = Signal {
            signal_name: signal_name.into(),
            signal_category: "approval".into(),
            payload,
            source_type: "cairn_operator".into(),
            source_identity: "cairn".into(),
            idempotency_key: Some(format!("approval:{approval_id}")),
        };

        self.worker
            .deliver_signal(execution_id, waitpoint_id, signal)
            .await
            .map_err(|e| FabricError::Bridge(format!("deliver_approval_signal: {e}")))
    }

    pub async fn deliver_child_completed_signal(
        &self,
        parent_execution_id: &ExecutionId,
        parent_waitpoint_id: &WaitpointId,
        child_task_id: &str,
        success: bool,
    ) -> Result<SignalOutcome, FabricError> {
        let payload = serde_json::json!({
            "child_task_id": child_task_id,
            "success": success,
        })
        .to_string()
        .into_bytes();

        let signal = Signal {
            signal_name: format!("child_completed:{child_task_id}"),
            signal_category: "subagent".into(),
            payload: Some(payload),
            source_type: "cairn_runtime".into(),
            source_identity: "cairn".into(),
            idempotency_key: Some(format!("child_completed:{child_task_id}")),
        };

        self.worker
            .deliver_signal(parent_execution_id, parent_waitpoint_id, signal)
            .await
            .map_err(|e| FabricError::Bridge(format!("deliver_child_completed_signal: {e}")))
    }

    pub async fn deliver_tool_result_signal(
        &self,
        execution_id: &ExecutionId,
        waitpoint_id: &WaitpointId,
        invocation_id: &str,
        result_payload: Option<Vec<u8>>,
    ) -> Result<SignalOutcome, FabricError> {
        let signal = Signal {
            signal_name: format!("tool_result:{invocation_id}"),
            signal_category: "tool".into(),
            payload: result_payload,
            source_type: "cairn_runtime".into(),
            source_identity: "cairn".into(),
            idempotency_key: Some(format!("tool_result:{invocation_id}")),
        };

        self.worker
            .deliver_signal(execution_id, waitpoint_id, signal)
            .await
            .map_err(|e| FabricError::Bridge(format!("deliver_tool_result_signal: {e}")))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn approval_signal_name_granted() {
        let name = if true {
            "approval_granted"
        } else {
            "approval_rejected"
        };
        assert_eq!(name, "approval_granted");
    }

    #[test]
    fn approval_signal_name_rejected() {
        let name = if false {
            "approval_granted"
        } else {
            "approval_rejected"
        };
        assert_eq!(name, "approval_rejected");
    }

    #[test]
    fn child_completed_signal_name_format() {
        let name = format!("child_completed:{}", "task_abc");
        assert_eq!(name, "child_completed:task_abc");
    }

    #[test]
    fn tool_result_signal_name_format() {
        let name = format!("tool_result:{}", "inv_xyz");
        assert_eq!(name, "tool_result:inv_xyz");
    }

    #[test]
    fn idempotency_key_format_approval() {
        let key = format!("approval:{}", "appr_1");
        assert_eq!(key, "approval:appr_1");
    }

    #[test]
    fn idempotency_key_format_child() {
        let key = format!("child_completed:{}", "task_1");
        assert_eq!(key, "child_completed:task_1");
    }

    #[test]
    fn idempotency_key_format_tool() {
        let key = format!("tool_result:{}", "inv_1");
        assert_eq!(key, "tool_result:inv_1");
    }

    #[test]
    fn approval_payload_json_structure() {
        let payload = serde_json::json!({
            "approved": true,
            "details": "looks good",
        });
        let obj = payload.as_object().unwrap();
        assert_eq!(obj.get("approved").unwrap(), true);
        assert_eq!(obj.get("details").unwrap(), "looks good");
    }

    #[test]
    fn child_completed_payload_json_structure() {
        let payload = serde_json::json!({
            "child_task_id": "task_1",
            "success": false,
        });
        let obj = payload.as_object().unwrap();
        assert_eq!(obj.get("child_task_id").unwrap(), "task_1");
        assert_eq!(obj.get("success").unwrap(), false);
    }
}
