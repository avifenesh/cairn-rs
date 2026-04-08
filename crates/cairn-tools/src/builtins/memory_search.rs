//! `memory_search` built-in tool — queries the retrieval service.
//!
//! ## Architecture note
//! The concrete implementation lives in `cairn-memory` (which can safely
//! depend on both `cairn-tools` and the retrieval infrastructure) to avoid
//! a cairn-api → cairn-tools → cairn-memory → cairn-api dependency cycle.
//!
//! This stub provides the schema and metadata so the registry and LLM prompt
//! builder work correctly.  Wire the real implementation in cairn-memory or
//! cairn-app with:
//!
//! ```rust,ignore
//! registry.add(Arc::new(ConcreteMemorySearchTool::new(retrieval.clone())));
//! ```

use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

/// Stub tool handler — describes the memory_search interface for the registry
/// and LLM prompt.  Replace with a concrete implementation in cairn-memory.
pub struct MemorySearchTool;

impl MemorySearchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemorySearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolHandler for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Core
    }

    fn description(&self) -> &str {
        "Search the agent's memory for relevant information. \
         Returns the most relevant text chunks from previously stored knowledge."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results (default 5, max 20)",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 20
                },
                "mode": {
                    "type": "string",
                    "enum": ["lexical", "vector", "hybrid"],
                    "default": "lexical"
                }
            }
        })
    }

    async fn execute(&self, _project: &ProjectKey, _args: Value) -> Result<ToolResult, ToolError> {
        // Stub: concrete implementation is in cairn-memory.
        // Returns an empty result so the LLM can reason about it without crashing.
        Ok(ToolResult::ok(serde_json::json!({
            "results": [],
            "total": 0,
            "note": "memory_search stub — wire ConcreteMemorySearchTool from cairn-memory"
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
        assert_eq!(MemorySearchTool::new().tier(), ToolTier::Core);
    }

    #[test]
    fn schema_has_required_query() {
        let s = MemorySearchTool::new().parameters_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("query")));
    }

    #[tokio::test]
    async fn stub_returns_empty_results() {
        let tool = MemorySearchTool::new();
        let res = tool
            .execute(&project(), serde_json::json!({"query":"test"}))
            .await
            .unwrap();
        assert!(!res.truncated);
        assert_eq!(res.output["total"], 0);
    }
}
