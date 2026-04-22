use cairn_domain::{ProjectKey, RuntimeEvent};
use cairn_store::event_log::StoredEvent;

use crate::projections::{
    EdgeKind, GraphEdge, GraphNode, GraphProjection, GraphProjectionError, NodeKind,
};

/// Projects runtime events into graph nodes and edges.
///
/// This is an asynchronous derived projection (RFC 002) — it runs
/// after event persistence, not within the same transaction.
/// Graph projections are rebuildable from the event log.
pub struct EventProjector<P: GraphProjection> {
    projection: P,
}

impl<P: GraphProjection> EventProjector<P> {
    pub fn new(projection: P) -> Self {
        Self { projection }
    }

    /// Project a batch of stored events into graph structure.
    pub async fn project_events(
        &self,
        events: &[StoredEvent],
    ) -> Result<ProjectionResult, GraphProjectionError> {
        let mut nodes_created = 0u64;
        let mut edges_created = 0u64;

        for event in events {
            let (n, e) = self.project_single(event).await?;
            nodes_created += n;
            edges_created += e;
        }

        Ok(ProjectionResult {
            nodes_created,
            edges_created,
        })
    }

    async fn project_single(
        &self,
        event: &StoredEvent,
    ) -> Result<(u64, u64), GraphProjectionError> {
        let ts = event.stored_at;
        let mut nodes = 0u64;
        let mut edges = 0u64;

        match &event.envelope.payload {
            RuntimeEvent::SessionCreated(e) => {
                self.add_node(
                    e.session_id.as_str(),
                    NodeKind::Session,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 1;
            }

            RuntimeEvent::RunCreated(e) => {
                self.add_node(e.run_id.as_str(), NodeKind::Run, Some(&e.project), ts)
                    .await?;
                nodes += 1;

                // Run -> Session
                self.add_edge(
                    e.run_id.as_str(),
                    e.session_id.as_str(),
                    EdgeKind::Triggered,
                    ts,
                )
                .await?;
                edges += 1;

                // Run -> parent run
                if let Some(parent) = &e.parent_run_id {
                    self.add_edge(parent.as_str(), e.run_id.as_str(), EdgeKind::Spawned, ts)
                        .await?;
                    edges += 1;
                }
            }

            RuntimeEvent::TaskCreated(e) => {
                self.add_node(e.task_id.as_str(), NodeKind::Task, Some(&e.project), ts)
                    .await?;
                nodes += 1;

                if let Some(run_id) = &e.parent_run_id {
                    self.add_edge(run_id.as_str(), e.task_id.as_str(), EdgeKind::Spawned, ts)
                        .await?;
                    edges += 1;
                }

                if let Some(task_id) = &e.parent_task_id {
                    self.add_edge(
                        task_id.as_str(),
                        e.task_id.as_str(),
                        EdgeKind::DependedOn,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
            }

            RuntimeEvent::ApprovalRequested(e) => {
                self.add_node(
                    e.approval_id.as_str(),
                    NodeKind::Approval,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 1;

                if let Some(run_id) = &e.run_id {
                    // run -> approval (Triggered)
                    self.add_edge(
                        run_id.as_str(),
                        e.approval_id.as_str(),
                        EdgeKind::Triggered,
                        ts,
                    )
                    .await?;
                    edges += 1;
                    // approval -> run (ApprovedBy) for reverse provenance traversal
                    self.add_edge(
                        e.approval_id.as_str(),
                        run_id.as_str(),
                        EdgeKind::ApprovedBy,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
                if let Some(task_id) = &e.task_id {
                    // task -> approval (Triggered)
                    self.add_edge(
                        task_id.as_str(),
                        e.approval_id.as_str(),
                        EdgeKind::Triggered,
                        ts,
                    )
                    .await?;
                    edges += 1;
                    // approval -> task (ApprovedBy) for reverse provenance traversal
                    self.add_edge(
                        e.approval_id.as_str(),
                        task_id.as_str(),
                        EdgeKind::ApprovedBy,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
            }

            RuntimeEvent::ApprovalResolved(_) => {
                // Decision recorded; node and edges already exist from ApprovalRequested.
            }

            RuntimeEvent::CheckpointRecorded(e) => {
                self.add_node(
                    e.checkpoint_id.as_str(),
                    NodeKind::Checkpoint,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 1;

                self.add_edge(
                    e.run_id.as_str(),
                    e.checkpoint_id.as_str(),
                    EdgeKind::Triggered,
                    ts,
                )
                .await?;
                edges += 1;
            }

            RuntimeEvent::TriggerCreated(e) => {
                self.add_node(
                    &trigger_node_id(e.trigger_id.as_str()),
                    NodeKind::Trigger,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 1;
            }

            RuntimeEvent::TriggerFired(e) => {
                let trigger_node_id = trigger_node_id(e.trigger_id.as_str());

                self.add_edge(
                    e.signal_id.as_str(),
                    &trigger_node_id,
                    EdgeKind::MatchedBy,
                    ts,
                )
                .await?;
                edges += 1;

                self.add_edge(&trigger_node_id, e.run_id.as_str(), EdgeKind::Fired, ts)
                    .await?;
                edges += 1;
            }

            RuntimeEvent::CheckpointRestored(e) => {
                self.add_edge(
                    e.checkpoint_id.as_str(),
                    e.run_id.as_str(),
                    EdgeKind::ResumedFrom,
                    ts,
                )
                .await?;
                edges += 1;
            }

            RuntimeEvent::MailboxMessageAppended(e) => {
                self.add_node(
                    e.message_id.as_str(),
                    NodeKind::MailboxMessage,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 1;

                if let Some(run_id) = &e.run_id {
                    self.add_edge(run_id.as_str(), e.message_id.as_str(), EdgeKind::SentTo, ts)
                        .await?;
                    edges += 1;
                }
                if let Some(task_id) = &e.task_id {
                    self.add_edge(
                        task_id.as_str(),
                        e.message_id.as_str(),
                        EdgeKind::SentTo,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
            }

            RuntimeEvent::ToolInvocationStarted(e) => {
                self.add_node(
                    e.invocation_id.as_str(),
                    NodeKind::ToolInvocation,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 1;

                if let Some(run_id) = &e.run_id {
                    self.add_edge(
                        run_id.as_str(),
                        e.invocation_id.as_str(),
                        EdgeKind::UsedTool,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
                if let Some(task_id) = &e.task_id {
                    self.add_edge(
                        task_id.as_str(),
                        e.invocation_id.as_str(),
                        EdgeKind::UsedTool,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
            }

            RuntimeEvent::SubagentSpawned(e) => {
                // Create child session and task nodes if they don't exist.
                self.add_node(
                    e.child_session_id.as_str(),
                    NodeKind::Session,
                    Some(&e.project),
                    ts,
                )
                .await?;
                self.add_node(
                    e.child_task_id.as_str(),
                    NodeKind::Task,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 2;

                // Parent run spawned child task.
                self.add_edge(
                    e.parent_run_id.as_str(),
                    e.child_task_id.as_str(),
                    EdgeKind::Spawned,
                    ts,
                )
                .await?;
                edges += 1;

                // Child task linked to child session.
                self.add_edge(
                    e.child_task_id.as_str(),
                    e.child_session_id.as_str(),
                    EdgeKind::Triggered,
                    ts,
                )
                .await?;
                edges += 1;

                if let Some(child_run) = &e.child_run_id {
                    self.add_node(child_run.as_str(), NodeKind::Run, Some(&e.project), ts)
                        .await?;
                    nodes += 1;
                    self.add_edge(
                        e.child_session_id.as_str(),
                        child_run.as_str(),
                        EdgeKind::Triggered,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
            }

            RuntimeEvent::SignalIngested(e) => {
                self.add_node(e.signal_id.as_str(), NodeKind::Signal, Some(&e.project), ts)
                    .await?;
                nodes += 1;
            }

            RuntimeEvent::IngestJobStarted(e) => {
                self.add_node(e.job_id.as_str(), NodeKind::IngestJob, Some(&e.project), ts)
                    .await?;
                nodes += 1;
            }

            // State-change and audit events don't create new nodes/edges.
            RuntimeEvent::SessionStateChanged(_)
            | RuntimeEvent::RunStateChanged(_)
            | RuntimeEvent::TaskLeaseClaimed(_)
            | RuntimeEvent::TaskLeaseHeartbeated(_)
            | RuntimeEvent::TaskStateChanged(_)
            | RuntimeEvent::ToolInvocationCompleted(_)
            | RuntimeEvent::ToolInvocationFailed(_)
            | RuntimeEvent::ExternalWorkerRegistered(_)
            | RuntimeEvent::ExternalWorkerReported(_)
            | RuntimeEvent::ExternalWorkerSuspended(_)
            | RuntimeEvent::ExternalWorkerReactivated(_)
            | RuntimeEvent::SoulPatchProposed(_)
            | RuntimeEvent::SoulPatchApplied(_)
            | RuntimeEvent::SessionCostUpdated(_)
            | RuntimeEvent::SpendAlertTriggered(_)
            | RuntimeEvent::RunCostUpdated(_)
            | RuntimeEvent::RecoveryAttempted(_)
            | RuntimeEvent::RecoveryCompleted(_)
            | RuntimeEvent::UserMessageAppended(_)
            | RuntimeEvent::IngestJobCompleted(_)
            | RuntimeEvent::PromptAssetCreated(_)
            | RuntimeEvent::PromptVersionCreated(_)
            | RuntimeEvent::ApprovalPolicyCreated(_)
            | RuntimeEvent::PromptReleaseCreated(_)
            | RuntimeEvent::PromptReleaseTransitioned(_)
            | RuntimeEvent::PromptRolloutStarted(_)
            | RuntimeEvent::TenantCreated(_)
            | RuntimeEvent::WorkspaceCreated(_)
            | RuntimeEvent::ProjectCreated(_)
            | RuntimeEvent::RouteDecisionMade(_)
            | RuntimeEvent::ProviderCallCompleted(_)
            | RuntimeEvent::OutcomeRecorded(_)
            | RuntimeEvent::ScheduledTaskCreated(_)
            | RuntimeEvent::PlanProposed(_)
            | RuntimeEvent::PlanApproved(_)
            | RuntimeEvent::PlanRejected(_)
            | RuntimeEvent::PlanRevisionRequested(_)
            | RuntimeEvent::ProviderBudgetSet(_)
            | RuntimeEvent::ChannelCreated(_)
            | RuntimeEvent::ChannelMessageSent(_)
            | RuntimeEvent::ChannelMessageConsumed(_)
            | RuntimeEvent::DefaultSettingSet(_)
            | RuntimeEvent::DefaultSettingCleared(_)
            | RuntimeEvent::LicenseActivated(_)
            | RuntimeEvent::EntitlementOverrideSet(_)
            | RuntimeEvent::NotificationPreferenceSet(_)
            | RuntimeEvent::NotificationSent(_)
            | RuntimeEvent::ProviderPoolCreated(_)
            | RuntimeEvent::ProviderPoolConnectionAdded(_)
            | RuntimeEvent::ProviderPoolConnectionRemoved(_)
            | RuntimeEvent::TenantQuotaSet(_)
            | RuntimeEvent::TenantQuotaViolated(_)
            | RuntimeEvent::RetentionPolicySet(_)
            | RuntimeEvent::RunCostAlertSet(_)
            | RuntimeEvent::RunCostAlertTriggered(_)
            | RuntimeEvent::WorkspaceMemberAdded(_)
            | RuntimeEvent::WorkspaceMemberRemoved(_)
            | RuntimeEvent::TriggerEnabled(_)
            | RuntimeEvent::TriggerDisabled(_)
            | RuntimeEvent::TriggerSuspended(_)
            | RuntimeEvent::TriggerResumed(_)
            | RuntimeEvent::TriggerDeleted(_)
            | RuntimeEvent::TriggerSkipped(_)
            | RuntimeEvent::TriggerDenied(_)
            | RuntimeEvent::TriggerRateLimited(_)
            | RuntimeEvent::TriggerPendingApproval(_)
            | RuntimeEvent::RunTemplateCreated(_)
            | RuntimeEvent::RunTemplateDeleted(_)
            | RuntimeEvent::ApprovalDelegated(_)
            | RuntimeEvent::AuditLogEntryRecorded(_)
            | RuntimeEvent::CheckpointStrategySet(_)
            | RuntimeEvent::CredentialKeyRotated(_)
            | RuntimeEvent::CredentialRevoked(_)
            | RuntimeEvent::CredentialStored(_)
            | RuntimeEvent::EvalBaselineLocked(_)
            | RuntimeEvent::EvalBaselineSet(_)
            | RuntimeEvent::EvalDatasetCreated(_)
            | RuntimeEvent::EvalDatasetEntryAdded(_)
            | RuntimeEvent::EvalRubricCreated(_)
            | RuntimeEvent::EventLogCompacted(_)
            | RuntimeEvent::GuardrailPolicyCreated(_)
            | RuntimeEvent::GuardrailPolicyEvaluated(_)
            | RuntimeEvent::OperatorIntervention(_)
            | RuntimeEvent::OperatorProfileCreated(_)
            | RuntimeEvent::OperatorProfileUpdated(_)
            | RuntimeEvent::PauseScheduled(_)
            | RuntimeEvent::PermissionDecisionRecorded(_)
            | RuntimeEvent::ProviderBindingCreated(_)
            | RuntimeEvent::ProviderBindingStateChanged(_)
            | RuntimeEvent::ProviderBudgetAlertTriggered(_)
            | RuntimeEvent::ProviderBudgetExceeded(_)
            | RuntimeEvent::ProviderConnectionRegistered(_)
            | RuntimeEvent::ProviderHealthChecked(_)
            | RuntimeEvent::ProviderHealthScheduleSet(_)
            | RuntimeEvent::ProviderHealthScheduleTriggered(_)
            | RuntimeEvent::ProviderMarkedDegraded(_)
            | RuntimeEvent::ProviderModelRegistered(_)
            | RuntimeEvent::ProviderRecovered(_)
            | RuntimeEvent::ProviderRetryPolicySet(_)
            | RuntimeEvent::RecoveryEscalated(_)
            | RuntimeEvent::ResourceShareRevoked(_)
            | RuntimeEvent::ResourceShared(_)
            | RuntimeEvent::RoutePolicyCreated(_)
            | RuntimeEvent::RoutePolicyUpdated(_)
            | RuntimeEvent::RunSlaBreached(_)
            | RuntimeEvent::RunSlaSet(_)
            | RuntimeEvent::SignalRouted(_)
            | RuntimeEvent::SignalSubscriptionCreated(_)
            | RuntimeEvent::SnapshotCreated(_)
            | RuntimeEvent::TaskDependencyAdded(_)
            | RuntimeEvent::TaskDependencyResolved(_)
            | RuntimeEvent::TaskLeaseExpired(_)
            | RuntimeEvent::TaskPriorityChanged(_)
            | RuntimeEvent::ToolInvocationProgressUpdated(_)
            // RFC 020 Track 3: audit-only events; no graph projection.
            | RuntimeEvent::ToolInvocationCacheHit(_)
            | RuntimeEvent::ToolRecoveryPaused(_)
            // RFC 020 decision-cache survival: durable for startup replay,
            // does not feed the graph projection.
            | RuntimeEvent::DecisionRecorded(_)
            | RuntimeEvent::DecisionCacheWarmup(_) => {}

            RuntimeEvent::EvalRunStarted(e) => {
                self.add_node(
                    e.eval_run_id.as_str(),
                    NodeKind::EvalRun,
                    Some(&e.project),
                    ts,
                )
                .await?;
                nodes += 1;
            }

            RuntimeEvent::EvalRunCompleted(e) => {
                // EvaluatedBy edge: eval run -> subject being evaluated.
                if let Some(ref subject_id) = e.subject_node_id {
                    self.add_edge(
                        e.eval_run_id.as_str(),
                        subject_id,
                        EdgeKind::EvaluatedBy,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
            }
        }

        Ok((nodes, edges))
    }

    async fn add_node(
        &self,
        id: &str,
        kind: NodeKind,
        project: Option<&ProjectKey>,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: id.to_owned(),
                kind,
                project: project.cloned(),
                created_at: ts,
            })
            .await
    }

    async fn add_edge(
        &self,
        source: &str,
        target: &str,
        kind: EdgeKind,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_edge(GraphEdge {
                source_node_id: source.to_owned(),
                target_node_id: target.to_owned(),
                kind,
                created_at: ts,
                confidence: None,
            })
            .await
    }
}

fn trigger_node_id(trigger_id: &str) -> String {
    format!("trigger:{trigger_id}")
}

/// Result from projecting a batch of events.
#[derive(Clone, Debug)]
pub struct ProjectionResult {
    pub nodes_created: u64,
    pub edges_created: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cairn_domain::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MemGraph {
        nodes: Mutex<HashMap<String, GraphNode>>,
        edges: Mutex<Vec<GraphEdge>>,
    }

    impl MemGraph {
        fn new() -> Self {
            Self {
                nodes: Mutex::new(HashMap::new()),
                edges: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl GraphProjection for Arc<MemGraph> {
        async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError> {
            self.nodes
                .lock()
                .unwrap()
                .insert(node.node_id.clone(), node);
            Ok(())
        }

        async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError> {
            self.edges.lock().unwrap().push(edge);
            Ok(())
        }

        async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError> {
            Ok(self.nodes.lock().unwrap().contains_key(node_id))
        }
    }

    fn make_stored(payload: RuntimeEvent) -> StoredEvent {
        StoredEvent {
            position: cairn_store::EventPosition(1),
            envelope: EventEnvelope::for_runtime_event(
                EventId::new("evt_1"),
                EventSource::Runtime,
                payload,
            ),
            stored_at: 1000,
        }
    }

    #[tokio::test]
    async fn projects_session_and_run_with_edges() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        let events = vec![
            make_stored(RuntimeEvent::SessionCreated(SessionCreated {
                project: ProjectKey::new("t", "w", "p"),
                session_id: SessionId::new("sess_1"),
            })),
            make_stored(RuntimeEvent::RunCreated(RunCreated {
                project: ProjectKey::new("t", "w", "p"),
                session_id: SessionId::new("sess_1"),
                run_id: RunId::new("run_1"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
        ];

        let result = projector.project_events(&events).await.unwrap();
        assert_eq!(result.nodes_created, 2); // session + run
        assert_eq!(result.edges_created, 1); // run -> session

        let nodes = graph.nodes.lock().unwrap();
        assert!(nodes.contains_key("sess_1"));
        assert!(nodes.contains_key("run_1"));

        let edges = graph.edges.lock().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_node_id, "run_1");
        assert_eq!(edges[0].target_node_id, "sess_1");
    }

    #[tokio::test]
    async fn projects_subagent_spawn() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        let events = vec![make_stored(RuntimeEvent::SubagentSpawned(
            SubagentSpawned {
                project: ProjectKey::new("t", "w", "p"),
                parent_run_id: RunId::new("run_1"),
                parent_task_id: None,
                child_task_id: TaskId::new("child_task"),
                child_session_id: SessionId::new("child_sess"),
                child_run_id: Some(RunId::new("child_run")),
            },
        ))];

        let result = projector.project_events(&events).await.unwrap();
        assert_eq!(result.nodes_created, 3); // child session + task + run
        assert_eq!(result.edges_created, 3); // spawned + triggered + triggered
    }

    #[tokio::test]
    async fn state_changes_produce_no_graph_mutations() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        let events = vec![make_stored(RuntimeEvent::RunStateChanged(
            RunStateChanged {
                project: ProjectKey::new("t", "w", "p"),
                run_id: RunId::new("run_1"),
                transition: StateTransition {
                    from: Some(RunState::Pending),
                    to: RunState::Running,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            },
        ))];

        let result = projector.project_events(&events).await.unwrap();
        assert_eq!(result.nodes_created, 0);
        assert_eq!(result.edges_created, 0);
    }

    #[tokio::test]
    async fn projects_eval_run_started_and_completed() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        let events = vec![
            make_stored(RuntimeEvent::EvalRunStarted(EvalRunStarted {
                project: ProjectKey::new("t", "w", "p"),
                eval_run_id: EvalRunId::new("eval_1"),
                subject_kind: "prompt_release".to_owned(),
                evaluator_type: "automated".to_owned(),
                started_at: 100,
                prompt_asset_id: None,
                prompt_version_id: None,
                prompt_release_id: None,
                created_by: None,
            })),
            make_stored(RuntimeEvent::EvalRunCompleted(EvalRunCompleted {
                project: ProjectKey::new("t", "w", "p"),
                eval_run_id: EvalRunId::new("eval_1"),
                success: true,
                error_message: None,
                subject_node_id: Some("release_1".to_owned()),
                completed_at: 200,
            })),
        ];

        let result = projector.project_events(&events).await.unwrap();
        assert_eq!(result.nodes_created, 1); // EvalRun node
        assert_eq!(result.edges_created, 1); // EvaluatedBy edge

        let nodes = graph.nodes.lock().unwrap();
        assert!(nodes.contains_key("eval_1"));
        assert_eq!(nodes["eval_1"].kind, NodeKind::EvalRun);

        let edges = graph.edges.lock().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_node_id, "eval_1");
        assert_eq!(edges[0].target_node_id, "release_1");
        assert_eq!(edges[0].kind, EdgeKind::EvaluatedBy);
    }

    #[tokio::test]
    async fn eval_completed_without_subject_creates_no_edge() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        let events = vec![
            make_stored(RuntimeEvent::EvalRunStarted(EvalRunStarted {
                project: ProjectKey::new("t", "w", "p"),
                eval_run_id: EvalRunId::new("eval_2"),
                subject_kind: "prompt_release".to_owned(),
                evaluator_type: "automated".to_owned(),
                started_at: 100,
                prompt_asset_id: None,
                prompt_version_id: None,
                prompt_release_id: None,
                created_by: None,
            })),
            make_stored(RuntimeEvent::EvalRunCompleted(EvalRunCompleted {
                project: ProjectKey::new("t", "w", "p"),
                eval_run_id: EvalRunId::new("eval_2"),
                success: false,
                error_message: Some("timeout".to_owned()),
                subject_node_id: None,
                completed_at: 200,
            })),
        ];

        let result = projector.project_events(&events).await.unwrap();
        assert_eq!(result.nodes_created, 1); // EvalRun node only
        assert_eq!(result.edges_created, 0); // No edge without subject
    }

    #[tokio::test]
    async fn projects_trigger_provenance_edges() {
        let graph = Arc::new(MemGraph::new());
        let projector = EventProjector::new(graph.clone());

        let events = vec![
            make_stored(RuntimeEvent::SignalIngested(SignalIngested {
                project: ProjectKey::new("t", "w", "p"),
                signal_id: SignalId::new("sig_1"),
                source: "github.issue.labeled".to_owned(),
                payload: serde_json::json!({"action": "labeled"}),
                timestamp_ms: 100,
            })),
            make_stored(RuntimeEvent::RunCreated(RunCreated {
                project: ProjectKey::new("t", "w", "p"),
                session_id: SessionId::new("sess_1"),
                run_id: RunId::new("run_1"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            make_stored(RuntimeEvent::TriggerCreated(TriggerCreated {
                project: ProjectKey::new("t", "w", "p"),
                trigger_id: TriggerId::new("trigger_1"),
                name: "GitHub issue labeled".to_owned(),
                description: None,
                signal_type: "github.issue.labeled".to_owned(),
                plugin_id: Some("github".to_owned()),
                conditions: vec![],
                run_template_id: RunTemplateId::new("tmpl_1"),
                max_per_minute: 10,
                max_burst: 20,
                max_chain_depth: 5,
                created_by: OperatorId::new("operator"),
                created_at: 100,
            })),
            make_stored(RuntimeEvent::TriggerFired(TriggerFired {
                project: ProjectKey::new("t", "w", "p"),
                trigger_id: TriggerId::new("trigger_1"),
                signal_id: SignalId::new("sig_1"),
                signal_type: "github.issue.labeled".to_owned(),
                run_id: RunId::new("run_1"),
                chain_depth: 1,
                fired_at: 110,
            })),
        ];

        let result = projector.project_events(&events).await.unwrap();
        assert_eq!(result.nodes_created, 3);
        assert_eq!(result.edges_created, 3);

        let nodes = graph.nodes.lock().unwrap();
        assert!(nodes.contains_key("sig_1"));
        assert!(nodes.contains_key("run_1"));
        assert!(nodes.contains_key("trigger:trigger_1"));
        assert_eq!(nodes["trigger:trigger_1"].kind, NodeKind::Trigger);

        let edges = graph.edges.lock().unwrap();
        assert!(edges.iter().any(|edge| {
            edge.source_node_id == "sig_1"
                && edge.target_node_id == "trigger:trigger_1"
                && edge.kind == EdgeKind::MatchedBy
        }));
        assert!(edges.iter().any(|edge| {
            edge.source_node_id == "trigger:trigger_1"
                && edge.target_node_id == "run_1"
                && edge.kind == EdgeKind::Fired
        }));
    }
}
