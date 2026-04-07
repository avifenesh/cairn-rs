//! Integration tests for StandardGatherPhase.
//!
//! Uses InMemoryStore + InMemoryRetrieval to verify that gather() produces
//! a populated GatherOutput across all five data sources.

use async_trait::async_trait;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunId, RuntimeEvent,
    TaskId, TaskStateChanged, StateTransition, lifecycle::TaskState,
    DefaultSettingSet, Scope,
};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_orchestrator::gather::{GatherInput, GatherPhase};
use cairn_orchestrator::StandardGatherPhase;
use cairn_store::{EntityRef, EventLog, EventPosition, InMemoryStore, StoredEvent};
use cairn_store::error::StoreError;
use std::sync::Arc;

fn project() -> ProjectKey { ProjectKey::new("t_orch", "w_orch", "p_orch") }
fn run_id() -> RunId { RunId::new("run_gather_int") }

fn base_input() -> GatherInput {
    let mut i = GatherInput::new(run_id(), project(), "Build a cairn-rs knowledge pipeline");
    i.iteration = 1;
    i
}

/// Minimal EventLog stub — always returns the events it was constructed with.
struct FixedLog(Vec<StoredEvent>);

#[async_trait]
impl EventLog for FixedLog {
    async fn append(&self, _: &[EventEnvelope<RuntimeEvent>]) -> Result<Vec<EventPosition>, StoreError> { Ok(vec![]) }
    async fn read_by_entity(&self, _: &EntityRef, _: Option<EventPosition>, _: usize) -> Result<Vec<StoredEvent>, StoreError> { Ok(self.0.clone()) }
    async fn read_stream(&self, _: Option<EventPosition>, _: usize) -> Result<Vec<StoredEvent>, StoreError> { Ok(self.0.clone()) }
    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> { Ok(None) }
    async fn find_by_causation_id(&self, _: &str) -> Result<Option<EventPosition>, StoreError> { Ok(None) }
}

fn task_completed_event(task_id: &str) -> StoredEvent {
    StoredEvent {
        position: EventPosition(1),
        stored_at: 1_000,
        envelope: EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_{task_id}")),
            EventSource::Runtime,
            RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: project(),
                task_id: TaskId::new(task_id),
                transition: StateTransition { from: Some(TaskState::Running), to: TaskState::Completed },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        ),
    }
}

// ── Test: empty sources → baseline output ────────────────────────────────────

#[tokio::test]
async fn empty_sources_produce_baseline_output() {
    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(vec![]))).build();
    let output = phase.gather(&base_input()).await.unwrap();

    assert_eq!(output.run_id, "run_gather_int");
    assert_eq!(output.goal, "Build a cairn-rs knowledge pipeline");
    assert_eq!(output.iteration, 1);
    assert!(output.task_history.is_empty());
    assert!(output.memory_chunks.is_empty());
    assert!(output.operator_defaults.is_empty());
    assert!(output.latest_checkpoint_data.is_none());
    assert!(output.graph_neighbors.is_empty());
}

// ── Test: task transitions in events → appear in task_history ────────────────

#[tokio::test]
async fn task_state_events_become_task_history() {
    let events = vec![
        task_completed_event("task_fetch"),
        task_completed_event("task_analyse"),
    ];
    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(events))).build();
    let output = phase.gather(&base_input()).await.unwrap();

    assert_eq!(output.task_history.len(), 2, "both tasks must appear in history");
    assert!(output.task_history.iter().any(|t| t.task_id == "task_fetch"));
    assert!(output.task_history.iter().any(|t| t.task_id == "task_analyse"));
    for task in &output.task_history {
        assert_eq!(task.state, "completed");
    }
}

// ── Test: memory retrieval via InMemoryRetrieval ──────────────────────────────

#[tokio::test]
async fn gather_with_retrieval_returns_populated_chunks() {
    let doc_store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(doc_store.clone(), ParagraphChunker::default());

    pipeline.submit(IngestRequest {
        document_id: cairn_domain::KnowledgeDocumentId::new("doc_orch_1"),
        source_id:   cairn_domain::SourceId::new("src_orch"),
        source_type: SourceType::PlainText,
        project:     project(),
        content:     "cairn-rs is an event-sourced knowledge pipeline for AI agents."
                         .to_owned(),
        tags:         vec![],
        corpus_id:    None,
        import_id:    None,
        bundle_source_id: None,
    }).await.unwrap();

    let retrieval = Arc::new(InMemoryRetrieval::new(doc_store));
    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(vec![])))
        .with_retrieval(retrieval)
        .build();

    let mut input = base_input();
    input.goal = "event-sourced knowledge pipeline".to_owned();
    let output = phase.gather(&input).await.unwrap();

    assert!(!output.memory_chunks.is_empty(), "memory chunks must be non-empty");
    assert!(
        output.memory_chunks.iter().any(|c| c.text.contains("event-sourced")),
        "retrieved chunk must contain goal keywords"
    );
}

// ── Test: operator defaults via InMemoryStore ─────────────────────────────────

#[tokio::test]
async fn gather_reads_operator_defaults_from_store() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[EventEnvelope::for_runtime_event(
        EventId::new("evt_default"),
        EventSource::System,
        RuntimeEvent::DefaultSettingSet(DefaultSettingSet {
            scope:    Scope::System,
            scope_id: "system".to_owned(),
            key:      "generate_model".to_owned(),
            value:    serde_json::json!("qwen3.5:9b"),
        }),
    )]).await.unwrap();

    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(vec![])))
        .with_defaults(store.clone())
        .build();

    let output = phase.gather(&base_input()).await.unwrap();

    assert_eq!(
        output.operator_defaults.get("generate_model"),
        Some(&serde_json::json!("qwen3.5:9b")),
        "operator defaults must contain generate_model=qwen3.5:9b"
    );
}

// ── Test: raw recent_events passed through ───────────────────────────────────

#[tokio::test]
async fn recent_events_are_passed_through() {
    let events = vec![task_completed_event("task_passthrough")];
    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(events))).build();
    let output = phase.gather(&base_input()).await.unwrap();
    assert_eq!(output.recent_events.len(), 1);
}

// ── Test: GatherPhase as dyn trait ────────────────────────────────────────────

#[tokio::test]
async fn gather_phase_works_as_dyn_trait() {
    let phase: Box<dyn GatherPhase> = Box::new(
        StandardGatherPhase::builder(Arc::new(FixedLog(vec![]))).build()
    );
    let output = phase.gather(&base_input()).await.unwrap();
    assert_eq!(output.run_id, "run_gather_int");
    assert_eq!(output.project, project());
}
