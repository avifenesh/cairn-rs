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

    /// Remove the entry and return whether it existed.
    /// Unlike take(), returns true even for handles created via
    /// new_without_claimed_task (API-claimed tasks with no local ClaimedTask).
    pub fn remove_entry(&self, task_id: &TaskId) -> bool {
        self.tasks.remove(&task_id.to_string()).is_some()
    }

    pub fn take_with_context(
        &self,
        task_id: &TaskId,
    ) -> Option<(Option<ClaimedTask>, LeaseId, LeaseEpoch, AttemptIndex)> {
        self.tasks.remove(&task_id.to_string()).map(|(_, mut h)| {
            (
                h.claimed_task.take(),
                h.lease_id,
                h.lease_epoch,
                h.attempt_index,
            )
        })
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

    #[test]
    fn take_with_context_nonexistent_returns_none() {
        let registry = ActiveTaskRegistry::new();
        assert!(registry
            .take_with_context(&TaskId::new("nonexistent"))
            .is_none());
    }

    #[test]
    fn take_with_context_removes_and_returns_context() {
        let registry = ActiveTaskRegistry::new();
        let eid = ExecutionId::from_uuid(uuid::Uuid::nil());
        let lid = LeaseId::new();
        let epoch = LeaseEpoch::new(3);
        let att = AttemptIndex::new(1);
        let handle = ActiveTaskHandle::new_without_claimed_task(eid, lid.clone(), epoch, att);
        let tid = TaskId::new("t1");
        registry.register(&tid, handle);
        assert_eq!(registry.len(), 1);

        let (_, lease_id, lease_epoch, att_idx) =
            registry.take_with_context(&tid).expect("should exist");
        assert_eq!(lease_id, lid);
        assert_eq!(lease_epoch.0, 3);
        assert_eq!(att_idx.0, 1);
        assert!(registry.is_empty());
    }
}
