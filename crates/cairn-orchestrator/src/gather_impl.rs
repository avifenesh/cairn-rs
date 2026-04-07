//! StandardGatherPhase — concrete GATHER phase implementation.
//!
//! Reads from five sources:
//!   1. EventLog::read_by_entity   → recent run events → task_history
//!   2. RetrievalService::query    → semantic memory chunks
//!   3. DefaultsReadModel          → operator defaults (model, tokens, etc.)
//!   4. CheckpointReadModel        → latest checkpoint data for the run
//!   5. GraphQueryService          → downstream neighbors of the run node

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::RuntimeEvent;
use cairn_graph::queries::{GraphQueryService, TraversalDirection};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};
use cairn_store::projections::{CheckpointReadModel, DefaultsReadModel};
use cairn_store::{EntityRef, EventLog};
use cairn_domain::Scope;

use crate::gather::{
    GatherError, GatherInput, GatherOutput, GatherPhase, GraphNeighbor, MemoryChunk, TaskSummary,
};

/// Production implementation of the GATHER phase.
///
/// All five sources are optional (`Option<Arc<dyn ...>>`).  When a source is
/// absent the corresponding output field is left empty — the DECIDE phase can
/// still work with partial context.
pub struct StandardGatherPhase {
    event_log:  Arc<dyn EventLog + Send + Sync>,
    retrieval:  Option<Arc<dyn RetrievalService>>,
    defaults:   Option<Arc<dyn DefaultsReadModel + Send + Sync>>,
    checkpoint: Option<Arc<dyn CheckpointReadModel + Send + Sync>>,
    graph:      Option<Arc<dyn GraphQueryService>>,
}

impl StandardGatherPhase {
    pub fn builder(event_log: Arc<dyn EventLog + Send + Sync>) -> StandardGatherPhaseBuilder {
        StandardGatherPhaseBuilder {
            event_log,
            retrieval:  None,
            defaults:   None,
            checkpoint: None,
            graph:      None,
        }
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

pub struct StandardGatherPhaseBuilder {
    event_log:  Arc<dyn EventLog + Send + Sync>,
    retrieval:  Option<Arc<dyn RetrievalService>>,
    defaults:   Option<Arc<dyn DefaultsReadModel + Send + Sync>>,
    checkpoint: Option<Arc<dyn CheckpointReadModel + Send + Sync>>,
    graph:      Option<Arc<dyn GraphQueryService>>,
}

impl StandardGatherPhaseBuilder {
    pub fn with_retrieval(mut self, r: Arc<dyn RetrievalService>) -> Self {
        self.retrieval = Some(r); self
    }
    pub fn with_defaults(mut self, d: Arc<dyn DefaultsReadModel + Send + Sync>) -> Self {
        self.defaults = Some(d); self
    }
    pub fn with_checkpoint(mut self, c: Arc<dyn CheckpointReadModel + Send + Sync>) -> Self {
        self.checkpoint = Some(c); self
    }
    pub fn with_graph(mut self, g: Arc<dyn GraphQueryService>) -> Self {
        self.graph = Some(g); self
    }
    pub fn build(self) -> StandardGatherPhase {
        StandardGatherPhase {
            event_log:  self.event_log,
            retrieval:  self.retrieval,
            defaults:   self.defaults,
            checkpoint: self.checkpoint,
            graph:      self.graph,
        }
    }
}

// ── GatherPhase impl ──────────────────────────────────────────────────────────

#[async_trait]
impl GatherPhase for StandardGatherPhase {
    async fn gather(&self, input: &GatherInput) -> Result<GatherOutput, GatherError> {
        // ── 1. Recent run events ──────────────────────────────────────────────
        let recent_events = self.event_log
            .read_by_entity(
                &EntityRef::Run(input.run_id.clone()),
                None,
                input.event_limit,
            )
            .await?;

        // Derive task_history from TaskStateChanged events.
        let task_history: Vec<TaskSummary> = recent_events.iter().filter_map(|e| {
            if let RuntimeEvent::TaskStateChanged(ev) = &e.envelope.payload {
                Some(TaskSummary {
                    task_id:     ev.task_id.as_str().to_owned(),
                    state:       format!("{:?}", ev.transition.to).to_lowercase(),
                    description: None,
                    result:      None,
                })
            } else { None }
        }).collect();

        // ── 2. Memory chunks (semantic retrieval) ─────────────────────────────
        let memory_chunks = if let Some(retrieval) = &self.retrieval {
            retrieval
                .query(RetrievalQuery {
                    project: input.project.clone(),
                    query_text: input.goal.clone(),
                    mode: RetrievalMode::LexicalOnly,
                    reranker: RerankerStrategy::None,
                    limit: input.memory_limit,
                    metadata_filters: vec![],
                    scoring_policy: None,
                })
                .await
                .map_err(|e| GatherError::Memory(e.to_string()))?
                .results
                .into_iter()
                .map(|r| MemoryChunk {
                    chunk_id: r.chunk.chunk_id.as_str().to_owned(),
                    text:     r.chunk.text,
                    score:    r.score,
                })
                .collect()
        } else {
            vec![]
        };

        // ── 3. Operator defaults ──────────────────────────────────────────────
        let operator_defaults = if let Some(defaults) = &self.defaults {
            defaults
                .list_by_scope(Scope::System, "system")
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|s| (s.key, s.value))
                .collect::<HashMap<_, _>>()
        } else {
            HashMap::new()
        };

        // ── 4. Latest checkpoint data ─────────────────────────────────────────
        let latest_checkpoint_data = if let Some(checkpoints) = &self.checkpoint {
            checkpoints
                .latest_for_run(&input.run_id)
                .await
                .ok()
                .flatten()
                .and_then(|c| c.data)
        } else {
            None
        };

        // ── 5. Graph neighbors ────────────────────────────────────────────────
        let graph_neighbors = if let Some(graph) = &self.graph {
            graph
                .neighbors(
                    input.run_id.as_str(),
                    None,
                    TraversalDirection::Downstream,
                    input.graph_limit,
                )
                .await
                .map_err(|e| GatherError::Graph(e.to_string()))?
                .into_iter()
                .map(|(edge, node)| GraphNeighbor {
                    node_id: node.node_id,
                    kind:    format!("{:?}", edge.kind),
                })
                .collect()
        } else {
            vec![]
        };

        Ok(GatherOutput {
            run_id:                  input.run_id.as_str().to_owned(),
            project:                 input.project.clone(),
            goal:                    input.goal.clone(),
            iteration:               input.iteration,
            recent_events,
            memory_chunks,
            operator_defaults,
            latest_checkpoint_data,
            graph_neighbors,
            task_history,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{
        EventEnvelope, EventId, EventSource, ProjectKey, RunId, RuntimeEvent,
        TaskId, TaskStateChanged, StateTransition, lifecycle::TaskState,
    };
    use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
    use cairn_memory::ingest::{IngestRequest, SourceType};
    use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
    use cairn_store::{EventLog, EventPosition, InMemoryStore, StoredEvent, EntityRef};
    use cairn_store::error::StoreError;
    use std::sync::Arc;

    fn project() -> ProjectKey { ProjectKey::new("t", "w", "p") }
    fn run_id() -> RunId { RunId::new("run_gather_test") }

    fn base_input() -> GatherInput {
        let mut i = GatherInput::new(run_id(), project(), "Build a cairn-rs retrieval pipeline");
        i.iteration = 2;
        i
    }

    // ── Minimal EventLog stub ─────────────────────────────────────────────────

    struct StubLog(Vec<StoredEvent>);

    #[async_trait]
    impl EventLog for StubLog {
        async fn append(&self, _: &[EventEnvelope<RuntimeEvent>]) -> Result<Vec<EventPosition>, StoreError> { Ok(vec![]) }
        async fn read_by_entity(&self, _: &EntityRef, _: Option<EventPosition>, _: usize) -> Result<Vec<StoredEvent>, StoreError> { Ok(self.0.clone()) }
        async fn read_stream(&self, _: Option<EventPosition>, _: usize) -> Result<Vec<StoredEvent>, StoreError> { Ok(self.0.clone()) }
        async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> { Ok(None) }
        async fn find_by_causation_id(&self, _: &str) -> Result<Option<EventPosition>, StoreError> { Ok(None) }
    }

    fn task_state_event(task_id: &str, to: TaskState) -> StoredEvent {
        StoredEvent {
            position: EventPosition(1),
            stored_at: 1_000,
            envelope: EventEnvelope::for_runtime_event(
                EventId::new(format!("evt_{task_id}")),
                EventSource::Runtime,
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: TaskId::new(task_id),
                    transition: StateTransition { from: Some(TaskState::Running), to },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        }
    }

    // ── gather with empty sources returns baseline output ────────────────────

    #[tokio::test]
    async fn gather_empty_sources_returns_baseline() {
        let phase = StandardGatherPhase::builder(Arc::new(StubLog(vec![])))
            .build();
        let output = phase.gather(&base_input()).await.unwrap();

        assert_eq!(output.run_id, "run_gather_test");
        assert_eq!(output.goal, "Build a cairn-rs retrieval pipeline");
        assert_eq!(output.iteration, 2);
        assert!(output.task_history.is_empty());
        assert!(output.memory_chunks.is_empty());
        assert!(output.operator_defaults.is_empty());
        assert!(output.latest_checkpoint_data.is_none());
        assert!(output.graph_neighbors.is_empty());
    }

    // ── task_history derived from TaskStateChanged events ─────────────────────

    #[tokio::test]
    async fn task_transitions_become_task_history() {
        let events = vec![
            task_state_event("task_research", TaskState::Completed),
            task_state_event("task_write",    TaskState::Failed),
        ];
        let phase = StandardGatherPhase::builder(Arc::new(StubLog(events)))
            .build();
        let output = phase.gather(&base_input()).await.unwrap();

        assert_eq!(output.task_history.len(), 2);
        let ids: Vec<&str> = output.task_history.iter().map(|t| t.task_id.as_str()).collect();
        assert!(ids.contains(&"task_research"));
        assert!(ids.contains(&"task_write"));

        let research = output.task_history.iter().find(|t| t.task_id == "task_research").unwrap();
        assert_eq!(research.state, "completed");
        let write = output.task_history.iter().find(|t| t.task_id == "task_write").unwrap();
        assert_eq!(write.state, "failed");
    }

    // ── memory retrieval using InMemoryRetrieval ──────────────────────────────

    #[tokio::test]
    async fn gather_with_retrieval_returns_memory_chunks() {
        let doc_store = Arc::new(InMemoryDocumentStore::new());
        let pipeline = IngestPipeline::new(doc_store.clone(), ParagraphChunker::default());
        pipeline.submit(IngestRequest {
            document_id: cairn_domain::KnowledgeDocumentId::new("doc_gather_1"),
            source_id:   cairn_domain::SourceId::new("src_gather"),
            source_type: SourceType::PlainText,
            project:     project(),
            content:     "cairn-rs retrieval pipeline uses chunking and lexical search."
                .to_owned(),
            tags:        vec![],
            corpus_id:   None,
            import_id:   None,
            bundle_source_id: None,
        }).await.unwrap();

        let retrieval = Arc::new(InMemoryRetrieval::new(doc_store));
        let phase = StandardGatherPhase::builder(Arc::new(StubLog(vec![])))
            .with_retrieval(retrieval)
            .build();

        let mut input = base_input();
        input.goal = "retrieval pipeline chunking".to_owned();
        let output = phase.gather(&input).await.unwrap();

        assert!(!output.memory_chunks.is_empty(), "memory chunks must be non-empty");
        assert!(output.memory_chunks.iter().any(|c| c.text.contains("retrieval")));
    }

    // ── operator defaults from InMemoryStore ─────────────────────────────────

    #[tokio::test]
    async fn gather_reads_operator_defaults() {
        use cairn_domain::{DefaultSettingSet, RuntimeEvent, Scope};
        use cairn_store::EventLog;

        let store = Arc::new(InMemoryStore::new());
        // Write a system-scoped default.
        store.append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_def"),
            EventSource::System,
            RuntimeEvent::DefaultSettingSet(DefaultSettingSet {
                scope:    Scope::System,
                scope_id: "system".to_owned(),
                key:      "max_tokens".to_owned(),
                value:    serde_json::json!(4096),
            }),
        )]).await.unwrap();

        let phase = StandardGatherPhase::builder(Arc::new(StubLog(vec![])))
            .with_defaults(store.clone())
            .build();

        let output = phase.gather(&base_input()).await.unwrap();
        assert_eq!(
            output.operator_defaults.get("max_tokens"),
            Some(&serde_json::json!(4096)),
            "operator defaults must include max_tokens=4096"
        );
    }

    // ── recent_events are passed through ─────────────────────────────────────

    #[tokio::test]
    async fn gather_passes_raw_events_through() {
        let events = vec![task_state_event("task_x", TaskState::Completed)];
        let phase = StandardGatherPhase::builder(Arc::new(StubLog(events.clone())))
            .build();
        let output = phase.gather(&base_input()).await.unwrap();
        assert_eq!(output.recent_events.len(), 1);
        assert_eq!(output.recent_events[0].position, EventPosition(1));
    }

    // ── GatherPhase trait is object-safe ─────────────────────────────────────

    #[tokio::test]
    async fn gather_phase_as_dyn_trait() {
        let phase: Box<dyn GatherPhase> = Box::new(
            StandardGatherPhase::builder(Arc::new(StubLog(vec![]))).build()
        );
        let output = phase.gather(&base_input()).await.unwrap();
        assert_eq!(output.run_id, "run_gather_test");
    }
}
