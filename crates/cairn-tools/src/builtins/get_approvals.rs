//! get_approvals — list pending or resolved approvals for a project.
//!
//! ## Parameters
//! ```json
//! { "run_id": "run_abc", "limit": 20 }  // run_id optional
//! ```
//!
//! ## Output
//! ```json
//! {
//!   "approvals": [
//!     { "approval_id": "...", "run_id": "...", "requirement": "required",
//!       "decision": null, "pending": true }
//!   ],
//!   "total": 1
//! }
//! ```

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey, RunId};
use cairn_store::projections::ApprovalReadModel;
use serde_json::Value;
use std::sync::Arc;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

pub struct GetApprovalsTool {
    store: Arc<dyn ApprovalReadModel>,
}

impl GetApprovalsTool {
    pub fn new(store: Arc<dyn ApprovalReadModel>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ToolHandler for GetApprovalsTool {
    fn name(&self) -> &str {
        "get_approvals"
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
        "List pending approvals for the current project, optionally filtered by run."
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "Filter to approvals for this run." },
                "limit":  { "type": "integer", "default": 20, "description": "Max results (max 100)." }
            }
        })
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_LIMIT as u64)
            .min(MAX_LIMIT as u64) as usize;

        let records = ApprovalReadModel::list_pending(self.store.as_ref(), project, limit, 0)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        // Optional run_id filter applied in-process (list_pending is project-scoped).
        let run_filter: Option<RunId> = args.get("run_id").and_then(|v| v.as_str()).map(RunId::new);

        let approvals: Vec<Value> = records
            .iter()
            .filter(|a| {
                run_filter
                    .as_ref()
                    .is_none_or(|rid| a.run_id.as_ref() == Some(rid))
            })
            .map(|a| {
                serde_json::json!({
                    "approval_id": a.approval_id.as_str(),
                    "run_id":      a.run_id.as_ref().map(|id| id.as_str()),
                    "task_id":     a.task_id.as_ref().map(|id| id.as_str()),
                    "requirement": format!("{:?}", a.requirement).to_lowercase(),
                    "decision":    a.decision.as_ref().map(|d| format!("{:?}", d).to_lowercase()),
                    "title":       a.title,
                    "pending":     a.decision.is_none(),
                    "created_at":  a.created_at,
                })
            })
            .collect();

        let total = approvals.len();
        Ok(ToolResult::ok(
            serde_json::json!({ "approvals": approvals, "total": total }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_store::InMemoryStore;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[test]
    fn name_tier_class() {
        let t = GetApprovalsTool::new(Arc::new(InMemoryStore::new()));
        assert_eq!(t.name(), "get_approvals");
        assert_eq!(t.execution_class(), ExecutionClass::SupervisedProcess);
    }

    #[tokio::test]
    async fn empty_store_returns_empty_list() {
        let result = GetApprovalsTool::new(Arc::new(InMemoryStore::new()))
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 0);
    }

    #[tokio::test]
    async fn returns_pending_approvals() {
        use cairn_domain::{
            policy::ApprovalRequirement, ApprovalId, ApprovalRequested, EventEnvelope, EventId,
            EventSource, RunId, RuntimeEvent,
        };
        use cairn_store::{EventLog, InMemoryStore};
        let store = Arc::new(InMemoryStore::new());
        store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("e1"),
                EventSource::Runtime,
                RuntimeEvent::ApprovalRequested(ApprovalRequested {
                    project: project(),
                    approval_id: ApprovalId::new("appr_1"),
                    run_id: Some(RunId::new("run_1")),
                    task_id: None,
                    requirement: ApprovalRequirement::Required,
                }),
            )])
            .await
            .unwrap();
        let result = GetApprovalsTool::new(store)
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 1);
        assert_eq!(result.output["approvals"][0]["approval_id"], "appr_1");
        assert_eq!(result.output["approvals"][0]["pending"], true);
    }

    #[tokio::test]
    async fn run_id_filter_works() {
        use cairn_domain::{
            policy::ApprovalRequirement, ApprovalId, ApprovalRequested, EventEnvelope, EventId,
            EventSource, RunId, RuntimeEvent,
        };
        use cairn_store::{EventLog, InMemoryStore};
        let store = Arc::new(InMemoryStore::new());
        for (aid, rid) in [("a1", "run_a"), ("a2", "run_b")] {
            store
                .append(&[EventEnvelope::for_runtime_event(
                    EventId::new(format!("e_{aid}")),
                    EventSource::Runtime,
                    RuntimeEvent::ApprovalRequested(ApprovalRequested {
                        project: project(),
                        approval_id: ApprovalId::new(aid),
                        run_id: Some(RunId::new(rid)),
                        task_id: None,
                        requirement: ApprovalRequirement::Required,
                    }),
                )])
                .await
                .unwrap();
        }
        let result = GetApprovalsTool::new(store)
            .execute(&project(), serde_json::json!({"run_id":"run_a"}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 1);
        assert_eq!(result.output["approvals"][0]["approval_id"], "a1");
    }
}
