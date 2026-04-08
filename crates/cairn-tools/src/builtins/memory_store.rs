//! `memory_store` built-in tool — stub definition.
//!
//! Concrete implementation lives in cairn-memory to avoid the
//! cairn-api → cairn-tools → cairn-memory → cairn-api cycle.

use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

/// Stub: schema + metadata only. Concrete impl in cairn-memory.
pub struct MemoryStoreTool;

impl MemoryStoreTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemoryStoreTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolHandler for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Core
    }
    fn description(&self) -> &str {
        "Store new content or a fact into the agent's memory for future retrieval."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": { "type": "string", "description": "Text to store" },
                "source_id": { "type": "string", "description": "Optional source identifier" }
            }
        })
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let content = args["content"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "content".into(),
                message: "required".into(),
            })?;
        Ok(ToolResult::ok(serde_json::json!({
            "stored": true,
            "content_length": content.len(),
            "note": "memory_store stub — wire ConcreteMemoryStoreTool from cairn-memory"
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[test]
    fn tier_is_core() {
        assert_eq!(MemoryStoreTool::new().tier(), ToolTier::Core);
    }

    #[tokio::test]
    async fn stores_content() {
        let res = MemoryStoreTool::new()
            .execute(&project(), serde_json::json!({"content":"hello"}))
            .await
            .unwrap();
        assert_eq!(res.output["stored"], true);
    }

    #[tokio::test]
    async fn requires_content() {
        let err = MemoryStoreTool::new()
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
}
