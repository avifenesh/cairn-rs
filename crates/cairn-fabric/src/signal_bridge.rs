use std::sync::Arc;

use ff_core::keys::ExecKeyContext;
use ff_core::types::{ExecutionId, SignalId, TimestampMs, WaitpointId, WaitpointToken};
use ff_sdk::task::{Signal, SignalOutcome};

use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::helpers::sanitize_signal_component;

/// Read the HMAC waitpoint token from FF's waitpoint hash.
///
/// FF mints the token during `ff_suspend_execution` and writes it to the
/// `waitpoint_token` field of the waitpoint hash (see lua/suspension.lua
/// line 185). It is the ONLY source of truth — cairn never caches it.
///
/// Returns `Err(Validation)` ONLY when the field is missing or empty — i.e.
/// the waitpoint hash has never been written, or was deleted. FF does NOT
/// clear `waitpoint_token` on close (audit retention): a closed waitpoint
/// still has its token, so this helper returns `Ok(token)` for it and the
/// downstream `ff_deliver_signal` reply surfaces `waitpoint_closed` at the
/// state boundary where it belongs. That separation matters — mixing
/// "waitpoint never existed" with "waitpoint is closed" at the auth layer
/// would re-create the exact oracle FF's Lua took pains to eliminate.
pub(crate) async fn read_waitpoint_token(
    client: &ferriskey::Client,
    ctx: &ExecKeyContext,
    waitpoint_id: &WaitpointId,
) -> Result<WaitpointToken, FabricError> {
    let token_str: Option<String> = client
        .hget(&ctx.waitpoint(waitpoint_id), "waitpoint_token")
        .await
        .map_err(|e| FabricError::Valkey(format!("HGET waitpoint_token: {e}")))?;
    match token_str {
        Some(s) if !s.is_empty() => Ok(WaitpointToken::new(s)),
        _ => Err(FabricError::Validation {
            reason: format!("waitpoint {waitpoint_id} is not active (missing token)"),
        }),
    }
}

pub struct SignalBridge {
    runtime: Arc<FabricRuntime>,
}

impl SignalBridge {
    pub fn new(runtime: &Arc<FabricRuntime>) -> Self {
        Self {
            runtime: runtime.clone(),
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
        let safe_id = sanitize_signal_component(approval_id);
        let signal_name = if approved {
            format!("approval_granted:{safe_id}")
        } else {
            format!("approval_rejected:{safe_id}")
        };

        let payload = details.map(|d| {
            serde_json::json!({
                "approved": approved,
                "details": d,
            })
            .to_string()
            .into_bytes()
        });

        let partition =
            ff_core::partition::execution_partition(execution_id, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, execution_id);
        let waitpoint_token =
            read_waitpoint_token(&self.runtime.client, &ctx, waitpoint_id).await?;

        let signal = Signal {
            signal_name,
            signal_category: "approval".into(),
            payload,
            source_type: crate::constants::SOURCE_TYPE_APPROVAL_OPERATOR.into(),
            source_identity: "cairn".into(),
            idempotency_key: Some(format!("approval:{safe_id}")),
            waitpoint_token,
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

        let safe_id = sanitize_signal_component(child_task_id);
        let partition = ff_core::partition::execution_partition(
            parent_execution_id,
            &self.runtime.partition_config,
        );
        let ctx = ExecKeyContext::new(&partition, parent_execution_id);
        let waitpoint_token =
            read_waitpoint_token(&self.runtime.client, &ctx, parent_waitpoint_id).await?;

        let signal = Signal {
            signal_name: format!("child_completed:{safe_id}"),
            signal_category: "subagent".into(),
            payload: Some(payload),
            source_type: crate::constants::SOURCE_TYPE_RUNTIME.into(),
            source_identity: "cairn".into(),
            idempotency_key: Some(format!("child_completed:{safe_id}")),
            waitpoint_token,
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
        let safe_id = sanitize_signal_component(invocation_id);
        let partition =
            ff_core::partition::execution_partition(execution_id, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, execution_id);
        let waitpoint_token =
            read_waitpoint_token(&self.runtime.client, &ctx, waitpoint_id).await?;

        let signal = Signal {
            signal_name: format!("tool_result:{safe_id}"),
            signal_category: "tool".into(),
            payload: result_payload,
            source_type: crate::constants::SOURCE_TYPE_RUNTIME.into(),
            source_identity: "cairn".into(),
            idempotency_key: Some(format!("tool_result:{safe_id}")),
            waitpoint_token,
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
            ff_core::partition::execution_partition(execution_id, &self.runtime.partition_config);
        let ctx = ff_core::keys::ExecKeyContext::new(&partition, execution_id);
        let idx = ff_core::keys::IndexKeys::new(&partition);

        let signal_id = SignalId::new();
        let now = TimestampMs::now();

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .map_err(|e| FabricError::Valkey(format!("HGET lane_id: {e}")))?;
        let lane_id = ff_core::types::LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let derived_idem = format!("{}:{}:{}", execution_id, signal.signal_name, waitpoint_id);
        let effective_idem = signal
            .idempotency_key
            .clone()
            .unwrap_or_else(|| derived_idem.clone());
        let idem_key = ctx.signal_dedup(waitpoint_id, &effective_idem);

        let payload_str = signal
            .payload
            .as_ref()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .unwrap_or_default();

        let (keys, args) = crate::fcall::suspension::build_deliver_signal(
            &ctx,
            &idx,
            &lane_id,
            &signal_id,
            waitpoint_id,
            idem_key,
            execution_id,
            signal.signal_name,
            signal.signal_category,
            signal.source_type,
            signal.source_identity,
            payload_str,
            effective_idem,
            now,
            self.runtime.config.signal_dedup_ttl_ms,
            crate::constants::DEFAULT_SIGNAL_MAXLEN,
            crate::constants::DEFAULT_MAX_SIGNALS_PER_EXECUTION,
            signal.waitpoint_token.as_str(),
        );

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_DELIVER_SIGNAL, &key_refs, &arg_refs)
            .await?;

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
    use super::*;

    #[test]
    fn signal_carries_waitpoint_token_field() {
        // Pin the contract: after FF v2 #4, every Signal MUST carry a
        // waitpoint_token. If ff-sdk removes or renames the field, this
        // test fails to compile — which is the point.
        let token = WaitpointToken::new("kid_1:deadbeef");
        let signal = Signal {
            signal_name: "approval_granted:foo".into(),
            signal_category: "approval".into(),
            payload: None,
            source_type: crate::constants::SOURCE_TYPE_APPROVAL_OPERATOR.into(),
            source_identity: "cairn".into(),
            idempotency_key: Some("approval:foo".into()),
            waitpoint_token: token.clone(),
        };
        assert_eq!(signal.waitpoint_token.as_str(), "kid_1:deadbeef");
        // Debug MUST redact; it's ok to rely on the ff-core guarantee, but
        // pin it here because a regression would leak HMAC material into logs.
        let dbg = format!("{:?}", signal.waitpoint_token);
        assert!(dbg.contains("REDACTED"), "token Debug must redact: {dbg}");
        assert!(!dbg.contains("deadbeef"), "token hex must not leak: {dbg}");
    }

    #[test]
    fn signal_debug_redacts_token_transitively() {
        // Derive(Debug) on Signal delegates to WaitpointToken::Debug, which
        // redacts. Pin this so a future ff-sdk custom Debug impl that used
        // token.as_str() would leak HMAC material into every tracing!(?signal).
        let token = WaitpointToken::new("kid_3:deadbeefcafe");
        let signal = Signal {
            signal_name: "tool_result:x".into(),
            signal_category: "tool".into(),
            payload: None,
            source_type: "cairn_runtime".into(),
            source_identity: "cairn".into(),
            idempotency_key: None,
            waitpoint_token: token,
        };
        let dbg = format!("{signal:?}");
        assert!(
            !dbg.contains("deadbeef"),
            "Signal Debug leaked token material: {dbg}"
        );
        assert!(
            dbg.contains("REDACTED"),
            "Signal Debug should surface redaction marker: {dbg}"
        );
    }

    #[test]
    fn waitpoint_token_display_redacts() {
        // Defensive: if we ever log a token accidentally via Display (e.g. in
        // an error message), the redaction must hold.
        let token = WaitpointToken::new("kid_2:cafebabe0011");
        let disp = format!("{token}");
        assert!(disp.contains("REDACTED"), "Display must redact: {disp}");
        assert!(
            !disp.contains("cafebabe"),
            "token hex must not leak: {disp}"
        );
    }

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
