use cairn_domain::RuntimeEvent;
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
                self.add_node(&e.session_id.as_str(), NodeKind::Session, ts)
                    .await?;
                nodes += 1;
            }

            RuntimeEvent::RunCreated(e) => {
                self.add_node(e.run_id.as_str(), NodeKind::Run, ts).await?;
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
                self.add_node(e.task_id.as_str(), NodeKind::Task, ts)
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
                self.add_node(e.approval_id.as_str(), NodeKind::Approval, ts)
                    .await?;
                nodes += 1;

                if let Some(run_id) = &e.run_id {
                    self.add_edge(
                        run_id.as_str(),
                        e.approval_id.as_str(),
                        EdgeKind::Triggered,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
                if let Some(task_id) = &e.task_id {
                    self.add_edge(
                        task_id.as_str(),
                        e.approval_id.as_str(),
                        EdgeKind::Triggered,
                        ts,
                    )
                    .await?;
                    edges += 1;
                }
            }

            RuntimeEvent::ApprovalResolved(e) => {
                // Edge from approval to itself marking resolution — the
                // node already exists from ApprovalRequested.
                // No new node needed.
                let _ = e;
            }

            RuntimeEvent::CheckpointRecorded(e) => {
                self.add_node(e.checkpoint_id.as_str(), NodeKind::Checkpoint, ts)
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
                self.add_node(e.message_id.as_str(), NodeKind::MailboxMessage, ts)
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
                self.add_node(e.invocation_id.as_str(), NodeKind::ToolInvocation, ts)
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
                self.add_node(e.child_session_id.as_str(), NodeKind::Session, ts)
                    .await?;
                self.add_node(e.child_task_id.as_str(), NodeKind::Task, ts)
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
                    self.add_node(child_run.as_str(), NodeKind::Run, ts).await?;
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

            // State-change and audit events don't create new nodes/edges.
            RuntimeEvent::SessionStateChanged(_)
            | RuntimeEvent::RunStateChanged(_)
            | RuntimeEvent::TaskLeaseClaimed(_)
            | RuntimeEvent::TaskLeaseHeartbeated(_)
            | RuntimeEvent::TaskStateChanged(_)
            | RuntimeEvent::ToolInvocationCompleted(_)
            | RuntimeEvent::ToolInvocationFailed(_)
            | RuntimeEvent::ExternalWorkerReported(_)
            | RuntimeEvent::RecoveryAttempted(_)
            | RuntimeEvent::RecoveryCompleted(_)
            | RuntimeEvent::SignalIngested(_) => {}
        }

        Ok((nodes, edges))
    }

    async fn add_node(
        &self,
        id: &str,
        kind: NodeKind,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: id.to_owned(),
                kind,
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
            })
            .await
    }
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
            },
        ))];

        let result = projector.project_events(&events).await.unwrap();
        assert_eq!(result.nodes_created, 0);
        assert_eq!(result.edges_created, 0);
    }
}
