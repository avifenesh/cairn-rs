//! resolve_approval — agent-initiated approval resolution.
use super::{ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::{policy::ApprovalDecision, ApprovalId, ProjectKey};
use cairn_runtime::ApprovalService;
use serde_json::Value;
use std::sync::Arc;

pub struct ResolveApprovalTool {
    svc: Option<Arc<dyn ApprovalService>>,
}

impl ResolveApprovalTool {
    pub fn new(svc: Arc<dyn ApprovalService>) -> Self {
        Self { svc: Some(svc) }
    }
    pub fn stub() -> Self {
        Self { svc: None }
    }
}
impl Default for ResolveApprovalTool {
    fn default() -> Self {
        Self::stub()
    }
}

#[async_trait]
impl ToolHandler for ResolveApprovalTool {
    fn name(&self) -> &str {
        "resolve_approval"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Core
    }
    fn description(&self) -> &str {
        "Approve or reject a pending approval gate. \
         Use when you have the authority to unblock a waiting sub-agent or task. \
         Requires operator-level trust."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type":"object","required":["approval_id","decision"],
            "properties":{
                "approval_id":{"type":"string","description":"The approval ID to resolve"},
                "decision":{"type":"string","enum":["approved","rejected"],"description":"The decision"},
                "reason":{"type":"string","description":"Explanation for audit trail (recommended)"}
            }
        })
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let approval_id = args["approval_id"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "approval_id".into(),
                message: "required string".into(),
            })?
            .trim();
        if approval_id.is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "approval_id".into(),
                message: "must not be empty".into(),
            });
        }
        let decision_str = args["decision"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "decision".into(),
                message: "required: 'approved' or 'rejected'".into(),
            })?;
        let decision = match decision_str {
            "approved" => ApprovalDecision::Approved,
            "rejected" => ApprovalDecision::Rejected,
            other => {
                return Err(ToolError::InvalidArgs {
                    field: "decision".into(),
                    message: format!("must be 'approved' or 'rejected', got '{other}'"),
                })
            }
        };

        let svc = self
            .svc
            .as_ref()
            .ok_or_else(|| ToolError::Permanent("no approval service configured".into()))?;

        let record = svc
            .resolve(&ApprovalId::new(approval_id), decision)
            .await
            .map_err(|e| ToolError::Permanent(e.to_string()))?;

        Ok(ToolResult::ok(serde_json::json!({
            "resolved": true,
            "approval_id": approval_id,
            "decision": decision_str,
            "record_version": record.version,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[test]
    fn tier_is_core() {
        assert_eq!(ResolveApprovalTool::stub().tier(), ToolTier::Core);
    }
    #[test]
    fn schema_requires_approval_id_and_decision() {
        let s = ResolveApprovalTool::stub().parameters_schema();
        let req: Vec<&str> = s["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(req.contains(&"approval_id") && req.contains(&"decision"));
    }
    #[tokio::test]
    async fn missing_service_err() {
        let err = ResolveApprovalTool::stub()
            .execute(
                &p(),
                serde_json::json!({"approval_id":"a1","decision":"approved"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
    }
    #[tokio::test]
    async fn invalid_decision_err() {
        let err = ResolveApprovalTool::stub()
            .execute(
                &p(),
                serde_json::json!({"approval_id":"a1","decision":"maybe"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test]
    async fn missing_approval_id_err() {
        let err = ResolveApprovalTool::stub()
            .execute(&p(), serde_json::json!({"decision":"approved"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test]
    async fn empty_approval_id_err() {
        let err = ResolveApprovalTool::stub()
            .execute(
                &p(),
                serde_json::json!({"approval_id":"  ","decision":"approved"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
}
