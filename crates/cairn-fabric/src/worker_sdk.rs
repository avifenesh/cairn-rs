//! Thin worker wrapper over `ff_sdk::FlowFabricWorker`.
//!
//! Cairn claims run through the in-process scheduler: callers ask
//! [`FabricSchedulerService::claim_for_worker`] for a `ClaimGrant`
//! (budget + quota + capability admission all run server-side inside
//! cairn), then hand that grant to
//! [`CairnWorker::claim_from_grant`] to materialize a
//! [`CairnTask`]. The external-process HTTP claim path
//! (`claim_via_server` in ff-sdk) is not used — cairn is a single
//! binary with the scheduler embedded.
//!
//! Terminal ops on the returned `CairnTask` read every lease field
//! from FF's exec_core on demand; no cairn-side registry is cached.
//!
//! [`FabricSchedulerService::claim_for_worker`]: crate::services::scheduler_service::FabricSchedulerService::claim_for_worker

use std::sync::Arc;

use cairn_domain::ids::{RunId, SessionId};
use cairn_domain::lifecycle::{FailureClass, RunState};
use cairn_domain::tenancy::ProjectKey;
use ff_core::contracts::ClaimGrant;
use ff_sdk::task::{ClaimedTask, FailOutcome, ResumeSignal, SuspendOutcome};
use ff_sdk::{FlowFabricWorker, WorkerConfig};

use crate::config::FabricConfig;
use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers;
use crate::stream::StreamWriter;
use crate::suspension;

pub struct CairnWorker {
    inner: FlowFabricWorker,
    bridge: Arc<EventBridge>,
}

impl CairnWorker {
    pub async fn connect(
        config: &FabricConfig,
        bridge: Arc<EventBridge>,
    ) -> Result<Self, FabricError> {
        // FF's WorkerConfig carries capabilities as a Vec<String>; our
        // FabricConfig uses a BTreeSet for deterministic ordering.
        // Collecting preserves that order on the wire.
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

        Ok(Self { inner, bridge })
    }

    /// Consume a `ClaimGrant` (from
    /// [`FabricSchedulerService::claim_for_worker`]) and materialize a
    /// [`CairnTask`] with the cairn tag fields (run_id, session_id,
    /// project) pre-extracted.
    ///
    /// [`FabricSchedulerService::claim_for_worker`]: crate::services::scheduler_service::FabricSchedulerService::claim_for_worker
    pub async fn claim_from_grant(
        &self,
        lane: ff_core::types::LaneId,
        grant: ClaimGrant,
    ) -> Result<CairnTask, FabricError> {
        let task = self
            .inner
            .claim_from_grant(lane, grant)
            .await
            .map_err(|e| FabricError::Bridge(format!("claim_from_grant: {e}")))?;

        let run_id = extract_tag(task.tags(), "cairn.run_id");
        let session_id = extract_tag(task.tags(), "cairn.session_id");
        let project_str = extract_tag(task.tags(), "cairn.project");

        Ok(CairnTask {
            task: Some(task),
            bridge: self.bridge.clone(),
            run_id: run_id.map(RunId::new),
            session_id: session_id.map(SessionId::new),
            project: project_str.and_then(|s| helpers::try_parse_project_key(&s)),
        })
    }

    pub fn inner(&self) -> &FlowFabricWorker {
        &self.inner
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
    /// Panicking accessor — used only on paths where the Option<ClaimedTask>
    /// is guaranteed Some by caller context (the public observers like
    /// `input_payload` and `stream_writer`, where the caller holds a live
    /// `&CairnTask` reference that by construction cannot be consumed).
    fn task(&self) -> &ClaimedTask {
        self.task.as_ref().expect("CairnTask already consumed")
    }

    /// Fallible accessor — returns `FabricError::Bridge("task already
    /// consumed")` for a consumed CairnTask instead of panicking. The
    /// async log_* methods and `save_checkpoint` route through this so
    /// a stray `&CairnTask` reused after a terminal call surfaces a
    /// typed error (swallowed by the orchestrator's frame-sink
    /// log-and-continue contract) rather than bringing down the loop.
    ///
    /// The blanket `impl TaskFrameSink for CairnTask` routes every
    /// `log_tool_call` / `log_tool_result` / `log_llm_response` /
    /// `save_checkpoint` through this helper instead of `task()`, so a
    /// consumed task yields `Err` rather than panicking inside the
    /// frame-sink trait object.
    fn try_task(&self) -> Result<&ClaimedTask, FabricError> {
        self.task
            .as_ref()
            .ok_or_else(|| FabricError::Bridge("task already consumed".to_owned()))
    }

    /// Fallible variant of `stream_writer()`. Returns the same
    /// `task already consumed` error as `try_task` when the
    /// Option<ClaimedTask> has been taken. Used by the async log_*
    /// methods so they can propagate the typed error up through the
    /// TaskFrameSink trait instead of panicking.
    fn try_stream_writer(&self) -> Result<StreamWriter<'_>, FabricError> {
        Ok(StreamWriter::new(self.try_task()?))
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

    /// Signals that satisfied the waitpoint and triggered the current
    /// resume. Returns an empty list for fresh claims (task was not
    /// resumed from suspension). Useful for workers that branch on
    /// *which* of several possible signals woke them.
    pub async fn resume_signals(&self) -> Result<Vec<ResumeSignal>, FabricError> {
        self.try_task()?
            .resume_signals()
            .await
            .map_err(|e| FabricError::Bridge(format!("resume_signals: {e}")))
    }

    pub fn stream_writer(&self) -> StreamWriter<'_> {
        StreamWriter::new(self.task())
    }

    pub async fn log_tool_call(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), FabricError> {
        self.try_stream_writer()?.log_tool_call(name, args).await?;
        Ok(())
    }

    pub async fn log_tool_result(
        &self,
        tool_name: &str,
        output: &serde_json::Value,
        success: bool,
        duration_ms: u64,
    ) -> Result<(), FabricError> {
        self.try_stream_writer()?
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
        self.try_stream_writer()?
            .log_llm_response(model, tokens_in, tokens_out, latency_ms)
            .await?;
        Ok(())
    }

    pub async fn save_checkpoint(&self, context_json: &[u8]) -> Result<(), FabricError> {
        self.try_stream_writer()?
            .save_checkpoint(context_json)
            .await?;
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
        self.try_task()?
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
                            // Worker SDK only runs while the task is in
                            // Running; FF can also re-schedule retries from
                            // Suspended/Delayed, but those paths go through
                            // FF's own scheduler loop, not this SDK call.
                            prev_state: Some(RunState::Running),
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
                        to: RunState::WaitingApproval,
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
                        // `suspension::for_subagent` sets FF blocking_reason
                        // `waiting_for_children`; the corresponding cairn
                        // domain state is WaitingDependency.
                        to: RunState::WaitingDependency,
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
                        // `suspension::for_tool_result` uses FF blocking_reason
                        // `waiting_for_tool_result`. cairn domain has no
                        // dedicated state for that today; it collapses to
                        // Paused. See T4-M7 for the tracked follow-up.
                        to: RunState::Paused,
                    })
                    .await;
            }
        }

        Ok(outcome)
    }
}

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

    /// A consumed-task None-branch for the async `log_*` and
    /// `save_checkpoint` path must return `FabricError::Bridge("task
    /// already consumed")`, not panic — otherwise a stray call after a
    /// terminal op crashes the orchestrator's panic-catch boundary.
    #[tokio::test]
    async fn log_methods_on_consumed_task_return_bridge_error() {
        let (bridge, _handle) = EventBridge::start(Arc::new(cairn_store::InMemoryStore::default()));
        let task = CairnTask {
            task: None,
            bridge: Arc::new(bridge),
            run_id: None,
            session_id: None,
            project: None,
        };

        // Spot-check one of each shape. Every method routes through
        // try_stream_writer() / try_task() so a single bad hunk would
        // flip all five; one assertion per would be redundant. We check
        // the error TYPE + MESSAGE since that's what the orchestrator's
        // frame-sink log-and-continue contract would see.
        let err = task
            .log_tool_call("fs.read", &serde_json::json!({}))
            .await
            .expect_err("consumed task must Err, not panic");
        match err {
            FabricError::Bridge(msg) => assert!(
                msg.contains("task already consumed"),
                "expected 'task already consumed' in Bridge error, got: {msg}"
            ),
            other => panic!("expected FabricError::Bridge, got {other:?}"),
        }

        // save_checkpoint shares the same helper — sanity-check it too so
        // anyone refactoring try_stream_writer can't miss either path.
        let err = task
            .save_checkpoint(b"{}")
            .await
            .expect_err("save_checkpoint on consumed task must Err");
        assert!(matches!(err, FabricError::Bridge(_)));

        // log_progress uses try_task directly (no StreamWriter). Same
        // error contract.
        let err = task
            .log_progress(50, "progress")
            .await
            .expect_err("log_progress on consumed task must Err");
        assert!(matches!(err, FabricError::Bridge(_)));
    }
}
