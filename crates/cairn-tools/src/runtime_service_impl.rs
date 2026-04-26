//! Concrete `RuntimeToolService` implementation wired to Worker 4's
//! `ToolInvocationService` for event persistence and our pipeline for execution.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::tool_invocation::ToolInvocationTarget;
use cairn_runtime::services::tool_invocation_impl::ToolInvocationService;

use crate::builtin::{ToolHost, ToolOutcome};
use crate::invocation::{outcome_error_message, outcome_to_kind};
use crate::permissions::PermissionGate;
use crate::pipeline::{run_builtin_pipeline, PipelineOutcome};
use crate::runtime_service::{
    RuntimeToolOutcome, RuntimeToolRequest, RuntimeToolResponse, RuntimeToolService,
    ToolLifecycleOutput,
};

/// Concrete implementation that bridges our pipeline to the runtime's
/// event persistence layer.
pub struct RuntimeToolServiceImpl<G, H, T> {
    gate: Arc<G>,
    host: Arc<H>,
    tool_invocation_service: Arc<T>,
}

impl<G, H, T> RuntimeToolServiceImpl<G, H, T> {
    pub fn new(gate: Arc<G>, host: Arc<H>, tool_invocation_service: Arc<T>) -> Self {
        Self {
            gate,
            host,
            tool_invocation_service,
        }
    }
}

#[async_trait]
impl<G, H, T> RuntimeToolService for RuntimeToolServiceImpl<G, H, T>
where
    G: PermissionGate + Send + Sync + 'static,
    H: ToolHost + Send + Sync + 'static,
    T: ToolInvocationService + 'static,
{
    async fn invoke(
        &self,
        request: RuntimeToolRequest,
    ) -> Result<RuntimeToolResponse, Box<dyn std::error::Error + Send + Sync>> {
        let tool_name = match &request.target {
            ToolInvocationTarget::Builtin { tool_name } => tool_name.clone(),
            ToolInvocationTarget::Plugin { tool_name, .. } => tool_name.clone(),
        };

        // Run through our pipeline (permission check + execution)
        let pipeline_result = run_builtin_pipeline(
            self.gate.as_ref(),
            self.host.as_ref(),
            &request.project,
            request.invocation_id.clone(),
            request.session_id.clone(),
            request.run_id.clone(),
            request.task_id.clone(),
            &tool_name,
            request.params.clone(),
            request.execution_class,
            request.requested_at_ms,
        );

        // Persist through Worker 4's ToolInvocationService
        match &pipeline_result.outcome {
            PipelineOutcome::Completed(outcome) => {
                // Record start
                // F55: thread the raw tool params into the projection so
                // operators can see the arguments via
                // GET /v1/tool-invocations.
                self.tool_invocation_service
                    .record_start(
                        &request.project,
                        request.invocation_id.clone(),
                        request.session_id.clone(),
                        request.run_id.clone(),
                        request.task_id.clone(),
                        request.target.clone(),
                        request.execution_class,
                        Some(request.params.clone()),
                    )
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

                // Record completion or failure
                match outcome {
                    ToolOutcome::Success { .. } => {
                        self.tool_invocation_service
                            .record_completed(
                                &request.project,
                                request.invocation_id.clone(),
                                request.task_id.clone(),
                                tool_name.clone(),
                                &[],
                                None,
                                None,
                            )
                            .await
                            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                Box::new(e)
                            })?;
                    }
                    other => {
                        self.tool_invocation_service
                            .record_failed(
                                &request.project,
                                request.invocation_id.clone(),
                                request.task_id.clone(),
                                tool_name.clone(),
                                outcome_to_kind(other),
                                outcome_error_message(other),
                                None,
                            )
                            .await
                            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                                Box::new(e)
                            })?;
                    }
                }
            }
            PipelineOutcome::PermissionDenied { .. } | PipelineOutcome::HeldForApproval { .. } => {
                // No runtime events for denied/held — runtime handles pause/reject
            }
        }

        // Build response with lifecycle output
        let (runtime_outcome, lifecycle) = match &pipeline_result.outcome {
            PipelineOutcome::Completed(ToolOutcome::Success { output }) => (
                RuntimeToolOutcome::Success,
                ToolLifecycleOutput::completed(&tool_name, Some(output.clone())),
            ),
            PipelineOutcome::Completed(ToolOutcome::RetryableFailure { reason }) => (
                RuntimeToolOutcome::Failed {
                    retryable: true,
                    reason: reason.clone(),
                },
                ToolLifecycleOutput::failed(&tool_name, reason),
            ),
            PipelineOutcome::Completed(ToolOutcome::PermanentFailure { reason }) => (
                RuntimeToolOutcome::Failed {
                    retryable: false,
                    reason: reason.clone(),
                },
                ToolLifecycleOutput::failed(&tool_name, reason),
            ),
            PipelineOutcome::Completed(ToolOutcome::Timeout) => (
                RuntimeToolOutcome::Timeout,
                ToolLifecycleOutput::failed(&tool_name, "timeout"),
            ),
            PipelineOutcome::Completed(ToolOutcome::Canceled) => (
                RuntimeToolOutcome::Canceled,
                ToolLifecycleOutput::failed(&tool_name, "canceled"),
            ),
            PipelineOutcome::PermissionDenied { reason } => (
                RuntimeToolOutcome::PermissionDenied {
                    reason: reason.clone(),
                },
                ToolLifecycleOutput::failed(&tool_name, reason),
            ),
            PipelineOutcome::HeldForApproval { reason } => (
                RuntimeToolOutcome::HeldForApproval {
                    reason: reason.clone(),
                },
                ToolLifecycleOutput::started(&tool_name, Some(request.params)),
            ),
        };

        Ok(RuntimeToolResponse {
            records: pipeline_result.records,
            outcome: runtime_outcome,
            lifecycle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use cairn_domain::ids::ToolInvocationId;
    use cairn_domain::policy::ExecutionClass;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};

    struct MockGate;
    impl crate::permissions::PermissionGate for MockGate {
        fn check(
            &self,
            project: &ProjectKey,
            _required: &crate::permissions::DeclaredPermissions,
            execution_class: ExecutionClass,
        ) -> crate::permissions::PermissionCheckResult {
            crate::permissions::PermissionCheckResult::Granted(
                crate::permissions::InvocationGrants {
                    project: project.clone(),
                    execution_class,
                    granted: vec![],
                },
            )
        }
    }

    struct MockHost;
    impl crate::builtin::ToolHost for MockHost {
        fn list_tools(&self) -> Vec<crate::builtin::ToolDescriptor> {
            vec![crate::builtin::ToolDescriptor {
                name: "test.echo".to_owned(),
                description: "Echo".to_owned(),
                required_permissions: crate::permissions::DeclaredPermissions::default(),
            }]
        }
        fn invoke(&self, input: crate::builtin::ToolInput) -> crate::builtin::ToolOutcome {
            crate::builtin::ToolOutcome::Success {
                output: input.params,
            }
        }
    }

    #[test]
    fn service_impl_is_constructable_with_concrete_types() {
        // MockInvocationService stands in for the real ToolInvocationService
        struct MockInvocationService;

        #[async_trait]
        impl ToolInvocationService for MockInvocationService {
            async fn record_start(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::SessionId>,
                _: Option<cairn_domain::RunId>,
                _: Option<cairn_domain::TaskId>,
                _: ToolInvocationTarget,
                _: ExecutionClass,
                _: Option<serde_json::Value>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }

            async fn record_completed(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::TaskId>,
                _: String,
                _: &[cairn_domain::RuntimeEvent],
                _: Option<String>,
                _: Option<serde_json::Value>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }

            async fn record_failed(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::TaskId>,
                _: String,
                _: ToolInvocationOutcomeKind,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }

            async fn append_audit_events(
                &self,
                _: &[cairn_domain::RuntimeEvent],
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }
        }

        let _service = RuntimeToolServiceImpl::new(
            Arc::new(MockGate),
            Arc::new(MockHost),
            Arc::new(MockInvocationService),
        );
    }

    fn mock_invocation_svc() -> Arc<impl ToolInvocationService> {
        struct Svc;
        #[async_trait]
        impl ToolInvocationService for Svc {
            async fn record_start(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::SessionId>,
                _: Option<cairn_domain::RunId>,
                _: Option<cairn_domain::TaskId>,
                _: ToolInvocationTarget,
                _: ExecutionClass,
                _: Option<serde_json::Value>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }
            async fn record_completed(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::TaskId>,
                _: String,
                _: &[cairn_domain::RuntimeEvent],
                _: Option<String>,
                _: Option<serde_json::Value>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }
            async fn record_failed(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::TaskId>,
                _: String,
                _: ToolInvocationOutcomeKind,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }

            async fn append_audit_events(
                &self,
                _: &[cairn_domain::RuntimeEvent],
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }
        }
        Arc::new(Svc)
    }

    #[tokio::test]
    async fn end_to_end_builtin_invocation_coherence() {
        let service = RuntimeToolServiceImpl::new(
            Arc::new(MockGate),
            Arc::new(MockHost),
            mock_invocation_svc(),
        );

        let request = RuntimeToolRequest {
            plugin_id: None,
            invocation_id: ToolInvocationId::new("inv_e2e"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: None,
            run_id: None,
            task_id: None,
            target: ToolInvocationTarget::Builtin {
                tool_name: "test.echo".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            params: serde_json::json!({"msg": "hello"}),
            requested_at_ms: 1000,
        };

        let response = service.invoke(request).await.unwrap();

        // Outcome
        assert!(response.outcome.is_success());
        assert!(!response.outcome.should_pause_task());

        // Records trace full lifecycle
        assert_eq!(response.records.len(), 3);
        use cairn_domain::tool_invocation::ToolInvocationState;
        assert_eq!(response.records[0].state, ToolInvocationState::Requested);
        assert_eq!(response.records[1].state, ToolInvocationState::Started);
        assert_eq!(response.records[2].state, ToolInvocationState::Completed);

        // Lifecycle output is SSE-ready
        assert_eq!(response.lifecycle.tool_name, "test.echo");
        assert_eq!(response.lifecycle.phase, "completed");
        assert!(response.lifecycle.result.is_some());
        let json = serde_json::to_value(&response.lifecycle).unwrap();
        assert_eq!(json["toolName"], "test.echo");

        // Graph-linkable data from terminal record
        let node = crate::graph_events::to_node_data(&response.records[2]);
        assert_eq!(node.tool_name, "test.echo");
        assert_eq!(
            node.outcome,
            Some(cairn_domain::tool_invocation::ToolInvocationOutcomeKind::Success)
        );

        let edge = crate::graph_events::to_edge_data(&response.records[2]);
        assert_eq!(edge.invocation_id.as_str(), "inv_e2e");
    }

    #[tokio::test]
    async fn denied_invocation_produces_single_record() {
        struct DenyGate;
        impl crate::permissions::PermissionGate for DenyGate {
            fn check(
                &self,
                _: &ProjectKey,
                _: &crate::permissions::DeclaredPermissions,
                _: ExecutionClass,
            ) -> crate::permissions::PermissionCheckResult {
                crate::permissions::PermissionCheckResult::Denied(
                    cairn_domain::policy::PolicyVerdict::deny("blocked"),
                )
            }
        }

        let service = RuntimeToolServiceImpl::new(
            Arc::new(DenyGate),
            Arc::new(MockHost),
            mock_invocation_svc(),
        );

        let request = RuntimeToolRequest {
            plugin_id: None,
            invocation_id: ToolInvocationId::new("inv_denied"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: None,
            run_id: None,
            task_id: None,
            target: ToolInvocationTarget::Builtin {
                tool_name: "test.echo".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            params: serde_json::json!({}),
            requested_at_ms: 2000,
        };

        let response = service.invoke(request).await.unwrap();
        assert!(response.outcome.is_terminal_failure());
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.lifecycle.phase, "failed");

        // Policy denial reason flows through the lifecycle seam
        assert_eq!(response.lifecycle.error_detail, Some("blocked".to_owned()));
        assert_eq!(response.lifecycle.tool_name, "test.echo");

        // Lifecycle serializes with errorDetail for Worker 8 SSE shaping
        let json = serde_json::to_value(&response.lifecycle).unwrap();
        assert_eq!(json["errorDetail"], "blocked");
        assert!(json.get("result").is_none());
        assert!(json.get("args").is_none());
    }

    #[tokio::test]
    async fn lifecycle_output_matches_worker8_sse_contract() {
        // Verifies ToolLifecycleOutput serialization matches exactly what
        // Worker 8's build_enriched_tool_call_frame expects:
        // { toolName, phase, args?, result?, errorDetail? } in camelCase
        let service = RuntimeToolServiceImpl::new(
            Arc::new(MockGate),
            Arc::new(MockHost),
            mock_invocation_svc(),
        );

        let request = RuntimeToolRequest {
            plugin_id: None,
            invocation_id: ToolInvocationId::new("inv_sse"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: None,
            run_id: None,
            task_id: None,
            target: ToolInvocationTarget::Builtin {
                tool_name: "test.echo".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            params: serde_json::json!({"key": "value"}),
            requested_at_ms: 5000,
        };

        let response = service.invoke(request).await.unwrap();
        let json = serde_json::to_value(&response.lifecycle).unwrap();

        // Worker 8 contract: camelCase field names
        assert!(json.get("toolName").is_some(), "must have toolName");
        assert!(json.get("phase").is_some(), "must have phase");
        assert_eq!(json["toolName"], "test.echo");
        assert_eq!(json["phase"], "completed");

        // result present for success
        assert!(json.get("result").is_some(), "success must have result");

        // args and errorDetail absent for success (skip_serializing_if)
        assert!(json.get("args").is_none(), "success should not have args");
        assert!(
            json.get("errorDetail").is_none(),
            "success should not have errorDetail"
        );

        // Verify the shape is directly consumable by Worker 8:
        // { toolName: string, phase: string, result?: value }
        let obj = json.as_object().unwrap();
        assert!(
            obj.keys()
                .all(|k| { ["toolName", "phase", "result"].contains(&k.as_str()) }),
            "no unexpected fields: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn held_invocation_lifecycle_is_coherent() {
        struct HoldGate;
        impl crate::permissions::PermissionGate for HoldGate {
            fn check(
                &self,
                _: &ProjectKey,
                _: &crate::permissions::DeclaredPermissions,
                _: ExecutionClass,
            ) -> crate::permissions::PermissionCheckResult {
                crate::permissions::PermissionCheckResult::HeldForApproval(
                    cairn_domain::policy::PolicyVerdict::hold("needs operator review"),
                )
            }
        }

        let service = RuntimeToolServiceImpl::new(
            Arc::new(HoldGate),
            Arc::new(MockHost),
            mock_invocation_svc(),
        );

        let request = RuntimeToolRequest {
            plugin_id: None,
            invocation_id: ToolInvocationId::new("inv_held"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: None,
            run_id: None,
            task_id: None,
            target: ToolInvocationTarget::Builtin {
                tool_name: "test.echo".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            params: serde_json::json!({"important": true}),
            requested_at_ms: 6000,
        };

        let response = service.invoke(request).await.unwrap();

        // Held means runtime should pause the task
        assert!(response.outcome.should_pause_task());
        assert!(!response.outcome.is_success());
        assert!(!response.outcome.is_terminal_failure());

        // Only Requested record — no execution happened
        assert_eq!(response.records.len(), 1);

        // Lifecycle shows "start" phase with the original args preserved
        assert_eq!(response.lifecycle.phase, "start");
        assert_eq!(response.lifecycle.tool_name, "test.echo");
        assert!(response.lifecycle.args.is_some());
        assert_eq!(response.lifecycle.args.as_ref().unwrap()["important"], true);
    }

    #[tokio::test]
    async fn runtime_request_linkage_flows_into_pipeline_and_persistence() {
        type StartArgsList = Vec<(Option<String>, Option<String>, Option<String>)>;

        struct RecordingInvocationService {
            start_args: Mutex<StartArgsList>,
        }

        #[async_trait]
        impl ToolInvocationService for RecordingInvocationService {
            async fn record_start(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                session_id: Option<cairn_domain::SessionId>,
                run_id: Option<cairn_domain::RunId>,
                task_id: Option<cairn_domain::TaskId>,
                _: ToolInvocationTarget,
                _: ExecutionClass,
                _: Option<serde_json::Value>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                self.start_args.lock().unwrap().push((
                    session_id.map(|id| id.to_string()),
                    run_id.map(|id| id.to_string()),
                    task_id.map(|id| id.to_string()),
                ));
                Ok(())
            }

            async fn record_completed(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::TaskId>,
                _: String,
                _: &[cairn_domain::RuntimeEvent],
                _: Option<String>,
                _: Option<serde_json::Value>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }

            async fn record_failed(
                &self,
                _: &ProjectKey,
                _: ToolInvocationId,
                _: Option<cairn_domain::TaskId>,
                _: String,
                _: ToolInvocationOutcomeKind,
                _: Option<String>,
                _: Option<String>,
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }

            async fn append_audit_events(
                &self,
                _: &[cairn_domain::RuntimeEvent],
            ) -> Result<(), cairn_runtime::error::RuntimeError> {
                Ok(())
            }
        }

        let recorder = Arc::new(RecordingInvocationService {
            start_args: Mutex::new(Vec::new()),
        });
        let service =
            RuntimeToolServiceImpl::new(Arc::new(MockGate), Arc::new(MockHost), recorder.clone());

        let request = RuntimeToolRequest {
            plugin_id: None,
            invocation_id: ToolInvocationId::new("inv_linked"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: Some(cairn_domain::SessionId::new("sess_linked")),
            run_id: Some(cairn_domain::RunId::new("run_linked")),
            task_id: Some(cairn_domain::TaskId::new("task_linked")),
            target: ToolInvocationTarget::Builtin {
                tool_name: "test.echo".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            params: serde_json::json!({"msg": "linked"}),
            requested_at_ms: 7000,
        };

        let response = service.invoke(request).await.unwrap();

        assert_eq!(
            response.records[0]
                .session_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("sess_linked")
        );
        assert_eq!(
            response.records[0].run_id.as_ref().map(|id| id.as_str()),
            Some("run_linked")
        );
        assert_eq!(
            response.records[0].task_id.as_ref().map(|id| id.as_str()),
            Some("task_linked")
        );

        let start_args = recorder.start_args.lock().unwrap();
        assert_eq!(start_args.len(), 1);
        assert_eq!(
            start_args[0],
            (
                Some("sess_linked".to_owned()),
                Some("run_linked".to_owned()),
                Some("task_linked".to_owned())
            )
        );
    }

    #[test]
    fn all_tool_outcomes_produce_coherent_lifecycle_shapes() {
        // Verifies every ToolOutcome variant maps to a ToolLifecycleOutput
        // with the right phase and field presence — protecting tool outcome
        // coherence without widening plugin scope.
        use crate::builtin::ToolOutcome;
        use crate::runtime_service::ToolLifecycleOutput;

        let cases: Vec<(ToolOutcome, &str, bool, bool)> = vec![
            // (outcome, expected_phase, has_result, has_error_detail)
            (
                ToolOutcome::Success {
                    output: serde_json::json!({"ok": true}),
                },
                "completed",
                true,
                false,
            ),
            (
                ToolOutcome::RetryableFailure {
                    reason: "transient".to_owned(),
                },
                "failed",
                false,
                true,
            ),
            (
                ToolOutcome::PermanentFailure {
                    reason: "bad input".to_owned(),
                },
                "failed",
                false,
                true,
            ),
            (ToolOutcome::Timeout, "failed", false, true),
            (ToolOutcome::Canceled, "failed", false, true),
        ];

        for (outcome, expected_phase, has_result, has_error) in cases {
            let lifecycle = match &outcome {
                ToolOutcome::Success { output } => {
                    ToolLifecycleOutput::completed("test.tool", Some(output.clone()))
                }
                ToolOutcome::RetryableFailure { reason } => {
                    ToolLifecycleOutput::failed("test.tool", reason)
                }
                ToolOutcome::PermanentFailure { reason } => {
                    ToolLifecycleOutput::failed("test.tool", reason)
                }
                ToolOutcome::Timeout => ToolLifecycleOutput::failed("test.tool", "timeout"),
                ToolOutcome::Canceled => ToolLifecycleOutput::failed("test.tool", "canceled"),
            };

            assert_eq!(lifecycle.phase, expected_phase, "phase for {outcome:?}");
            assert_eq!(lifecycle.tool_name, "test.tool");
            assert_eq!(
                lifecycle.result.is_some(),
                has_result,
                "result for {outcome:?}"
            );
            assert_eq!(
                lifecycle.error_detail.is_some(),
                has_error,
                "error_detail for {outcome:?}"
            );

            // Every lifecycle must serialize to valid camelCase JSON
            let json = serde_json::to_value(&lifecycle).unwrap();
            assert!(json.get("toolName").is_some());
            assert!(json.get("phase").is_some());
        }
    }
}
