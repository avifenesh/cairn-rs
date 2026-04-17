//! `get_run` built-in tool — inspect a specific run by ID.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey, RunId};
use cairn_store::projections::RunReadModel;
use serde_json::Value;

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

/// Read the current state of a run.
pub struct GetRunTool {
    store: Arc<dyn RunReadModel>,
}

impl GetRunTool {
    pub fn new(store: Arc<dyn RunReadModel>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ToolHandler for GetRunTool {
    fn name(&self) -> &str {
        "get_run"
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
        "Inspect a specific run by its ID. \
         Returns the run state, session linkage, and timestamps."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["run_id"],
            "properties": {
                "run_id": { "type": "string", "description": "The run ID to look up" }
            }
        })
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SandboxedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let run_id =
            args.get("run_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "run_id".into(),
                    message: "required string".into(),
                })?;

        match RunReadModel::get(self.store.as_ref(), &RunId::new(run_id)).await {
            Ok(Some(run)) => Ok(ToolResult::ok(serde_json::json!({
                "run_id":     run.run_id.as_str(),
                "session_id": run.session_id.as_str(),
                "state":      format!("{:?}", run.state).to_lowercase(),
                "is_terminal": run.state.is_terminal(),
                "created_at":  run.created_at,
                "updated_at":  run.updated_at,
                "project": {
                    "tenant_id":    run.project.tenant_id.as_str(),
                    "workspace_id": run.project.workspace_id.as_str(),
                    "project_id":   run.project.project_id.as_str(),
                },
            }))),
            Ok(None) => Err(ToolError::Permanent(format!("run not found: {run_id}"))),
            Err(e) => Err(ToolError::Transient(format!("store error: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{ProjectKey, RunId, SessionId};
    use cairn_runtime::InMemoryServices;
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    async fn svc() -> Arc<InMemoryServices> {
        Arc::new(InMemoryServices::new())
    }

    #[tokio::test]
    async fn returns_run_state() {
        let svc = svc().await;
        svc.sessions
            .create(&project(), SessionId::new("sess_gr"))
            .await
            .unwrap();
        svc.runs
            .start(
                &project(),
                &SessionId::new("sess_gr"),
                RunId::new("run_gr"),
                None,
            )
            .await
            .unwrap();

        let tool = GetRunTool::new(svc.store.clone());
        let res = tool
            .execute(&project(), serde_json::json!({ "run_id": "run_gr" }))
            .await
            .unwrap();
        assert_eq!(res.output["run_id"], "run_gr");
        assert_eq!(res.output["state"], "pending");
        assert_eq!(res.output["is_terminal"], false);
    }

    #[tokio::test]
    async fn not_found_is_permanent_error() {
        let svc = svc().await;
        let tool = GetRunTool::new(svc.store.clone());
        let err = tool
            .execute(&project(), serde_json::json!({ "run_id": "nope" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
    }

    #[test]
    fn tier_is_registered() {
        assert_eq!(
            GetRunTool::new(Arc::new(cairn_store::InMemoryStore::new())).tier(),
            ToolTier::Registered
        );
    }
}
