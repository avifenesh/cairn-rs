//! list_runs — query RunReadModel for run summaries.
//!
//! Gives the agent self-awareness: it can discover which runs exist in the
//! current project, filter by state, and see their key metadata before
//! deciding what to do next.
//!
//! ## Parameters
//! ```json
//! {
//!   "state_filter": "running",   // optional; any RunState snake_case value
//!   "limit":        20           // default 20, max 100
//! }
//! ```
//!
//! ## Output
//! ```json
//! {
//!   "runs": [
//!     { "run_id": "...", "state": "running", "session_id": "...", "created_at": 0 }
//!   ],
//!   "total": 1
//! }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey, RunState};
use cairn_store::projections::RunReadModel;
use serde_json::Value;

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

/// Run listing tool — wraps RunReadModel for agent self-awareness.
pub struct ListRunsTool {
    store: Arc<dyn RunReadModel>,
}

impl ListRunsTool {
    pub fn new(store: Arc<dyn RunReadModel>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ToolHandler for ListRunsTool {
    fn name(&self) -> &str {
        "list_runs"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }

    fn description(&self) -> &str {
        "List runs in the current project, optionally filtered by state."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "state_filter": {
                    "type": "string",
                    "enum": [
                        "pending", "running", "waiting_approval",
                        "paused", "waiting_dependency",
                        "completed", "failed", "canceled"
                    ],
                    "description": "Only return runs in this state. Omit for all active runs."
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "description": "Maximum number of runs to return (max 100)."
                }
            }
        })
    }

    // Read-only store access — no approval required.
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_LIMIT as u64)
            .min(MAX_LIMIT as u64) as usize;

        // Parse optional state filter.
        let state_filter: Option<RunState> = match args.get("state_filter").and_then(|v| v.as_str())
        {
            None => None,
            Some(s) => {
                let state = parse_run_state(s).ok_or_else(|| ToolError::InvalidArgs {
                    field: "state_filter".into(),
                    message: format!("unknown run state: '{s}'"),
                })?;
                Some(state)
            }
        };

        let records = match state_filter {
            Some(state) => RunReadModel::list_by_state(self.store.as_ref(), state, limit)
                .await
                .map_err(|e| ToolError::Transient(e.to_string()))?,
            None => {
                // No filter: list active runs for the project.
                RunReadModel::list_active_by_project(self.store.as_ref(), project, limit)
                    .await
                    .map_err(|e| ToolError::Transient(e.to_string()))?
            }
        };

        let runs: Vec<Value> = records
            .iter()
            .map(|r| {
                serde_json::json!({
                    "run_id":         r.run_id.as_str(),
                    "session_id":     r.session_id.as_str(),
                    "state":          format!("{:?}", r.state).to_lowercase(),
                    "agent_role":     r.agent_role_id,
                    "parent_run_id":  r.parent_run_id.as_ref().map(|id| id.as_str()),
                    "created_at":     r.created_at,
                    "updated_at":     r.updated_at,
                })
            })
            .collect();

        let total = runs.len();
        Ok(ToolResult::ok(serde_json::json!({
            "runs":  runs,
            "total": total,
        })))
    }
}

/// Parse a snake_case run state string.
fn parse_run_state(s: &str) -> Option<RunState> {
    match s {
        "pending" => Some(RunState::Pending),
        "running" => Some(RunState::Running),
        "waiting_approval" => Some(RunState::WaitingApproval),
        "paused" => Some(RunState::Paused),
        "waiting_dependency" => Some(RunState::WaitingDependency),
        "completed" => Some(RunState::Completed),
        "failed" => Some(RunState::Failed),
        "canceled" => Some(RunState::Canceled),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cairn_domain::{RunId, SessionId};
    use cairn_store::{error::StoreError, projections::RunRecord};
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    // ── Minimal stub ──────────────────────────────────────────────────────────

    struct StubStore {
        records: Vec<RunRecord>,
    }

    #[async_trait]
    impl RunReadModel for StubStore {
        async fn get(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError> {
            Ok(self.records.iter().find(|r| r.run_id == *id).cloned())
        }
        async fn list_by_session(
            &self,
            _: &SessionId,
            limit: usize,
            _: usize,
        ) -> Result<Vec<RunRecord>, StoreError> {
            Ok(self.records.iter().take(limit).cloned().collect())
        }
        async fn any_non_terminal(&self, _: &SessionId) -> Result<bool, StoreError> {
            Ok(!self.records.is_empty())
        }
        async fn latest_root_run(&self, _: &SessionId) -> Result<Option<RunRecord>, StoreError> {
            Ok(self.records.first().cloned())
        }
        async fn list_by_state(
            &self,
            state: RunState,
            limit: usize,
        ) -> Result<Vec<RunRecord>, StoreError> {
            Ok(self
                .records
                .iter()
                .filter(|r| r.state == state)
                .take(limit)
                .cloned()
                .collect())
        }
        async fn list_active_by_project(
            &self,
            _: &ProjectKey,
            limit: usize,
        ) -> Result<Vec<RunRecord>, StoreError> {
            Ok(self.records.iter().take(limit).cloned().collect())
        }
        async fn list_by_parent_run(
            &self,
            parent_run_id: &RunId,
            limit: usize,
        ) -> Result<Vec<RunRecord>, StoreError> {
            Ok(self
                .records
                .iter()
                .filter(|r| r.parent_run_id.as_ref() == Some(parent_run_id))
                .take(limit)
                .cloned()
                .collect())
        }
    }

    fn run_record(id: &str, state: RunState) -> RunRecord {
        RunRecord {
            run_id: RunId::new(id),
            session_id: SessionId::new("sess_1"),
            parent_run_id: None,
            project: project(),
            state,
            prompt_release_id: None,
            agent_role_id: None,
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
            version: 1,
            created_at: 1_000_000,
            updated_at: 1_000_001,
        }
    }

    fn make_tool(records: Vec<RunRecord>) -> ListRunsTool {
        ListRunsTool::new(Arc::new(StubStore { records }))
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn name_tier_class() {
        let t = make_tool(vec![]);
        assert_eq!(t.name(), "list_runs");
        assert_eq!(t.tier(), ToolTier::Registered);
        assert_eq!(t.execution_class(), ExecutionClass::SupervisedProcess);
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_state_is_invalid() {
        let err = make_tool(vec![])
            .execute(&project(), serde_json::json!({"state_filter": "zombie"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "state_filter"));
    }

    // ── Happy paths ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn empty_store_returns_empty_list() {
        let result = make_tool(vec![])
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 0);
        assert_eq!(result.output["runs"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn returns_all_active_runs_when_no_filter() {
        let records = vec![
            run_record("run_1", RunState::Running),
            run_record("run_2", RunState::Paused),
        ];
        let result = make_tool(records)
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 2);
        let runs = result.output["runs"].as_array().unwrap();
        let ids: Vec<&str> = runs.iter().map(|r| r["run_id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"run_1"));
        assert!(ids.contains(&"run_2"));
    }

    #[tokio::test]
    async fn state_filter_returns_matching_runs_only() {
        let records = vec![
            run_record("run_a", RunState::Running),
            run_record("run_b", RunState::Completed),
            run_record("run_c", RunState::Running),
        ];
        let result = make_tool(records)
            .execute(&project(), serde_json::json!({"state_filter": "running"}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 2);
        let runs = result.output["runs"].as_array().unwrap();
        assert!(runs.iter().all(|r| r["state"] == "running"));
    }

    #[tokio::test]
    async fn limit_is_respected() {
        let records = (0..50)
            .map(|i| run_record(&format!("run_{i:02}"), RunState::Running))
            .collect();
        let result = make_tool(records)
            .execute(&project(), serde_json::json!({"limit": 5}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 5);
    }

    #[tokio::test]
    async fn limit_capped_at_100() {
        let records = (0..150)
            .map(|i| run_record(&format!("run_{i:03}"), RunState::Running))
            .collect();
        let result = make_tool(records)
            .execute(&project(), serde_json::json!({"limit": 999}))
            .await
            .unwrap();
        // StubStore::list_active_by_project truncates to the capped limit
        assert!(result.output["total"].as_u64().unwrap() <= 100);
    }

    #[tokio::test]
    async fn run_fields_are_populated() {
        let result = make_tool(vec![run_record("run_x", RunState::Running)])
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap();
        let run = &result.output["runs"][0];
        assert_eq!(run["run_id"], "run_x");
        assert_eq!(run["state"], "running");
        assert_eq!(run["session_id"], "sess_1");
        assert_eq!(run["created_at"], 1_000_000u64);
    }

    #[test]
    fn parse_run_state_round_trips() {
        for (s, state) in [
            ("pending", RunState::Pending),
            ("running", RunState::Running),
            ("waiting_approval", RunState::WaitingApproval),
            ("paused", RunState::Paused),
            ("waiting_dependency", RunState::WaitingDependency),
            ("completed", RunState::Completed),
            ("failed", RunState::Failed),
            ("canceled", RunState::Canceled),
        ] {
            assert_eq!(parse_run_state(s), Some(state), "failed for '{s}'");
        }
        assert_eq!(parse_run_state("bogus"), None);
    }
}
