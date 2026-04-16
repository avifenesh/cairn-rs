use std::sync::Arc;

use cairn_domain::ids::{RunId, SessionId};
use cairn_domain::lifecycle::{FailureClass, RunState};
use cairn_domain::tenancy::ProjectKey;
use ff_sdk::task::{ClaimedTask, FailOutcome, SuspendOutcome};
use ff_sdk::{FlowFabricWorker, WorkerConfig};

use crate::active_tasks::{ActiveTaskHandle, ActiveTaskRegistry};
use crate::config::FabricConfig;
use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers;
use crate::stream::StreamWriter;
use crate::suspension;

pub struct CairnWorker {
    inner: FlowFabricWorker,
    bridge: Arc<EventBridge>,
    registry: Arc<ActiveTaskRegistry>,
}

impl CairnWorker {
    pub async fn connect(
        config: &FabricConfig,
        bridge: Arc<EventBridge>,
        registry: Arc<ActiveTaskRegistry>,
    ) -> Result<Self, FabricError> {
        let worker_config = WorkerConfig {
            host: config.valkey_host.clone(),
            port: config.valkey_port,
            tls: config.tls,
            cluster: config.cluster,
            worker_id: config.worker_id.clone(),
            worker_instance_id: config.worker_instance_id.clone(),
            namespace: config.namespace.clone(),
            lanes: vec![config.lane_id.clone()],
            capabilities: Vec::new(),
            lease_ttl_ms: config.lease_ttl_ms,
            claim_poll_interval_ms: 1_000,
            max_concurrent_tasks: config.max_concurrent_tasks,
        };

        let inner = FlowFabricWorker::connect(worker_config)
            .await
            .map_err(|e| FabricError::Bridge(format!("worker connect: {e}")))?;

        Ok(Self {
            inner,
            bridge,
            registry,
        })
    }

    pub async fn claim_next(&self) -> Result<Option<CairnTask>, FabricError> {
        let claimed = self
            .inner
            .claim_next()
            .await
            .map_err(|e| FabricError::Bridge(format!("claim_next: {e}")))?;

        let task = match claimed {
            Some(t) => t,
            None => return Ok(None),
        };

        let execution_id = task.execution_id().clone();
        let lease_id = task.lease_id().clone();
        let lease_epoch = task.lease_epoch();
        let attempt_index = task.attempt_index();

        // Store lightweight handle for lease context queries by other services.
        // CairnTask owns the ClaimedTask directly — registry.take() returns None
        // for CairnWorker-claimed tasks, which is expected.
        let context_handle = ActiveTaskHandle::new_without_claimed_task(
            execution_id,
            lease_id,
            lease_epoch,
            attempt_index,
        );

        let run_id = extract_tag(task.tags(), "cairn.run_id");
        let session_id = extract_tag(task.tags(), "cairn.session_id");
        let project_str = extract_tag(task.tags(), "cairn.project");

        // Register in ActiveTaskRegistry using cairn.task_id if present,
        // falling back to cairn.run_id. cairn-fabric task submissions always
        // set cairn.task_id; run submissions use cairn.run_id as the key.
        let registry_key = extract_tag(task.tags(), "cairn.task_id").or_else(|| run_id.clone());
        if let Some(ref key) = registry_key {
            let task_id = cairn_domain::ids::TaskId::new(key);
            self.registry.register(&task_id, context_handle);
        }

        Ok(Some(CairnTask {
            task,
            bridge: self.bridge.clone(),
            run_id: run_id.map(RunId::new),
            session_id: session_id.map(SessionId::new),
            project: project_str.map(|s| helpers::parse_project_key(&s)),
        }))
    }

    pub fn inner(&self) -> &FlowFabricWorker {
        &self.inner
    }

    pub fn registry(&self) -> &Arc<ActiveTaskRegistry> {
        &self.registry
    }
}

pub struct CairnTask {
    task: ClaimedTask,
    bridge: Arc<EventBridge>,
    run_id: Option<RunId>,
    session_id: Option<SessionId>,
    project: Option<ProjectKey>,
}

impl CairnTask {
    pub fn run_id(&self) -> Option<&RunId> {
        self.run_id.as_ref()
    }

    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    pub fn project(&self) -> Option<&ProjectKey> {
        self.project.as_ref()
    }

    pub fn input_payload(&self) -> &[u8] {
        self.task.input_payload()
    }

    pub fn stream_writer(&self) -> StreamWriter<'_> {
        StreamWriter::new(&self.task)
    }

    pub async fn log_tool_call(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), FabricError> {
        self.stream_writer().log_tool_call(name, args).await?;
        Ok(())
    }

    pub async fn log_tool_result(
        &self,
        tool_name: &str,
        output: &serde_json::Value,
        success: bool,
        duration_ms: u64,
    ) -> Result<(), FabricError> {
        self.stream_writer()
            .log_tool_result(tool_name, output, success, duration_ms)
            .await?;
        Ok(())
    }

    pub async fn log_llm_response(
        &self,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
        latency_ms: u64,
    ) -> Result<(), FabricError> {
        self.stream_writer()
            .log_llm_response(model, tokens_in, tokens_out, latency_ms)
            .await?;
        Ok(())
    }

    pub async fn save_checkpoint(&self, context_json: &[u8]) -> Result<(), FabricError> {
        self.stream_writer().save_checkpoint(context_json).await?;
        Ok(())
    }

    pub async fn log_progress(&self, pct: u8, message: &str) -> Result<(), FabricError> {
        self.task
            .update_progress(pct, message)
            .await
            .map_err(|e| FabricError::Bridge(format!("update_progress: {e}")))
    }

    pub async fn complete_with_result(self, result: Option<Vec<u8>>) -> Result<(), FabricError> {
        let run_id = self.run_id.clone();
        let project = self.project.clone();
        let bridge = self.bridge.clone();

        self.task
            .complete(result)
            .await
            .map_err(|e| FabricError::Bridge(format!("complete: {e}")))?;

        if let (Some(rid), Some(proj)) = (run_id, project) {
            bridge.emit(BridgeEvent::ExecutionCompleted {
                run_id: rid,
                project: proj,
                prev_state: Some(RunState::Running),
            });
        }

        Ok(())
    }

    /// Fail the execution. Returns `RetryScheduled` if FF's retry policy
    /// allows another attempt (execution re-enters the delayed queue and will
    /// be offered via `claim_next` when backoff expires), or `TerminalFailed`
    /// if retries are exhausted. Consumes self either way — don't hold state.
    pub async fn fail_with_retry(
        self,
        reason: &str,
        category: &str,
    ) -> Result<FailOutcome, FabricError> {
        let run_id = self.run_id.clone();
        let project = self.project.clone();
        let bridge = self.bridge.clone();

        let outcome = self
            .task
            .fail(reason, category)
            .await
            .map_err(|e| FabricError::Bridge(format!("fail: {e}")))?;

        if matches!(outcome, FailOutcome::TerminalFailed) {
            if let (Some(rid), Some(proj)) = (run_id, project) {
                bridge.emit(BridgeEvent::ExecutionFailed {
                    run_id: rid,
                    project: proj,
                    failure_class: FailureClass::ExecutionError,
                    prev_state: Some(RunState::Running),
                });
            }
        }

        Ok(outcome)
    }

    pub async fn cancel(self, reason: &str) -> Result<(), FabricError> {
        let run_id = self.run_id.clone();
        let project = self.project.clone();
        let bridge = self.bridge.clone();

        self.task
            .cancel(reason)
            .await
            .map_err(|e| FabricError::Bridge(format!("cancel: {e}")))?;

        if let (Some(rid), Some(proj)) = (run_id, project) {
            bridge.emit(BridgeEvent::ExecutionCancelled {
                run_id: rid,
                project: proj,
                prev_state: Some(RunState::Running),
            });
        }

        Ok(())
    }

    pub async fn suspend_for_approval(
        self,
        approval_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<SuspendOutcome, FabricError> {
        let run_id = self.run_id.clone();
        let project = self.project.clone();
        let bridge = self.bridge.clone();

        let params = suspension::for_approval(approval_id, timeout_ms);
        let outcome = self
            .task
            .suspend(
                &params.reason_code,
                &params.condition_matchers,
                params.timeout_ms,
                params.timeout_behavior,
            )
            .await
            .map_err(|e| FabricError::Bridge(format!("suspend_for_approval: {e}")))?;

        if matches!(outcome, SuspendOutcome::Suspended { .. }) {
            if let (Some(rid), Some(proj)) = (run_id, project) {
                bridge.emit(BridgeEvent::ExecutionSuspended {
                    run_id: rid,
                    project: proj,
                    prev_state: Some(RunState::Running),
                });
            }
        }

        Ok(outcome)
    }

    pub async fn suspend_for_subagent(
        self,
        child_task_id: &str,
        deadline_ms: Option<u64>,
    ) -> Result<SuspendOutcome, FabricError> {
        let run_id = self.run_id.clone();
        let project = self.project.clone();
        let bridge = self.bridge.clone();

        let params = suspension::for_subagent(child_task_id, deadline_ms);
        let outcome = self
            .task
            .suspend(
                &params.reason_code,
                &params.condition_matchers,
                params.timeout_ms,
                params.timeout_behavior,
            )
            .await
            .map_err(|e| FabricError::Bridge(format!("suspend_for_subagent: {e}")))?;

        if matches!(outcome, SuspendOutcome::Suspended { .. }) {
            if let (Some(rid), Some(proj)) = (run_id, project) {
                bridge.emit(BridgeEvent::ExecutionSuspended {
                    run_id: rid,
                    project: proj,
                    prev_state: Some(RunState::Running),
                });
            }
        }

        Ok(outcome)
    }
    pub async fn suspend_for_tool_result(
        self,
        invocation_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<SuspendOutcome, FabricError> {
        let run_id = self.run_id.clone();
        let project = self.project.clone();
        let bridge = self.bridge.clone();

        let params = suspension::for_tool_result(invocation_id, timeout_ms);
        let outcome = self
            .task
            .suspend(
                &params.reason_code,
                &params.condition_matchers,
                params.timeout_ms,
                params.timeout_behavior,
            )
            .await
            .map_err(|e| FabricError::Bridge(format!("suspend_for_tool_result: {e}")))?;

        if matches!(outcome, SuspendOutcome::Suspended { .. }) {
            if let (Some(rid), Some(proj)) = (run_id, project) {
                bridge.emit(BridgeEvent::ExecutionSuspended {
                    run_id: rid,
                    project: proj,
                    prev_state: Some(RunState::Running),
                });
            }
        }

        Ok(outcome)
    }
}

fn extract_tag(tags: &std::collections::HashMap<String, String>, key: &str) -> Option<String> {
    tags.get(key).filter(|v| !v.is_empty()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn extract_tag_present() {
        let mut tags = HashMap::new();
        tags.insert("cairn.run_id".into(), "run_123".into());
        assert_eq!(extract_tag(&tags, "cairn.run_id"), Some("run_123".into()));
    }

    #[test]
    fn extract_tag_missing() {
        let tags = HashMap::new();
        assert_eq!(extract_tag(&tags, "cairn.run_id"), None);
    }

    #[test]
    fn extract_tag_empty_value() {
        let mut tags = HashMap::new();
        tags.insert("cairn.run_id".into(), String::new());
        assert_eq!(extract_tag(&tags, "cairn.run_id"), None);
    }

    #[test]
    fn parse_project_key_delegates_to_helpers() {
        let pk = helpers::parse_project_key("t1/w1/p1");
        assert_eq!(pk.tenant_id.as_str(), "t1");
        assert_eq!(pk.workspace_id.as_str(), "w1");
        assert_eq!(pk.project_id.as_str(), "p1");
    }

    #[test]
    fn extract_multiple_tags() {
        let mut tags = HashMap::new();
        tags.insert("cairn.run_id".into(), "run_1".into());
        tags.insert("cairn.session_id".into(), "sess_1".into());
        tags.insert("cairn.project".into(), "t/w/p".into());

        assert_eq!(extract_tag(&tags, "cairn.run_id"), Some("run_1".into()));
        assert_eq!(
            extract_tag(&tags, "cairn.session_id"),
            Some("sess_1".into())
        );
        assert_eq!(extract_tag(&tags, "cairn.project"), Some("t/w/p".into()));
    }

    #[test]
    fn extract_tag_ignores_other_keys() {
        let mut tags = HashMap::new();
        tags.insert("other.key".into(), "value".into());
        assert_eq!(extract_tag(&tags, "cairn.run_id"), None);
    }
}
