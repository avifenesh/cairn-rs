use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{TaskDependencyReadModel, TaskDependencyRecord, TaskReadModel, TaskRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::tasks::TaskService;

pub struct TaskServiceImpl<S> {
    store: Arc<S>,
}

impl<S> TaskServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

impl<S: EventLog + TaskReadModel + TaskDependencyReadModel + 'static> TaskServiceImpl<S> {
    async fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        TaskReadModel::get(self.store.as_ref(), task_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "task",
                id: task_id.to_string(),
            })
    }

    async fn transition_task(
        &self,
        task_id: &TaskId,
        to: TaskState,
        failure_class: Option<FailureClass>,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if !can_transition_task_state(task.state, to) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: format!("{to:?}"),
            });
        }

        let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: task.project.clone(),
            task_id: task_id.clone(),
            transition: StateTransition {
                from: Some(task.state),
                to,
            },
            failure_class,
            pause_reason: None,
            resume_trigger: None,
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }
}

#[async_trait]
impl<S> TaskService for TaskServiceImpl<S>
where
    S: EventLog + TaskReadModel + TaskDependencyReadModel + 'static,
{
    async fn submit(
        &self,
        project: &ProjectKey,
        task_id: TaskId,
        parent_run_id: Option<RunId>,
        parent_task_id: Option<TaskId>,
        _priority: u32,
    ) -> Result<TaskRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
            project: project.clone(),
            task_id: task_id.clone(),
            parent_run_id,
            parent_task_id,
            prompt_release_id: None,
        }));

        self.store.append(&[event]).await?;
        self.get_task(&task_id).await
    }

    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, RuntimeError> {
        Ok(TaskReadModel::get(self.store.as_ref(), task_id).await?)
    }

    async fn claim(
        &self,
        task_id: &TaskId,
        lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if !can_transition_task_state(task.state, TaskState::Leased) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: "Leased".into(),
            });
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let events = vec![
            make_envelope(RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                project: task.project.clone(),
                task_id: task_id.clone(),
                lease_owner,
                lease_token: task.version + 1,
                lease_expires_at_ms: now_ms + lease_duration_ms,
            })),
            make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: task.project.clone(),
                task_id: task_id.clone(),
                transition: StateTransition {
                    from: Some(TaskState::Queued),
                    to: TaskState::Leased,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            })),
        ];

        self.store.append(&events).await?;
        self.get_task(task_id).await
    }

    async fn heartbeat(
        &self,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if task.state != TaskState::Leased && task.state != TaskState::Running {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: "heartbeat".into(),
            });
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let event = make_envelope(RuntimeEvent::TaskLeaseHeartbeated(TaskLeaseHeartbeated {
            project: task.project.clone(),
            task_id: task_id.clone(),
            lease_token: task.version,
            lease_expires_at_ms: now_ms + lease_extension_ms,
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }

    async fn start(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        self.transition_task(task_id, TaskState::Running, None)
            .await
    }

    async fn complete(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        let result = self.transition_task(task_id, TaskState::Completed, None).await?;
        // Mark all dependencies with this task as prerequisite as resolved.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let _ = TaskDependencyReadModel::resolve_dependency(
            self.store.as_ref(),
            task_id,
            now_ms,
        ).await;
        Ok(result)
    }

    async fn declare_dependency(
        &self,
        dependent_task_id: &TaskId,
        prerequisite_task_id: &TaskId,
    ) -> Result<TaskDependencyRecord, RuntimeError> {
        // Look up the dependent task to get its project.
        let task = self.get_task(dependent_task_id).await?;

        // Transition dependent task to WaitingDependency.
        if can_transition_task_state(task.state, TaskState::WaitingDependency) {
            let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: task.project.clone(),
                task_id: dependent_task_id.clone(),
                transition: StateTransition {
                    from: Some(task.state),
                    to: TaskState::WaitingDependency,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }));
            self.store.append(&[event]).await?;
        }

        // Store the dependency record.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let dep = cairn_domain::TaskDependency {
            dependent_task_id: dependent_task_id.clone(),
            depends_on_task_id: prerequisite_task_id.clone(),
            project: task.project.clone(),
            created_at_ms: now_ms,
        };
        let record = TaskDependencyRecord {
            dependency: dep,
            resolved_at_ms: None,
        };
        TaskDependencyReadModel::insert_dependency(
            self.store.as_ref(),
            record.clone(),
        ).await.map_err(RuntimeError::Store)?;
        Ok(record)
    }

    async fn check_dependencies(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, RuntimeError> {
        let deps = TaskDependencyReadModel::list_blocking(self.store.as_ref(), task_id)
            .await
            .map_err(RuntimeError::Store)?;
        let unresolved: Vec<TaskDependencyRecord> = deps
            .into_iter()
            .filter(|d| d.resolved_at_ms.is_none())
            .collect();
        // If all resolved, transition to Queued.
        if unresolved.is_empty() {
            if let Ok(task) = self.get_task(task_id).await {
                if can_transition_task_state(task.state, TaskState::Queued) {
                    let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                        project: task.project.clone(),
                        task_id: task_id.clone(),
                        transition: StateTransition {
                            from: Some(task.state),
                            to: TaskState::Queued,
                        },
                        failure_class: None,
                        pause_reason: None,
                        resume_trigger: None,
                    }));
                    let _ = self.store.append(&[event]).await;
                }
            }
        }
        Ok(unresolved)
    }

    async fn fail(
        &self,
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, RuntimeError> {
        // Choose retryable vs terminal based on failure class
        let target = match failure_class {
            FailureClass::LeaseExpired | FailureClass::DependencyFailed => {
                TaskState::RetryableFailed
            }
            _ => TaskState::Failed,
        };
        self.transition_task(task_id, target, Some(failure_class))
            .await
    }

    async fn cancel(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        self.transition_task(task_id, TaskState::Canceled, None)
            .await
    }

    async fn dead_letter(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        self.transition_task(task_id, TaskState::DeadLettered, None)
            .await
    }

    /// RFC 005: return all dead-lettered tasks for a project (the dead-letter queue).
    async fn list_dead_lettered(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(self
            .store
            .list_by_state(project, TaskState::DeadLettered, limit)
            .await?
            .into_iter()
            .skip(offset)
            .collect())
    }

    async fn pause(
        &self,
        task_id: &TaskId,
        reason: PauseReason,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if !can_transition_task_state(task.state, TaskState::Paused) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: format!("{:?}", TaskState::Paused),
            });
        }

        let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: task.project.clone(),
            task_id: task_id.clone(),
            transition: StateTransition {
                from: Some(task.state),
                to: TaskState::Paused,
            },
            failure_class: None,
            pause_reason: Some(reason),
            resume_trigger: None,
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }

    async fn resume(
        &self,
        task_id: &TaskId,
        trigger: ResumeTrigger,
        target: TaskResumeTarget,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;
        let to = match target {
            TaskResumeTarget::Queued => TaskState::Queued,
            TaskResumeTarget::Running => TaskState::Running,
        };

        if !can_transition_task_state(task.state, to) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: format!("{to:?}"),
            });
        }

        let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: task.project.clone(),
            task_id: task_id.clone(),
            transition: StateTransition {
                from: Some(task.state),
                to,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: Some(trigger),
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }

    async fn list_by_state(
        &self,
        project: &ProjectKey,
        state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(self.store.list_by_state(project, state, limit).await?)
    }

    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(self.store.list_expired_leases(now, limit).await?)
    }
}
