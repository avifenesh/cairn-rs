use crate::{ProjectKey, RunId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointStrategy {
    pub strategy_id: String,
    pub project: ProjectKey,
    pub run_id: RunId,
    pub interval_ms: u64,
    pub max_checkpoints: u32,
    pub trigger_on_task_complete: bool,
}
