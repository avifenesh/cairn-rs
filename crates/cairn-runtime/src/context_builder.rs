//! ContextBuilder service — assembles the DECIDE-phase prompt context.
//!
//! Reads from five sources and produces a ContextBundle:
//! EventLog, RetrievalService, GraphQueryService, RuntimeConfig, OutcomeReadModel.

use crate::runtime_config::RuntimeConfig;
use crate::services::orchestrator::{ContextBundle, TaskSummary};
use async_trait::async_trait;
use cairn_domain::{ProjectKey, RunId};
use cairn_store::error::StoreError;
use cairn_store::projections::OutcomeReadModel;
use cairn_store::EventLog;
use std::sync::Arc;

/// Input parameters for a single context-assembly call.
#[derive(Clone, Debug)]
pub struct ContextBuilderInput {
    pub run_id: RunId,
    pub project: ProjectKey,
    pub goal: String,
    pub recent_event_limit: usize,
    pub memory_snippet_limit: usize,
    pub graph_neighbor_limit: usize,
    pub outcome_history_limit: usize,
    pub iteration: u32,
}

impl ContextBuilderInput {
    pub fn new(run_id: RunId, project: ProjectKey, goal: impl Into<String>) -> Self {
        Self {
            run_id,
            project,
            goal: goal.into(),
            recent_event_limit: 50,
            memory_snippet_limit: 5,
            graph_neighbor_limit: 10,
            outcome_history_limit: 20,
            iteration: 0,
        }
    }
}

/// Errors from DefaultContextBuilder.
#[derive(Debug)]
pub enum ContextBuildError {
    Store(StoreError),
    Memory(String),
    Graph(String),
}

impl std::fmt::Display for ContextBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextBuildError::Store(e) => write!(f, "store error: {e}"),
            ContextBuildError::Memory(e) => write!(f, "memory error: {e}"),
            ContextBuildError::Graph(e) => write!(f, "graph error: {e}"),
        }
    }
}

impl std::error::Error for ContextBuildError {}

impl From<StoreError> for ContextBuildError {
    fn from(e: StoreError) -> Self {
        ContextBuildError::Store(e)
    }
}

/// Assembles a ContextBundle for the orchestrator's DECIDE phase.
#[async_trait]
pub trait ContextBuilder: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;
    async fn build(&self, input: &ContextBuilderInput) -> Result<ContextBundle, Self::Error>;
}

/// Production implementation — reads from EventLog, memory, graph, config, and outcomes.
pub struct DefaultContextBuilder<E, O>
where
    E: EventLog,
    O: OutcomeReadModel,
{
    event_log: Arc<E>,
    outcomes: Arc<O>,
    config: Arc<RuntimeConfig>,
    retrieval: Option<Arc<dyn RetrievalAdapter>>,
    graph: Option<Arc<dyn cairn_graph::queries::GraphQueryService>>,
}

/// Minimal retrieval adapter so cairn-runtime does not depend on cairn-memory.
/// Callers (cairn-app) wrap a concrete RetrievalService behind this trait.
#[async_trait]
pub trait RetrievalAdapter: Send + Sync {
    async fn query_snippets(
        &self,
        project: &cairn_domain::ProjectKey,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, String>;
}

impl<E, O> DefaultContextBuilder<E, O>
where
    E: EventLog + 'static,
    O: OutcomeReadModel + 'static,
{
    pub fn new(event_log: Arc<E>, outcomes: Arc<O>, config: Arc<RuntimeConfig>) -> Self {
        Self {
            event_log,
            outcomes,
            config,
            retrieval: None,
            graph: None,
        }
    }

    pub fn with_retrieval(mut self, r: Arc<dyn RetrievalAdapter>) -> Self {
        self.retrieval = Some(r);
        self
    }

    pub fn with_graph(mut self, g: Arc<dyn cairn_graph::queries::GraphQueryService>) -> Self {
        self.graph = Some(g);
        self
    }
}

#[async_trait]
impl<E, O> ContextBuilder for DefaultContextBuilder<E, O>
where
    E: EventLog + Send + Sync + 'static,
    O: OutcomeReadModel + Send + Sync + 'static,
{
    type Error = ContextBuildError;

    async fn build(&self, input: &ContextBuilderInput) -> Result<ContextBundle, ContextBuildError> {
        // 1. Recent run events → task_history
        let run_events = self
            .event_log
            .read_by_entity(
                &cairn_store::EntityRef::Run(input.run_id.clone()),
                None,
                input.recent_event_limit,
            )
            .await?;

        let task_history: Vec<TaskSummary> = run_events
            .iter()
            .filter_map(|e| {
                if let cairn_domain::RuntimeEvent::TaskStateChanged(ev) = &e.envelope.payload {
                    Some(TaskSummary {
                        task_id: ev.task_id.as_str().to_owned(),
                        description: None,
                        state: format!("{:?}", ev.transition.to).to_lowercase(),
                        result: None,
                    })
                } else {
                    None
                }
            })
            .collect();

        // 2. Memory snippets (via RetrievalAdapter — avoids cairn-memory dep cycle)
        let memory_snippets = if let Some(retrieval) = &self.retrieval {
            retrieval
                .query_snippets(&input.project, &input.goal, input.memory_snippet_limit)
                .await
                .map_err(ContextBuildError::Memory)?
        } else {
            vec![]
        };

        // 3. Graph neighbors
        let graph_snippets: Vec<String> = if let Some(graph) = &self.graph {
            use cairn_graph::queries::TraversalDirection;
            graph
                .neighbors(
                    input.run_id.as_str(),
                    None,
                    TraversalDirection::Downstream,
                    input.graph_neighbor_limit,
                )
                .await
                .map_err(|e| ContextBuildError::Graph(e.to_string()))?
                .into_iter()
                .map(|(_, node)| format!("graph_neighbor: {}", node.node_id))
                .collect()
        } else {
            vec![]
        };

        // 4. Outcome history summary
        let outcomes = OutcomeReadModel::list_by_project(
            self.outcomes.as_ref(),
            &input.project,
            input.outcome_history_limit,
            0,
        )
        .await?;

        let outcome_snippet = if !outcomes.is_empty() {
            let success_rate = outcomes
                .iter()
                .filter(|o| o.actual_outcome == cairn_domain::events::ActualOutcome::Success)
                .count() as f64
                / outcomes.len() as f64;
            Some(format!(
                "Recent outcomes ({} samples): {:.0}% success rate",
                outcomes.len(),
                success_rate * 100.0
            ))
        } else {
            None
        };

        // 5. Agent role from RuntimeConfig
        let agent_role = Some(self.config.default_generate_model().await);

        let mut all_snippets = memory_snippets;
        all_snippets.extend(graph_snippets);
        if let Some(s) = outcome_snippet {
            all_snippets.push(s);
        }

        Ok(ContextBundle {
            run_id: input.run_id.as_str().to_owned(),
            session_id: String::new(),
            goal: input.goal.clone(),
            task_history,
            available_tools: vec![],
            memory_snippets: all_snippets,
            pending_approvals: vec![],
            agent_role,
            iteration: input.iteration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cairn_domain::{
        events::StateTransition, lifecycle::TaskState, EventEnvelope, EventId, EventSource,
        ProjectKey, RunId, RuntimeEvent, TaskId, TaskStateChanged,
    };
    use cairn_store::error::StoreError;
    use cairn_store::projections::OutcomeRecord;
    use cairn_store::{EntityRef, EventLog, EventPosition, InMemoryStore, StoredEvent};
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }
    fn run_id() -> RunId {
        RunId::new("run_ctx")
    }
    fn make_config() -> Arc<RuntimeConfig> {
        Arc::new(RuntimeConfig::new(Arc::new(InMemoryStore::new())))
    }
    fn basic_input() -> ContextBuilderInput {
        ContextBuilderInput::new(run_id(), project(), "Research cairn-rs")
    }

    struct StubLog {
        events: Vec<StoredEvent>,
    }

    #[async_trait]
    impl EventLog for StubLog {
        async fn append(
            &self,
            _: &[EventEnvelope<RuntimeEvent>],
        ) -> Result<Vec<EventPosition>, StoreError> {
            Ok(vec![])
        }
        async fn read_by_entity(
            &self,
            _: &EntityRef,
            _: Option<EventPosition>,
            _: usize,
        ) -> Result<Vec<StoredEvent>, StoreError> {
            Ok(self.events.clone())
        }
        async fn read_stream(
            &self,
            _: Option<EventPosition>,
            _: usize,
        ) -> Result<Vec<StoredEvent>, StoreError> {
            Ok(self.events.clone())
        }
        async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
            Ok(None)
        }
        async fn find_by_causation_id(&self, _: &str) -> Result<Option<EventPosition>, StoreError> {
            Ok(None)
        }
    }

    struct StubOutcomes {
        records: Vec<OutcomeRecord>,
    }

    #[async_trait]
    impl OutcomeReadModel for StubOutcomes {
        async fn get(
            &self,
            _: &cairn_domain::OutcomeId,
        ) -> Result<Option<OutcomeRecord>, StoreError> {
            Ok(None)
        }
        async fn list_by_run(&self, _: &RunId, _: usize) -> Result<Vec<OutcomeRecord>, StoreError> {
            Ok(vec![])
        }
        async fn list_by_project(
            &self,
            _: &ProjectKey,
            limit: usize,
            offset: usize,
        ) -> Result<Vec<OutcomeRecord>, StoreError> {
            Ok(self
                .records
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }
    }

    #[tokio::test]
    async fn empty_sources_produce_minimal_bundle() {
        let builder = DefaultContextBuilder::new(
            Arc::new(StubLog { events: vec![] }),
            Arc::new(StubOutcomes { records: vec![] }),
            make_config(),
        );
        let bundle = builder.build(&basic_input()).await.unwrap();
        assert_eq!(bundle.run_id, "run_ctx");
        assert_eq!(bundle.goal, "Research cairn-rs");
        assert!(bundle.task_history.is_empty());
        assert!(bundle.memory_snippets.is_empty());
    }

    #[tokio::test]
    async fn task_transitions_appear_in_history() {
        let event = StoredEvent {
            position: EventPosition(1),
            stored_at: 1_000,
            envelope: EventEnvelope::for_runtime_event(
                EventId::new("evt_t1"),
                EventSource::Runtime,
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: TaskId::new("task_abc"),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Completed,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        };
        let builder = DefaultContextBuilder::new(
            Arc::new(StubLog {
                events: vec![event],
            }),
            Arc::new(StubOutcomes { records: vec![] }),
            make_config(),
        );
        let bundle = builder.build(&basic_input()).await.unwrap();
        assert_eq!(bundle.task_history.len(), 1);
        assert_eq!(bundle.task_history[0].task_id, "task_abc");
        assert_eq!(bundle.task_history[0].state, "completed");
    }

    #[tokio::test]
    async fn outcome_history_becomes_snippet() {
        use cairn_domain::{events::ActualOutcome, OutcomeId};
        let records = (0..4)
            .map(|i| OutcomeRecord {
                outcome_id: OutcomeId::new(format!("oc_{i}")),
                run_id: RunId::new(format!("r_{i}")),
                project: project(),
                agent_type: "test".to_owned(),
                predicted_confidence: 0.8,
                actual_outcome: ActualOutcome::Success,
                recorded_at: 1_000 + i as u64,
            })
            .collect();
        let builder = DefaultContextBuilder::new(
            Arc::new(StubLog { events: vec![] }),
            Arc::new(StubOutcomes { records }),
            make_config(),
        );
        let bundle = builder.build(&basic_input()).await.unwrap();
        let has_snippet = bundle
            .memory_snippets
            .iter()
            .any(|s| s.contains("100%") && s.contains("4 samples"));
        assert!(
            has_snippet,
            "outcome summary must be a snippet; got: {:?}",
            bundle.memory_snippets
        );
    }

    #[tokio::test]
    async fn iteration_propagated() {
        let mut input = basic_input();
        input.iteration = 3;
        let builder = DefaultContextBuilder::new(
            Arc::new(StubLog { events: vec![] }),
            Arc::new(StubOutcomes { records: vec![] }),
            make_config(),
        );
        let bundle = builder.build(&input).await.unwrap();
        assert_eq!(bundle.iteration, 3);
    }

    #[tokio::test]
    async fn trait_object_usable() {
        let builder: Box<dyn ContextBuilder<Error = ContextBuildError>> =
            Box::new(DefaultContextBuilder::new(
                Arc::new(StubLog { events: vec![] }),
                Arc::new(StubOutcomes { records: vec![] }),
                make_config(),
            ));
        let bundle = builder.build(&basic_input()).await.unwrap();
        assert_eq!(bundle.run_id, "run_ctx");
    }
}
