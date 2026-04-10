//! Integration tests for StandardGatherPhase.
//!
//! Uses InMemoryStore + InMemoryRetrieval to verify that gather() populates
//! all five context sources.

use async_trait::async_trait;
use cairn_domain::{
    DefaultSettingSet, EventEnvelope, EventId, EventSource, ProjectKey, RunId, RuntimeEvent, Scope,
    SessionId,
};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_orchestrator::context::OrchestrationContext;
use cairn_orchestrator::gather::GatherPhase;
use cairn_orchestrator::StandardGatherPhase;
use cairn_store::error::StoreError;
use cairn_store::{EntityRef, EventLog, EventPosition, InMemoryStore, StoredEvent};
use std::path::PathBuf;
use std::sync::Arc;

fn project() -> ProjectKey {
    ProjectKey::new("t_it", "w_it", "p_it")
}
fn run_id() -> RunId {
    RunId::new("run_gather_it")
}

fn base_ctx() -> OrchestrationContext {
    OrchestrationContext {
        project: project(),
        session_id: SessionId::new("sess_it"),
        run_id: run_id(),
        task_id: None,
        iteration: 1,
        goal: "Analyse cairn-rs event-sourcing architecture".to_owned(),
        agent_type: "orchestrator".to_owned(),
        run_started_at_ms: 1_000_000,
        working_dir: PathBuf::from("."),
        run_mode: cairn_domain::decisions::RunMode::Direct,
        discovered_tool_names: vec![],
    }
}

struct FixedLog(Vec<StoredEvent>);

#[async_trait]
impl EventLog for FixedLog {
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
        Ok(self.0.clone())
    }
    async fn read_stream(
        &self,
        _: Option<EventPosition>,
        _: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        Ok(self.0.clone())
    }
    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
        Ok(None)
    }
    async fn find_by_causation_id(&self, _: &str) -> Result<Option<EventPosition>, StoreError> {
        Ok(None)
    }
}

// ── Test 1: empty sources → minimal output ───────────────────────────────────

#[tokio::test]
async fn empty_sources_produce_empty_output() {
    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(vec![]))).build();
    let output = phase.gather(&base_ctx()).await.unwrap();

    assert!(output.recent_events.is_empty());
    assert!(output.memory_chunks.is_empty());
    assert!(output.operator_settings.is_empty());
    assert!(output.checkpoint.is_none());
    assert!(output.graph_nodes.is_empty());
}

// ── Test 2: events pass through ──────────────────────────────────────────────

#[tokio::test]
async fn recent_events_passed_through() {
    use cairn_domain::{lifecycle::TaskState, StateTransition, TaskId, TaskStateChanged};

    let event = StoredEvent {
        position: EventPosition(1),
        stored_at: 1_000,
        envelope: EventEnvelope::for_runtime_event(
            EventId::new("evt_task"),
            EventSource::Runtime,
            RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: project(),
                task_id: TaskId::new("task_x"),
                transition: StateTransition {
                    from: Some(TaskState::Running),
                    to: TaskState::Completed,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
        ),
    };

    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(vec![event]))).build();
    let output = phase.gather(&base_ctx()).await.unwrap();

    assert_eq!(output.recent_events.len(), 1);
}

// ── Test 3: memory retrieval via InMemoryRetrieval ────────────────────────────

#[tokio::test]
async fn gather_retrieves_memory_chunks() {
    let doc_store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(doc_store.clone(), ParagraphChunker::default());

    pipeline
        .submit(IngestRequest {
            document_id: cairn_domain::KnowledgeDocumentId::new("doc_g1"),
            source_id: cairn_domain::SourceId::new("src_g"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "cairn-rs uses event-sourcing for durability and replay.".to_owned(),
            tags: vec![],
            corpus_id: None,
            import_id: None,
            bundle_source_id: None,
        })
        .await
        .unwrap();

    let retrieval = Arc::new(InMemoryRetrieval::new(doc_store));
    let mut ctx = base_ctx();
    ctx.goal = "event-sourcing durability".to_owned();

    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(vec![])))
        .with_retrieval(retrieval)
        .build();

    let output = phase.gather(&ctx).await.unwrap();

    assert!(
        !output.memory_chunks.is_empty(),
        "memory chunks must be non-empty"
    );
    assert!(
        output
            .memory_chunks
            .iter()
            .any(|c| c.chunk.text.contains("event-sourcing")),
        "chunk text must contain query keywords"
    );
}

// ── Test 4: operator defaults via InMemoryStore ───────────────────────────────

#[tokio::test]
async fn gather_reads_operator_defaults() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_def"),
            EventSource::System,
            RuntimeEvent::DefaultSettingSet(DefaultSettingSet {
                scope: Scope::System,
                scope_id: "system".to_owned(),
                key: "generate_model".to_owned(),
                value: serde_json::json!("qwen3.5:9b"),
            }),
        )])
        .await
        .unwrap();

    let phase = StandardGatherPhase::builder(Arc::new(FixedLog(vec![])))
        .with_defaults(store.clone())
        .build();

    let output = phase.gather(&base_ctx()).await.unwrap();

    assert!(
        output
            .operator_settings
            .iter()
            .any(|s| s.key == "generate_model"),
        "operator settings must contain generate_model"
    );
}

// ── Test 5: GatherPhase as dyn trait ─────────────────────────────────────────

#[tokio::test]
async fn gather_phase_as_dyn_trait() {
    let phase: Box<dyn GatherPhase> =
        Box::new(StandardGatherPhase::builder(Arc::new(FixedLog(vec![]))).build());
    let output = phase.gather(&base_ctx()).await.unwrap();
    assert!(output.recent_events.is_empty());
}
