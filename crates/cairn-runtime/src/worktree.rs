//! Worktree divergence monitor (GAP-018).
//!
//! Tracks per-task git worktrees and detects divergence from the base branch.
//!
//! Mirrors `cairn/internal/worktree/` — per-task git worktree isolation +
//! divergence detection. Each agent task gets its own worktree so concurrent
//! runs don't share file state. The divergence monitor detects when a worktree
//! has modified, committed, or conflicted changes relative to the base branch.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Status of a single worktree relative to its base branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeStatus {
    /// Worktree is in sync with the base branch — no uncommitted or ahead commits.
    Clean,
    /// Worktree has uncommitted local changes (staged or unstaged).
    Dirty { modified_files: u32 },
    /// Worktree has committed changes ahead of the base branch.
    Diverged { commits_ahead: u32 },
    /// Worktree has merge conflicts that must be resolved before merge.
    Conflicted { conflicted_files: u32 },
}

impl WorktreeStatus {
    /// Returns true if the worktree requires operator attention before merge.
    pub fn needs_attention(&self) -> bool {
        matches!(
            self,
            WorktreeStatus::Diverged { .. } | WorktreeStatus::Conflicted { .. }
        )
    }
}

/// A per-task git worktree record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRecord {
    /// Unique worktree ID (e.g. `wt_<task_id>`).
    pub worktree_id: String,
    /// Task ID this worktree belongs to.
    pub task_id: String,
    /// Base branch the worktree was forked from (e.g. `main`).
    pub base_branch: String,
    /// Isolated branch for this worktree (e.g. `task/run_xyz`).
    pub worktree_branch: String,
    /// Filesystem path of the worktree checkout.
    pub path: String,
    /// Current divergence status.
    pub status: WorktreeStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Summary of divergence state across all registered worktrees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceSummary {
    pub total: usize,
    pub clean: usize,
    pub dirty: usize,
    pub diverged: usize,
    pub conflicted: usize,
}

/// In-memory worktree registry.
///
/// Thread-safety: wrap in `Arc<Mutex<WorktreeRegistry>>` or use
/// `WorktreeServiceImpl` which manages the lock internally.
#[derive(Debug, Default)]
pub struct WorktreeRegistry {
    records: HashMap<String, WorktreeRecord>,
}

impl WorktreeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a worktree, overwriting any existing entry with the same ID.
    pub fn register(&mut self, record: WorktreeRecord) {
        self.records.insert(record.worktree_id.clone(), record);
    }

    /// Update the status of an existing worktree.
    ///
    /// Returns `true` if the worktree was found and updated, `false` otherwise.
    pub fn update_status(&mut self, worktree_id: &str, status: WorktreeStatus) -> bool {
        if let Some(rec) = self.records.get_mut(worktree_id) {
            rec.status = status;
            true
        } else {
            false
        }
    }

    /// Look up a worktree by ID.
    pub fn get(&self, worktree_id: &str) -> Option<&WorktreeRecord> {
        self.records.get(worktree_id)
    }

    /// List all worktrees for a specific task, sorted by `created_at_ms`.
    pub fn list_by_task(&self, task_id: &str) -> Vec<&WorktreeRecord> {
        let mut v: Vec<_> = self
            .records
            .values()
            .filter(|r| r.task_id == task_id)
            .collect();
        v.sort_by_key(|r| r.created_at_ms);
        v
    }

    /// List worktrees in `Diverged` or `Conflicted` status, sorted by ID.
    pub fn list_diverged(&self) -> Vec<&WorktreeRecord> {
        let mut v: Vec<_> = self
            .records
            .values()
            .filter(|r| r.status.needs_attention())
            .collect();
        v.sort_by_key(|r| r.worktree_id.clone());
        v
    }

    /// Remove a worktree record and return it.
    pub fn remove(&mut self, worktree_id: &str) -> Option<WorktreeRecord> {
        self.records.remove(worktree_id)
    }

    /// Aggregate divergence counts across all registered worktrees.
    pub fn divergence_summary(&self) -> DivergenceSummary {
        let mut summary = DivergenceSummary {
            total: self.records.len(),
            clean: 0,
            dirty: 0,
            diverged: 0,
            conflicted: 0,
        };
        for rec in self.records.values() {
            match &rec.status {
                WorktreeStatus::Clean => summary.clean += 1,
                WorktreeStatus::Dirty { .. } => summary.dirty += 1,
                WorktreeStatus::Diverged { .. } => summary.diverged += 1,
                WorktreeStatus::Conflicted { .. } => summary.conflicted += 1,
            }
        }
        summary
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Service trait for worktree lifecycle management.
#[async_trait::async_trait]
pub trait WorktreeService: Send + Sync {
    async fn register_worktree(
        &self,
        record: WorktreeRecord,
    ) -> Result<(), crate::error::RuntimeError>;

    async fn update_status(
        &self,
        worktree_id: &str,
        status: WorktreeStatus,
    ) -> Result<(), crate::error::RuntimeError>;

    async fn get_diverged(&self) -> Result<Vec<WorktreeRecord>, crate::error::RuntimeError>;

    async fn summary(&self) -> Result<DivergenceSummary, crate::error::RuntimeError>;
}

/// In-process worktree service backed by a `Mutex<WorktreeRegistry>`.
pub struct WorktreeServiceImpl {
    registry: Mutex<WorktreeRegistry>,
}

impl WorktreeServiceImpl {
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(WorktreeRegistry::new()),
        }
    }
}

impl Default for WorktreeServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl WorktreeService for WorktreeServiceImpl {
    async fn register_worktree(
        &self,
        record: WorktreeRecord,
    ) -> Result<(), crate::error::RuntimeError> {
        self.registry.lock().unwrap().register(record);
        Ok(())
    }

    async fn update_status(
        &self,
        worktree_id: &str,
        status: WorktreeStatus,
    ) -> Result<(), crate::error::RuntimeError> {
        let updated = self
            .registry
            .lock()
            .unwrap()
            .update_status(worktree_id, status);
        if updated {
            Ok(())
        } else {
            Err(crate::error::RuntimeError::NotFound {
                entity: "worktree",
                id: worktree_id.to_owned(),
            })
        }
    }

    async fn get_diverged(&self) -> Result<Vec<WorktreeRecord>, crate::error::RuntimeError> {
        Ok(self
            .registry
            .lock()
            .unwrap()
            .list_diverged()
            .into_iter()
            .cloned()
            .collect())
    }

    async fn summary(&self) -> Result<DivergenceSummary, crate::error::RuntimeError> {
        Ok(self.registry.lock().unwrap().divergence_summary())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[cfg(test)]
fn make_record(
    id: &str,
    task_id: &str,
    status: WorktreeStatus,
    created_at_ms: u64,
) -> WorktreeRecord {
    WorktreeRecord {
        worktree_id: id.to_owned(),
        task_id: task_id.to_owned(),
        base_branch: "main".to_owned(),
        worktree_branch: format!("task/{id}"),
        path: format!("/tmp/worktrees/{id}"),
        status,
        created_at_ms,
        updated_at_ms: created_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_retrieve_worktree() {
        let mut reg = WorktreeRegistry::new();
        let rec = make_record("wt_1", "task_1", WorktreeStatus::Clean, 1000);
        reg.register(rec);

        let found = reg.get("wt_1").expect("worktree must be registered");
        assert_eq!(found.worktree_id, "wt_1");
        assert_eq!(found.task_id, "task_1");
        assert_eq!(found.base_branch, "main");
        assert_eq!(found.status, WorktreeStatus::Clean);
        assert!(reg.get("nonexistent").is_none());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn update_status_changes_status() {
        let mut reg = WorktreeRegistry::new();
        reg.register(make_record("wt_2", "t", WorktreeStatus::Clean, 1000));

        let updated = reg.update_status("wt_2", WorktreeStatus::Dirty { modified_files: 3 });
        assert!(updated, "update must succeed for existing worktree");

        let rec = reg.get("wt_2").unwrap();
        assert_eq!(rec.status, WorktreeStatus::Dirty { modified_files: 3 });
    }

    #[test]
    fn update_status_returns_false_for_unknown() {
        let mut reg = WorktreeRegistry::new();
        assert!(!reg.update_status("ghost", WorktreeStatus::Clean));
    }

    #[test]
    fn list_diverged_returns_only_diverged() {
        let mut reg = WorktreeRegistry::new();
        reg.register(make_record("wt_clean", "t", WorktreeStatus::Clean, 1000));
        reg.register(make_record(
            "wt_dirty",
            "t",
            WorktreeStatus::Dirty { modified_files: 1 },
            2000,
        ));
        reg.register(make_record(
            "wt_div",
            "t",
            WorktreeStatus::Diverged { commits_ahead: 2 },
            3000,
        ));
        reg.register(make_record(
            "wt_conf",
            "t",
            WorktreeStatus::Conflicted {
                conflicted_files: 1,
            },
            4000,
        ));

        let diverged = reg.list_diverged();
        assert_eq!(
            diverged.len(),
            2,
            "only Diverged and Conflicted need attention"
        );
        let ids: Vec<&str> = diverged.iter().map(|r| r.worktree_id.as_str()).collect();
        assert!(ids.contains(&"wt_div"));
        assert!(ids.contains(&"wt_conf"));
    }

    #[test]
    fn list_by_task_filters_correctly() {
        let mut reg = WorktreeRegistry::new();
        reg.register(make_record("wt_a1", "task_a", WorktreeStatus::Clean, 1000));
        reg.register(make_record(
            "wt_a2",
            "task_a",
            WorktreeStatus::Dirty { modified_files: 2 },
            2000,
        ));
        reg.register(make_record("wt_b1", "task_b", WorktreeStatus::Clean, 3000));

        let task_a = reg.list_by_task("task_a");
        assert_eq!(task_a.len(), 2);
        assert!(task_a.iter().all(|r| r.task_id == "task_a"));

        let task_b = reg.list_by_task("task_b");
        assert_eq!(task_b.len(), 1);
        assert_eq!(task_b[0].worktree_id, "wt_b1");

        let task_c = reg.list_by_task("task_c");
        assert!(task_c.is_empty());
    }

    #[test]
    fn remove_returns_record() {
        let mut reg = WorktreeRegistry::new();
        reg.register(make_record("wt_rem", "t", WorktreeStatus::Clean, 1000));

        let removed = reg.remove("wt_rem");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().worktree_id, "wt_rem");
        assert!(reg.is_empty());
        assert!(
            reg.remove("wt_rem").is_none(),
            "second remove must return None"
        );
    }

    #[test]
    fn divergence_summary_counts_correctly() {
        let mut reg = WorktreeRegistry::new();
        reg.register(make_record("w1", "t", WorktreeStatus::Clean, 1));
        reg.register(make_record("w2", "t", WorktreeStatus::Clean, 2));
        reg.register(make_record(
            "w3",
            "t",
            WorktreeStatus::Dirty { modified_files: 5 },
            3,
        ));
        reg.register(make_record(
            "w4",
            "t",
            WorktreeStatus::Diverged { commits_ahead: 1 },
            4,
        ));
        reg.register(make_record(
            "w5",
            "t",
            WorktreeStatus::Conflicted {
                conflicted_files: 2,
            },
            5,
        ));

        let s = reg.divergence_summary();
        assert_eq!(s.total, 5);
        assert_eq!(s.clean, 2);
        assert_eq!(s.dirty, 1);
        assert_eq!(s.diverged, 1);
        assert_eq!(s.conflicted, 1);
    }

    #[test]
    fn dirty_status_carries_modified_file_count() {
        let status = WorktreeStatus::Dirty { modified_files: 7 };
        assert!(
            !status.needs_attention(),
            "Dirty does not require merge approval"
        );
        if let WorktreeStatus::Dirty { modified_files } = status {
            assert_eq!(modified_files, 7);
        } else {
            panic!("expected Dirty");
        }
    }

    #[test]
    fn conflicted_status_carries_conflicted_file_count() {
        let status = WorktreeStatus::Conflicted {
            conflicted_files: 3,
        };
        assert!(
            status.needs_attention(),
            "Conflicted must require attention"
        );
        if let WorktreeStatus::Conflicted { conflicted_files } = status {
            assert_eq!(conflicted_files, 3);
        } else {
            panic!("expected Conflicted");
        }
    }

    #[test]
    fn diverged_status_needs_attention() {
        let status = WorktreeStatus::Diverged { commits_ahead: 4 };
        assert!(status.needs_attention());
        if let WorktreeStatus::Diverged { commits_ahead } = status {
            assert_eq!(commits_ahead, 4);
        }
    }

    #[test]
    fn clean_status_does_not_need_attention() {
        assert!(!WorktreeStatus::Clean.needs_attention());
    }

    #[tokio::test]
    async fn service_register_and_summary() {
        let svc = WorktreeServiceImpl::new();

        svc.register_worktree(make_record("w1", "t1", WorktreeStatus::Clean, 1000))
            .await
            .unwrap();
        svc.register_worktree(make_record(
            "w2",
            "t2",
            WorktreeStatus::Diverged { commits_ahead: 3 },
            2000,
        ))
        .await
        .unwrap();

        let summary = svc.summary().await.unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.clean, 1);
        assert_eq!(summary.diverged, 1);

        let diverged = svc.get_diverged().await.unwrap();
        assert_eq!(diverged.len(), 1);
        assert_eq!(diverged[0].worktree_id, "w2");
    }

    #[tokio::test]
    async fn service_update_status_returns_not_found_for_unknown() {
        let svc = WorktreeServiceImpl::new();
        let err = svc
            .update_status("ghost", WorktreeStatus::Clean)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::error::RuntimeError::NotFound { .. }));
    }
}
