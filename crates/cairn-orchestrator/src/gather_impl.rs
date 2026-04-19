//! StandardGatherPhase — concrete GATHER phase implementation.
//!
//! Reads from five sources in parallel and populates a GatherOutput:
//!   1. EventLog::read_by_entity   → recent run events
//!   2. RetrievalService::query    → semantic memory chunks
//!   3. DefaultsReadModel          → operator settings
//!   4. CheckpointReadModel        → latest checkpoint data
//!   5. GraphQueryService          → downstream graph neighbors

use crate::context::{GatherOutput, OrchestrationContext};
use crate::error::OrchestratorError;
use crate::gather::GatherPhase;
use async_trait::async_trait;
use cairn_domain::Scope;
use cairn_graph::queries::{GraphQueryService, TraversalDirection};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};
use cairn_store::projections::{CheckpointReadModel, DefaultsReadModel};
use cairn_store::{EntityRef, EventLog};
use std::sync::Arc;

/// Production implementation of GatherPhase.
///
/// All five sources are optional. When absent the corresponding output
/// field is left at its default (empty vec / None). The DECIDE phase
/// works with partial context.
pub struct StandardGatherPhase {
    event_log: Arc<dyn EventLog + Send + Sync>,
    retrieval: Option<Arc<dyn RetrievalService>>,
    defaults: Option<Arc<dyn DefaultsReadModel + Send + Sync>>,
    checkpoints: Option<Arc<dyn CheckpointReadModel + Send + Sync>>,
    graph: Option<Arc<dyn GraphQueryService>>,
    /// How many recent events to read per iteration.
    event_limit: usize,
    /// How many memory chunks to retrieve.
    memory_limit: usize,
    /// How many graph neighbors to include.
    graph_limit: usize,
}

impl StandardGatherPhase {
    /// Start building a StandardGatherPhase with a required EventLog.
    pub fn builder(event_log: Arc<dyn EventLog + Send + Sync>) -> StandardGatherPhaseBuilder {
        StandardGatherPhaseBuilder {
            event_log,
            retrieval: None,
            defaults: None,
            checkpoints: None,
            graph: None,
            event_limit: 50,
            memory_limit: 5,
            graph_limit: 10,
        }
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

pub struct StandardGatherPhaseBuilder {
    event_log: Arc<dyn EventLog + Send + Sync>,
    retrieval: Option<Arc<dyn RetrievalService>>,
    defaults: Option<Arc<dyn DefaultsReadModel + Send + Sync>>,
    checkpoints: Option<Arc<dyn CheckpointReadModel + Send + Sync>>,
    graph: Option<Arc<dyn GraphQueryService>>,
    event_limit: usize,
    memory_limit: usize,
    graph_limit: usize,
}

impl StandardGatherPhaseBuilder {
    pub fn with_retrieval(mut self, r: Arc<dyn RetrievalService>) -> Self {
        self.retrieval = Some(r);
        self
    }
    pub fn with_defaults(mut self, d: Arc<dyn DefaultsReadModel + Send + Sync>) -> Self {
        self.defaults = Some(d);
        self
    }
    pub fn with_checkpoints(mut self, c: Arc<dyn CheckpointReadModel + Send + Sync>) -> Self {
        self.checkpoints = Some(c);
        self
    }
    pub fn with_graph(mut self, g: Arc<dyn GraphQueryService>) -> Self {
        self.graph = Some(g);
        self
    }
    pub fn with_event_limit(mut self, n: usize) -> Self {
        self.event_limit = n;
        self
    }
    pub fn with_memory_limit(mut self, n: usize) -> Self {
        self.memory_limit = n;
        self
    }
    pub fn with_graph_limit(mut self, n: usize) -> Self {
        self.graph_limit = n;
        self
    }
    pub fn build(self) -> StandardGatherPhase {
        StandardGatherPhase {
            event_log: self.event_log,
            retrieval: self.retrieval,
            defaults: self.defaults,
            checkpoints: self.checkpoints,
            graph: self.graph,
            event_limit: self.event_limit,
            memory_limit: self.memory_limit,
            graph_limit: self.graph_limit,
        }
    }
}

// ── GatherPhase impl ──────────────────────────────────────────────────────────

#[async_trait]
impl GatherPhase for StandardGatherPhase {
    async fn gather(&self, ctx: &OrchestrationContext) -> Result<GatherOutput, OrchestratorError> {
        // 1. Recent run events
        let recent_events = self
            .event_log
            .read_by_entity(&EntityRef::Run(ctx.run_id.clone()), None, self.event_limit)
            .await
            .map_err(OrchestratorError::Store)?;

        // 2. Memory chunks (semantic retrieval)
        let memory_chunks = if let Some(retrieval) = &self.retrieval {
            retrieval
                .query(RetrievalQuery {
                    project: ctx.project.clone(),
                    query_text: ctx.goal.clone(),
                    mode: RetrievalMode::LexicalOnly,
                    reranker: RerankerStrategy::None,
                    limit: self.memory_limit,
                    metadata_filters: vec![],
                    scoring_policy: None,
                })
                .await
                .map_err(|e| OrchestratorError::Gather(e.to_string()))?
                .results
        } else {
            vec![]
        };

        // 3. Operator settings
        let operator_settings = if let Some(defaults) = &self.defaults {
            defaults
                .list_by_scope(Scope::System, "system")
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        // 4. Latest checkpoint
        let checkpoint = if let Some(cp) = &self.checkpoints {
            cp.latest_for_run(&ctx.run_id).await.unwrap_or(None)
        } else {
            None
        };

        // 5. Graph neighbors
        let graph_nodes = if let Some(graph) = &self.graph {
            graph
                .neighbors(
                    ctx.run_id.as_str(),
                    None,
                    TraversalDirection::Downstream,
                    self.graph_limit,
                )
                .await
                .map_err(|e| OrchestratorError::Gather(e.to_string()))?
                .into_iter()
                .map(|(_, node)| node)
                .collect()
        } else {
            vec![]
        };

        Ok(GatherOutput {
            memory_chunks,
            recent_events,
            graph_nodes,
            operator_settings,
            checkpoint,
            // T5-H1: surface the loop-maintained step history so
            // LlmDecidePhase::build_user_message sees prior steps.
            step_history: ctx.step_history.clone(),
        })
    }
}
