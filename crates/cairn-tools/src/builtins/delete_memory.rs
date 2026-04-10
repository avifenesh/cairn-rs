//! delete_memory — stub (concrete impl in cairn-memory to avoid dep cycle).
use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde_json::Value;
use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

pub struct DeleteMemoryTool;
impl Default for DeleteMemoryTool { fn default() -> Self { Self } }

#[async_trait]
impl ToolHandler for DeleteMemoryTool {
    fn name(&self) -> &str { "delete_memory" }
    fn tier(&self) -> ToolTier { ToolTier::Registered }
    fn tool_effect(&self) -> ToolEffect { ToolEffect::Internal }
    fn retry_safety(&self) -> RetrySafety { RetrySafety::AuthorResponsible }
    fn description(&self) -> &str { "Delete a document from the agent memory store." }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","required":["document_id"],"properties":{
            "document_id":{"type":"string","description":"Document ID to delete"}
        }})
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let id = args["document_id"].as_str()
            .ok_or_else(|| ToolError::InvalidArgs { field:"document_id".into(), message:"required".into() })?;
        Ok(ToolResult::ok(serde_json::json!({ "deleted": true, "document_id": id,
            "note": "delete_memory stub — wire concrete impl from cairn-memory" })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p() -> ProjectKey { ProjectKey::new("t","w","p") }
    #[tokio::test] async fn stub_deletes() {
        let r = DeleteMemoryTool.execute(&p(), serde_json::json!({"document_id":"doc1"})).await.unwrap();
        assert_eq!(r.output["deleted"], true);
    }
    #[tokio::test] async fn missing_id_err() {
        let err = DeleteMemoryTool.execute(&p(), serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
}
