use dashmap::DashMap;
use ff_core::types::{AttemptIndex, ExecutionId, LeaseEpoch, LeaseId};
use ff_sdk::ClaimedTask;

use cairn_domain::TaskId;

pub struct ActiveTaskHandle {
    claimed_task: Option<ClaimedTask>,
    pub execution_id: ExecutionId,
    pub lease_id: LeaseId,
    pub lease_epoch: LeaseEpoch,
    pub attempt_index: AttemptIndex,
}

impl ActiveTaskHandle {
    pub fn new(
        claimed_task: ClaimedTask,
        execution_id: ExecutionId,
        lease_id: LeaseId,
        lease_epoch: LeaseEpoch,
        attempt_index: AttemptIndex,
    ) -> Self {
        Self {
            claimed_task: Some(claimed_task),
            execution_id,
            lease_id,
            lease_epoch,
            attempt_index,
        }
    }

    pub fn new_without_claimed_task(
        execution_id: ExecutionId,
        lease_id: LeaseId,
        lease_epoch: LeaseEpoch,
        attempt_index: AttemptIndex,
    ) -> Self {
        Self {
            claimed_task: None,
            execution_id,
            lease_id,
            lease_epoch,
            attempt_index,
        }
    }
}

pub struct ActiveTaskRegistry {
    tasks: DashMap<String, ActiveTaskHandle>,
}

impl Default for ActiveTaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveTaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: DashMap::new(),
        }
    }

    pub fn register(&self, task_id: &TaskId, handle: ActiveTaskHandle) {
        self.tasks.insert(task_id.to_string(), handle);
    }

    pub fn take(&self, task_id: &TaskId) -> Option<ClaimedTask> {
        self.tasks
            .remove(&task_id.to_string())
            .and_then(|(_, mut handle)| handle.claimed_task.take())
    }

    pub fn get_lease_context(
        &self,
        task_id: &TaskId,
    ) -> Option<(LeaseId, LeaseEpoch, AttemptIndex)> {
        self.tasks.get(&task_id.to_string()).map(|handle| {
            (
                handle.lease_id.clone(),
                handle.lease_epoch,
                handle.attempt_index,
            )
        })
    }

    pub fn get_execution_id(&self, task_id: &TaskId) -> Option<ExecutionId> {
        self.tasks
            .get(&task_id.to_string())
            .map(|handle| handle.execution_id.clone())
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        let registry = ActiveTaskRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn take_nonexistent_returns_none() {
        let registry = ActiveTaskRegistry::new();
        assert!(registry.take(&TaskId::new("nonexistent")).is_none());
    }

    #[test]
    fn get_lease_context_nonexistent_returns_none() {
        let registry = ActiveTaskRegistry::new();
        assert!(registry
            .get_lease_context(&TaskId::new("nonexistent"))
            .is_none());
    }

    #[test]
    fn get_execution_id_nonexistent_returns_none() {
        let registry = ActiveTaskRegistry::new();
        assert!(registry
            .get_execution_id(&TaskId::new("nonexistent"))
            .is_none());
    }
}
