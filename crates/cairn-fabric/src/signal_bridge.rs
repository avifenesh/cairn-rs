use std::sync::Arc;

use ferriskey::Client;
use ff_core::partition::PartitionConfig;
use ff_core::types::{ExecutionId, SignalId, TimestampMs, WaitpointId};
use ff_sdk::task::{Signal, SignalOutcome};

use crate::boot::FabricRuntime;
use crate::error::FabricError;

pub struct SignalBridge {
    client: Client,
    partition_config: PartitionConfig,
}

impl SignalBridge {
    pub fn new(runtime: &Arc<FabricRuntime>) -> Self {
        Self {
            client: runtime.client.clone(),
            partition_config: runtime.partition_config,
        }
    }

    pub fn from_parts(client: Client, partition_config: PartitionConfig) -> Self {
        Self {
            client,
            partition_config,
        }
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
            format!("approval_granted:{approval_id}")
        } else {
            format!("approval_rejected:{approval_id}")
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
            signal_name,
            signal_category: "approval".into(),
            payload,
            source_type: "cairn_operator".into(),
            source_identity: "cairn".into(),
            idempotency_key: Some(format!("approval:{approval_id}")),
        };

        self.deliver_signal(execution_id, waitpoint_id, signal)
            .await
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

        self.deliver_signal(parent_execution_id, parent_waitpoint_id, signal)
            .await
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

        self.deliver_signal(execution_id, waitpoint_id, signal)
            .await
    }

    async fn deliver_signal(
        &self,
        execution_id: &ExecutionId,
        waitpoint_id: &WaitpointId,
        signal: Signal,
    ) -> Result<SignalOutcome, FabricError> {
        let partition =
            ff_core::partition::execution_partition(execution_id, &self.partition_config);
        let ctx = ff_core::keys::ExecKeyContext::new(&partition, execution_id);
        let idx = ff_core::keys::IndexKeys::new(&partition);

        let signal_id = SignalId::new();
        let now = TimestampMs::now();

        let lane_str: Option<String> = self
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = ff_core::types::LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let idem_key = if let Some(ref ik) = signal.idempotency_key {
            ctx.signal_dedup(waitpoint_id, ik)
        } else {
            ctx.noop()
        };

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.waitpoint_condition(waitpoint_id),
            ctx.waitpoint_signals(waitpoint_id),
            ctx.exec_signals(),
            ctx.signal(&signal_id),
            ctx.signal_payload(&signal_id),
            idem_key,
            ctx.waitpoint(waitpoint_id),
            ctx.suspension_current(),
            idx.lane_eligible(&lane_id),
            idx.lane_suspended(&lane_id),
            idx.lane_delayed(&lane_id),
            idx.suspension_timeout(),
        ];

        let payload_str = signal
            .payload
            .as_ref()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .unwrap_or_default();

        let args: Vec<String> = vec![
            signal_id.to_string(),
            execution_id.to_string(),
            waitpoint_id.to_string(),
            signal.signal_name,
            signal.signal_category,
            signal.source_type,
            signal.source_identity,
            payload_str,
            "json".to_owned(),
            signal.idempotency_key.unwrap_or_default(),
            String::new(),
            "waitpoint".to_owned(),
            now.to_string(),
            "86400000".to_owned(),
            "0".to_owned(),
            "1000".to_owned(),
            "10000".to_owned(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .client
            .fcall("ff_deliver_signal", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Bridge(format!("ff_deliver_signal: {e}")))?;

        parse_signal_result(&raw)
    }
}

fn parse_signal_result(raw: &ferriskey::Value) -> Result<SignalOutcome, FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => return Err(FabricError::Bridge("deliver_signal: expected Array".into())),
    };

    let status = match arr.first() {
        Some(Ok(ferriskey::Value::Int(n))) => *n,
        _ => return Err(FabricError::Bridge("deliver_signal: bad status".into())),
    };

    if status != 1 {
        let code = extract_str(arr, 1).unwrap_or_else(|| "unknown".into());
        return Err(FabricError::Bridge(format!(
            "deliver_signal rejected: {code}"
        )));
    }

    let sub = extract_str(arr, 1).unwrap_or_default();

    if sub == "DUPLICATE" {
        let existing_id = extract_str(arr, 2).unwrap_or_default();
        return Ok(SignalOutcome::Duplicate {
            existing_signal_id: existing_id,
        });
    }

    let signal_id_str = extract_str(arr, 2).unwrap_or_default();
    let effect = extract_str(arr, 3).unwrap_or_default();
    let signal_id = ff_core::types::SignalId::parse(&signal_id_str)
        .map_err(|e| FabricError::Bridge(format!("bad signal_id in response: {e}")))?;

    if effect == "resume_condition_satisfied" {
        Ok(SignalOutcome::TriggeredResume { signal_id })
    } else {
        Ok(SignalOutcome::Accepted { signal_id, effect })
    }
}

fn extract_str(arr: &[Result<ferriskey::Value, ferriskey::Error>], idx: usize) -> Option<String> {
    arr.get(idx).and_then(|v| match v {
        Ok(ferriskey::Value::BulkString(b)) => Some(String::from_utf8_lossy(b).into_owned()),
        Ok(ferriskey::Value::SimpleString(s)) => Some(s.clone()),
        Ok(ferriskey::Value::Int(n)) => Some(n.to_string()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn approval_signal_name_granted() {
        let id = "appr_1";
        let name = format!("approval_granted:{id}");
        assert_eq!(name, "approval_granted:appr_1");
    }

    #[test]
    fn approval_signal_name_rejected() {
        let id = "appr_2";
        let name = format!("approval_rejected:{id}");
        assert_eq!(name, "approval_rejected:appr_2");
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
