use async_trait::async_trait;
use cairn_domain::events::ActualOutcome;
use cairn_domain::{OutcomeId, ProjectKey, RunId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Current-state record for an outcome projection.
///
/// Part of the evaluator–optimizer feedback loop: tracks predicted confidence
/// vs actual outcome per run, enabling calibration analysis over time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutcomeRecord {
    pub outcome_id: OutcomeId,
    pub run_id: RunId,
    pub project: ProjectKey,
    pub agent_type: String,
    pub predicted_confidence: f64,
    pub actual_outcome: ActualOutcome,
    pub recorded_at: u64,
}

/// Read-model for outcome tracking.
#[async_trait]
pub trait OutcomeReadModel: Send + Sync {
    async fn get(&self, outcome_id: &OutcomeId) -> Result<Option<OutcomeRecord>, StoreError>;

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<OutcomeRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<OutcomeRecord>, StoreError>;
}
