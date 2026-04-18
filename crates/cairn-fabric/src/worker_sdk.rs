//! Thin worker wrapper over `ff_sdk::FlowFabricWorker`.
//!
//! # Claim path: dispute note (FF @a09871000574388256b1dd7c910239e992c0d3a6)
//!
//! Manager's round-1 brief proposed routing `CairnWorker::claim_next` through
//! `ff_scheduler::Scheduler::claim_for_worker` + a public
//! `FlowFabricWorker::claim_from_grant` entry point on ff-sdk. That entry
//! point does **not** exist in FF @a09871000574388256b1dd7c910239e992c0d3a6:
//!
//!   - `ff_sdk::worker::claim_next`        → `#[cfg(feature = "insecure-direct-claim")]`
//!   - `ff_sdk::worker::issue_claim_grant` → `#[cfg(feature = "insecure-direct-claim")]`
//!   - `ff_sdk::worker::claim_execution`   → `#[cfg(feature = "insecure-direct-claim")]`
//!   - `ff_sdk::worker::claim_resumed_execution` → same
//!   - `ff_sdk::task::ClaimedTask::new`    → `pub(crate)` — not constructable from cairn
//!
//! Without a `pub fn claim_from_grant` (or a `pub` `ClaimedTask::new`),
//! cairn cannot build a `ClaimedTask` from a scheduler-issued `ClaimGrant`.
//! The wrapper relies on `ClaimedTask` for `complete`, `fail`, `cancel`,
//! `suspend`, lease renewal, stream writes, progress — every operator
//! endpoint past claim. Re-implementing those atop the raw `Client` would
//! duplicate state FF owns (explicit violation of the THIN BRIDGE design
//! principle in the round brief).
//!
//! Chosen compromise until FF publishes a scheduler-mediated public path:
//!   1. Production claim path: `FabricSchedulerService::claim_for_worker`
//!      is always available (returns a `ff_scheduler::ClaimGrant`). Callers
//!      that only need a grant (and drive their own claim FCALL) use it.
//!      The Valkey-direct claim already used by `task_service::claim` is
//!      that same pattern.
//!   2. Legacy direct path: `CairnWorker::claim_next` is now gated behind
//!      the cairn feature `insecure-direct-claim`, which forwards to
//!      `ff-sdk/insecure-direct-claim`. Off by default. Available for
//!      integration tests and local dev that want a ClaimedTask with all
//!      the stream/progress/lease machinery pre-wired.
//!
//! Follow-up: file a FF issue asking for `pub fn FlowFabricWorker::claim_from_grant`
//! (or a pub constructor on `ClaimedTask`) so cairn can route the production
//! path through the scheduler without forcing the `insecure-direct-claim`
//! feature on every worker binary.

use std::sync::Arc;

use cairn_domain::ids::{RunId, SessionId};
use cairn_domain::lifecycle::{FailureClass, RunState};
use cairn_domain::tenancy::ProjectKey;
use ff_sdk::task::{ClaimedTask, FailOutcome, SuspendOutcome};
use ff_sdk::{FlowFabricWorker, WorkerConfig};

#[cfg(feature = "insecure-direct-claim")]
use crate::active_tasks::ActiveTaskHandle;
use crate::active_tasks::ActiveTaskRegistry;
use crate::config::FabricConfig;
use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
#[cfg(feature = "insecure-direct-claim")]
use crate::helpers;
use crate::stream::StreamWriter;
use crate::suspension;

pub struct CairnWorker {
    inner: FlowFabricWorker,
    // Only read inside `claim_next` (feature-gated); kept on the struct so
    // the constructor signature stays stable across feature flips.
    #[cfg_attr(not(feature = "insecure-direct-claim"), allow(dead_code))]
    bridge: Arc<EventBridge>,
    registry: Arc<ActiveTaskRegistry>,
}

impl CairnWorker {
    pub async fn connect(
        config: &FabricConfig,
        bridge: Arc<EventBridge>,
        registry: Arc<ActiveTaskRegistry>,
    ) -> Result<Self, FabricError> {
        // WorkerConfig.capabilities is a `Vec<String>` on ff-sdk; sort + dedup
        // mirrors the BTreeSet semantics carried by FabricConfig so both the
        // scheduler path and the direct-claim path advertise an identical,
        // deterministic set. FF re-validates on ingress.
        let capabilities: Vec<String> = config.worker_capabilities.iter().cloned().collect();

        let worker_config = WorkerConfig {
            host: config.valkey_host.clone(),
            port: config.valkey_port,
            tls: config.tls,
            cluster: config.cluster,
            worker_id: config.worker_id.clone(),
            worker_instance_id: config.worker_instance_id.clone(),
            namespace: config.namespace.clone(),
            lanes: vec![config.lane_id.clone()],
            capabilities,
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

    /// Direct (scheduler-bypassing) claim. Gated behind the cairn feature
    /// `insecure-direct-claim` — default builds don't expose it.
    ///
    /// Production callers should use `FabricSchedulerService::claim_for_worker`
    /// to obtain a `ClaimGrant` (which goes through budget + quota admission),
    /// then consume that grant via the direct FCALL pattern in
    /// `task_service::claim`. See the module-level dispute note for why a
    /// scheduler-mediated `CairnTask` wrapper is not available today.
    #[cfg(feature = "insecure-direct-claim")]
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
            task: Some(task),
            bridge: self.bridge.clone(),
            run_id: run_id.map(RunId::new),
            session_id: session_id.map(SessionId::new),
            project: project_str.and_then(|s| helpers::try_parse_project_key(&s)),
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
    task: Option<ClaimedTask>,
    bridge: Arc<EventBridge>,
    run_id: Option<RunId>,
    session_id: Option<SessionId>,
    project: Option<ProjectKey>,
}

impl Drop for CairnTask {
    fn drop(&mut self) {
        if self.task.is_some() {
            let run_id = self
                .run_id
                .as_ref()
                .map(|r| r.as_str())
                .unwrap_or("unknown");
            tracing::warn!(
                run_id,
                "CairnTask dropped without terminal call — lease will expire via FF scanner"
            );
        }
    }
}

impl CairnTask {
    fn task(&self) -> &ClaimedTask {
        self.task.as_ref().expect("CairnTask already consumed")
    }

    fn take_task(
        mut self,
    ) -> (
        ClaimedTask,
        Arc<EventBridge>,
        Option<RunId>,
        Option<ProjectKey>,
    ) {
        let task = self.task.take().expect("CairnTask already consumed");
        let bridge = self.bridge.clone();
        let run_id = self.run_id.clone();
        let project = self.project.clone();
        (task, bridge, run_id, project)
    }

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
        self.task().input_payload()
    }

    pub fn stream_writer(&self) -> StreamWriter<'_> {
        StreamWriter::new(self.task())
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

    /// Check before expensive side effects. `false` means the lease has
    /// failed 3 consecutive renewals (ff-sdk threshold) and the orchestrator
    /// should abort cleanly instead of committing irreversible work.
    ///
    /// Thin delegation — FF owns the renewal counter. Returns `false` for a
    /// consumed task (terminal methods take `self`, so callers holding a
    /// live reference cannot see a `None` here; the guard exists only to
    /// keep the signature total for the feature-gated claim path).
    pub fn is_lease_healthy(&self) -> bool {
        self.task
            .as_ref()
            .map(|t| t.is_lease_healthy())
            .unwrap_or(false)
    }

    pub async fn log_progress(&self, pct: u8, message: &str) -> Result<(), FabricError> {
        let clamped = pct.min(100);
        self.task()
            .update_progress(clamped, message)
            .await
            .map_err(|e| FabricError::Bridge(format!("update_progress: {e}")))
    }

    pub async fn complete_with_result(self, result: Option<Vec<u8>>) -> Result<(), FabricError> {
        let (task, bridge, run_id, project) = self.take_task();

        task.complete(result)
            .await
            .map_err(|e| FabricError::Bridge(format!("complete: {e}")))?;

        if let (Some(rid), Some(proj)) = (run_id, project) {
            bridge
                .emit(BridgeEvent::ExecutionCompleted {
                    run_id: rid,
                    project: proj,
                    prev_state: Some(RunState::Running),
                })
                .await;
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
        let attempt = self.task().attempt_index().0;
        let (task, bridge, run_id, project) = self.take_task();

        let outcome = task
            .fail(reason, category)
            .await
            .map_err(|e| FabricError::Bridge(format!("fail: {e}")))?;

        match outcome {
            FailOutcome::TerminalFailed => {
                if let (Some(rid), Some(proj)) = (run_id, project) {
                    bridge
                        .emit(BridgeEvent::ExecutionFailed {
                            run_id: rid,
                            project: proj,
                            failure_class: FailureClass::ExecutionError,
                            prev_state: Some(RunState::Running),
                        })
                        .await;
                }
            }
            FailOutcome::RetryScheduled { .. } => {
                if let (Some(rid), Some(proj)) = (run_id, project) {
                    bridge
                        .emit(BridgeEvent::ExecutionRetryScheduled {
                            run_id: rid,
                            project: proj,
                            attempt,
                        })
                        .await;
                }
            }
        }

        Ok(outcome)
    }

    pub async fn cancel(self, reason: &str) -> Result<(), FabricError> {
        let (task, bridge, run_id, project) = self.take_task();

        task.cancel(reason)
            .await
            .map_err(|e| FabricError::Bridge(format!("cancel: {e}")))?;

        if let (Some(rid), Some(proj)) = (run_id, project) {
            bridge
                .emit(BridgeEvent::ExecutionCancelled {
                    run_id: rid,
                    project: proj,
                    prev_state: Some(RunState::Running),
                })
                .await;
        }

        Ok(())
    }

    pub async fn suspend_for_approval(
        self,
        approval_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<SuspendOutcome, FabricError> {
        let params = suspension::for_approval(approval_id, timeout_ms);
        let (task, bridge, run_id, project) = self.take_task();

        let outcome = task
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
                bridge
                    .emit(BridgeEvent::ExecutionSuspended {
                        run_id: rid,
                        project: proj,
                        prev_state: Some(RunState::Running),
                    })
                    .await;
            }
        }

        Ok(outcome)
    }

    pub async fn suspend_for_subagent(
        self,
        child_task_id: &str,
        deadline_ms: Option<u64>,
    ) -> Result<SuspendOutcome, FabricError> {
        let params = suspension::for_subagent(child_task_id, deadline_ms);
        let (task, bridge, run_id, project) = self.take_task();

        let outcome = task
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
                bridge
                    .emit(BridgeEvent::ExecutionSuspended {
                        run_id: rid,
                        project: proj,
                        prev_state: Some(RunState::Running),
                    })
                    .await;
            }
        }

        Ok(outcome)
    }
    pub async fn suspend_for_tool_result(
        self,
        invocation_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<SuspendOutcome, FabricError> {
        let params = suspension::for_tool_result(invocation_id, timeout_ms);
        let (task, bridge, run_id, project) = self.take_task();

        let outcome = task
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
                bridge
                    .emit(BridgeEvent::ExecutionSuspended {
                        run_id: rid,
                        project: proj,
                        prev_state: Some(RunState::Running),
                    })
                    .await;
            }
        }

        Ok(outcome)
    }
}

// Used by `claim_next` (feature-gated) and by the unit tests below. When the
// feature is off in a non-test build, the function has no callers — mark it
// allow(dead_code) to keep cargo check quiet without hiding real dead code.
#[cfg_attr(not(feature = "insecure-direct-claim"), allow(dead_code))]
fn extract_tag(tags: &std::collections::HashMap<String, String>, key: &str) -> Option<String> {
    tags.get(key)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers;
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
    fn try_parse_project_key_delegates_to_helpers() {
        let pk = helpers::try_parse_project_key("t1/w1/p1").unwrap();
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

    #[tokio::test]
    async fn is_lease_healthy_returns_false_for_consumed_task() {
        // We can't build a real ClaimedTask here (ff-sdk keeps ClaimedTask::new
        // pub(crate); see the module-level dispute note). What we CAN prove is
        // the None-branch of the delegation: a CairnTask whose Option<task>
        // has already been taken must report unhealthy, not panic. That keeps
        // `is_lease_healthy()` total after a consuming call sequence and
        // guards against a regression that unwraps `self.task` instead of
        // going through `as_ref().map(...)`.
        //
        // The 3-consecutive-renewal-failure threshold itself lives inside FF's
        // ClaimedTask; exercising it needs a live Valkey + killed scanner and
        // belongs in an integration test, not a unit.
        let (bridge, _handle) = EventBridge::start(Arc::new(cairn_store::InMemoryStore::default()));
        let task = CairnTask {
            task: None,
            bridge: Arc::new(bridge),
            run_id: None,
            session_id: None,
            project: None,
        };
        assert!(
            !task.is_lease_healthy(),
            "consumed CairnTask must report unhealthy lease, not panic"
        );
    }
}
