use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{EvalRunReadModel, EvalRunRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::eval_runs::EvalRunService;

pub struct EvalRunServiceImpl<S> {
    store: Arc<S>,
}

impl<S> EvalRunServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> EvalRunService for EvalRunServiceImpl<S>
where
    S: EventLog + EvalRunReadModel + 'static,
{
    async fn start(
        &self,
        project: &ProjectKey,
        eval_run_id: EvalRunId,
        subject_kind: String,
        evaluator_type: String,
    ) -> Result<EvalRunRecord, RuntimeError> {
        if EvalRunReadModel::get(self.store.as_ref(), &eval_run_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "eval_run",
                id: eval_run_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::EvalRunStarted(EvalRunStarted {
            project: project.clone(),
            eval_run_id: eval_run_id.clone(),
            subject_kind,
            evaluator_type,
            started_at: now_millis(),
            prompt_asset_id: None,
            prompt_version_id: None,
            prompt_release_id: None,
            created_by: None,
        }));

        self.store.append(&[event]).await?;

        EvalRunReadModel::get(self.store.as_ref(), &eval_run_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("eval_run not found after start".into()))
    }

    async fn complete(
        &self,
        eval_run_id: &EvalRunId,
        success: bool,
        error_message: Option<String>,
    ) -> Result<EvalRunRecord, RuntimeError> {
        let existing = EvalRunReadModel::get(self.store.as_ref(), eval_run_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "eval_run",
                id: eval_run_id.to_string(),
            })?;

        if existing.completed_at.is_some() {
            return Err(RuntimeError::Conflict {
                entity: "eval_run",
                id: eval_run_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::EvalRunCompleted(EvalRunCompleted {
            project: existing.project.clone(),
            eval_run_id: eval_run_id.clone(),
            success,
            error_message,
            subject_node_id: None,
            completed_at: now_millis(),
        }));

        self.store.append(&[event]).await?;

        EvalRunReadModel::get(self.store.as_ref(), eval_run_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("eval_run not found after complete".into()))
    }

    async fn get(&self, eval_run_id: &EvalRunId) -> Result<Option<EvalRunRecord>, RuntimeError> {
        Ok(EvalRunReadModel::get(self.store.as_ref(), eval_run_id).await?)
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EvalRunRecord>, RuntimeError> {
        Ok(self.store.list_by_project(project, limit, offset).await?)
    }
}
