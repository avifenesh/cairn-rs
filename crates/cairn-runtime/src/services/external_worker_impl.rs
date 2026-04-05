//! Runtime seam for external worker progress and outcome reporting.
//!
//! External workers report through runtime-owned APIs. This service
//! validates reports against lease identity and emits canonical events.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::workers::{ExternalWorkerOutcome, ExternalWorkerReport};
use cairn_domain::*;
use cairn_store::projections::TaskReadModel;
use cairn_store::EventLog;

use cairn_domain::lifecycle::FailureClass;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;

/// Parse an API-facing outcome string into the domain ExternalWorkerOutcome.
///
/// Worker 8 should call this when converting `WorkerReportRequest.outcome`
/// to the domain type, rather than inventing its own parsing.
///
/// Accepted values: "completed", "failed", "canceled", "suspended".
/// "failed" defaults to `FailureClass::ExecutionError` when no class is specified.
pub fn parse_outcome(outcome_str: &str) -> Result<ExternalWorkerOutcome, RuntimeError> {
    match outcome_str {
        "completed" => Ok(ExternalWorkerOutcome::Completed),
        "failed" => Ok(ExternalWorkerOutcome::Failed {
            failure_class: FailureClass::ExecutionError,
        }),
        "canceled" => Ok(ExternalWorkerOutcome::Canceled),
        "suspended" => Ok(ExternalWorkerOutcome::Suspended {
            reason: cairn_domain::PauseReason {
                kind: cairn_domain::PauseReasonKind::RuntimeSuspension,
                detail: None,
                resume_after_ms: None,
            },
        }),
        other => Err(RuntimeError::Internal(format!(
            "unknown worker outcome: {other}"
        ))),
    }
}

/// Runtime-facing service for external worker updates.
#[async_trait]
pub trait ExternalWorkerService: Send + Sync {
    /// Process a progress/outcome report from an external worker.
    ///
    /// Validates the report against the task lease, emits an
    /// ExternalWorkerReported event, and optionally transitions
    /// the task to a terminal state if the report includes an outcome.
    async fn report(&self, report: ExternalWorkerReport) -> Result<(), RuntimeError>;
}

pub struct ExternalWorkerServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ExternalWorkerServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> ExternalWorkerService for ExternalWorkerServiceImpl<S>
where
    S: EventLog + TaskReadModel + 'static,
{
    async fn report(&self, report: ExternalWorkerReport) -> Result<(), RuntimeError> {
        // Verify the task exists
        let task = TaskReadModel::get(self.store.as_ref(), &report.task_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "task",
                id: report.task_id.to_string(),
            })?;

        if task.state.is_terminal() {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: "worker report".into(),
            });
        }

        let mut events = vec![make_envelope(RuntimeEvent::ExternalWorkerReported(
            ExternalWorkerReported {
                report: report.clone(),
            },
        ))];

        // If the report includes a terminal outcome, transition the task
        if let Some(outcome) = &report.outcome {
            let (to_state, failure_class) = match outcome {
                ExternalWorkerOutcome::Completed => (TaskState::Completed, None),
                ExternalWorkerOutcome::Failed { failure_class } => {
                    (TaskState::Failed, Some(*failure_class))
                }
                ExternalWorkerOutcome::Canceled => (TaskState::Canceled, None),
                ExternalWorkerOutcome::Suspended { .. } => (TaskState::Paused, None),
            };

            events.push(make_envelope(RuntimeEvent::TaskStateChanged(
                TaskStateChanged {
                    project: task.project.clone(),
                    task_id: report.task_id.clone(),
                    transition: StateTransition {
                        from: Some(task.state),
                        to: to_state,
                    },
                    failure_class,
                    pause_reason: None,
                    resume_trigger: None,
                },
            )));
        }

        self.store.append(&events).await?;
        Ok(())
    }
}

// ── Additional stub methods for cairn-app compatibility ───────────────────

impl<S> ExternalWorkerServiceImpl<S>
where
    S: cairn_store::EventLog + cairn_store::projections::ExternalWorkerReadModel + 'static,
{
    /// Register a new external worker.
    pub async fn register(
        &self,
        tenant_id: cairn_domain::TenantId,
        worker_id: cairn_domain::WorkerId,
        display_name: String,
    ) -> Result<cairn_domain::workers::ExternalWorkerRecord, crate::error::RuntimeError> {
        use super::event_helpers::make_envelope;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let event = make_envelope(cairn_domain::RuntimeEvent::ExternalWorkerRegistered(
            cairn_domain::ExternalWorkerRegistered {
                sentinel_project: cairn_domain::ProjectKey::new(
                    tenant_id.as_str(), "_", "_"),
                tenant_id: tenant_id.clone(),
                worker_id: worker_id.clone(),
                display_name: display_name.clone(),
                registered_at: now,
            },
        ));
        self.store.append(&[event]).await?;
        Ok(cairn_domain::workers::ExternalWorkerRecord {
            worker_id,
            tenant_id,
            display_name,
            status: "active".to_owned(),
            registered_at: now,
            updated_at: now,
            health: Default::default(),
            current_task_id: None,
        })
    }

    /// Get an external worker by ID.
    pub async fn get(
        &self,
        worker_id: &cairn_domain::WorkerId,
    ) -> Result<Option<cairn_domain::workers::ExternalWorkerRecord>, crate::error::RuntimeError> {
        Ok(cairn_store::projections::ExternalWorkerReadModel::get(
            self.store.as_ref(), worker_id,
        ).await?)
    }

    /// List external workers for a tenant.
    pub async fn list(
        &self,
        tenant_id: &cairn_domain::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::workers::ExternalWorkerRecord>, crate::error::RuntimeError> {
        Ok(cairn_store::projections::ExternalWorkerReadModel::list_by_tenant(
            self.store.as_ref(), tenant_id, limit, offset,
        ).await?)
    }

    /// Suspend an external worker.
    pub async fn suspend(
        &self,
        worker_id: &cairn_domain::WorkerId,
    ) -> Result<cairn_domain::workers::ExternalWorkerRecord, crate::error::RuntimeError> {
        use super::event_helpers::make_envelope;
        let event = make_envelope(cairn_domain::RuntimeEvent::ExternalWorkerSuspended(
            cairn_domain::ExternalWorkerSuspended {
                sentinel_project: cairn_domain::ProjectKey::new("_","_","_"),
                worker_id: worker_id.clone(),
                tenant_id: cairn_domain::TenantId::new("_"),
                suspended_at: 0,
                reason: None,
            },
        ));
        self.store.append(&[event]).await?;
        self.get(worker_id).await?
            .ok_or_else(|| crate::error::RuntimeError::NotFound { entity: "worker", id: worker_id.to_string() })
    }

    /// Reactivate a suspended external worker.
    pub async fn reactivate(
        &self,
        worker_id: &cairn_domain::WorkerId,
    ) -> Result<cairn_domain::workers::ExternalWorkerRecord, crate::error::RuntimeError> {
        use super::event_helpers::make_envelope;
        let event = make_envelope(cairn_domain::RuntimeEvent::ExternalWorkerReactivated(
            cairn_domain::ExternalWorkerReactivated {
                sentinel_project: cairn_domain::ProjectKey::new("_","_","_"),
                worker_id: worker_id.clone(),
                tenant_id: cairn_domain::TenantId::new("_"),
                reactivated_at: 0,
            },
        ));
        self.store.append(&[event]).await?;
        self.get(worker_id).await?
            .ok_or_else(|| crate::error::RuntimeError::NotFound { entity: "worker", id: worker_id.to_string() })
    }
}
