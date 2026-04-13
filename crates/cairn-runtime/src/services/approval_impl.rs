use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{ApprovalReadModel, ApprovalRecord, RunReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::approvals::ApprovalService;
use crate::error::RuntimeError;

pub struct ApprovalServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ApprovalServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> ApprovalService for ApprovalServiceImpl<S>
where
    S: EventLog + ApprovalReadModel + RunReadModel + 'static,
{
    async fn request(
        &self,
        project: &ProjectKey,
        approval_id: ApprovalId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        requirement: ApprovalRequirement,
    ) -> Result<ApprovalRecord, RuntimeError> {
        self.request_with_context(
            project,
            approval_id,
            run_id,
            task_id,
            requirement,
            None,
            None,
        )
        .await
    }

    async fn request_with_context(
        &self,
        project: &ProjectKey,
        approval_id: ApprovalId,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        requirement: ApprovalRequirement,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<ApprovalRecord, RuntimeError> {
        let saved_run_id = run_id.clone();
        let event = make_envelope(RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project.clone(),
            approval_id: approval_id.clone(),
            run_id,
            task_id,
            requirement,
            title,
            description,
        }));

        self.store.append(&[event]).await?;

        // Transition the associated run to WaitingApproval.
        if let Some(ref rid) = saved_run_id {
            if let Some(run) = RunReadModel::get(self.store.as_ref(), rid).await? {
                if can_transition_run_state(run.state, RunState::WaitingApproval) {
                    let transition_event =
                        make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                            project: project.clone(),
                            run_id: rid.clone(),
                            transition: StateTransition {
                                from: Some(run.state),
                                to: RunState::WaitingApproval,
                            },
                            failure_class: None,
                            pause_reason: None,
                            resume_trigger: None,
                        }));
                    self.store.append(&[transition_event]).await?;
                }
            }
        }

        ApprovalReadModel::get(self.store.as_ref(), &approval_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("approval not found after request".into()))
    }

    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, RuntimeError> {
        Ok(ApprovalReadModel::get(self.store.as_ref(), approval_id).await?)
    }

    async fn resolve(
        &self,
        approval_id: &ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<ApprovalRecord, RuntimeError> {
        let approval = ApprovalReadModel::get(self.store.as_ref(), approval_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "approval",
                id: approval_id.to_string(),
            })?;

        if approval.decision.is_some() {
            return Err(RuntimeError::InvalidTransition {
                entity: "approval",
                from: "resolved".into(),
                to: format!("{decision:?}"),
            });
        }

        let event = make_envelope(RuntimeEvent::ApprovalResolved(ApprovalResolved {
            project: approval.project.clone(),
            approval_id: approval_id.clone(),
            decision,
        }));

        self.store.append(&[event]).await?;

        // Cascade the decision to the associated run's state machine.
        if let Some(ref run_id) = approval.run_id {
            if let Some(run) = RunReadModel::get(self.store.as_ref(), run_id).await? {
                let (to_state, failure_class, resume_trigger) = match decision {
                    ApprovalDecision::Approved => {
                        (RunState::Running, None, Some(ResumeTrigger::OperatorResume))
                    }
                    ApprovalDecision::Rejected => {
                        (RunState::Failed, Some(FailureClass::ApprovalRejected), None)
                    }
                };
                if can_transition_run_state(run.state, to_state) {
                    let transition_event =
                        make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                            project: run.project.clone(),
                            run_id: run_id.clone(),
                            transition: StateTransition {
                                from: Some(run.state),
                                to: to_state,
                            },
                            failure_class,
                            pause_reason: None,
                            resume_trigger,
                        }));
                    self.store.append(&[transition_event]).await?;
                }
            }
        }

        ApprovalReadModel::get(self.store.as_ref(), approval_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("approval not found after resolve".into()))
    }

    async fn list_pending(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, RuntimeError> {
        Ok(self.store.list_pending(project, limit, offset).await?)
    }

    async fn list_all(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, RuntimeError> {
        Ok(self.store.list_all(project, limit, offset).await?)
    }
}
