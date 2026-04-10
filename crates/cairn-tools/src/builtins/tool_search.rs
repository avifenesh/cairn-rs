//! `tool_search` built-in tool — lazy discovery of Deferred-tier tools.
//!
//! ## Problem
//! Listing all tools in every system prompt wastes context tokens.  The
//! three-tier registry keeps Deferred tools out of the prompt by default.
//!
//! ## Solution
//! `tool_search` is a Core-tier tool (always in the prompt) that lets the LLM
//! discover Deferred tools on demand by capability query.  The orchestrator
//! injects the matching descriptors into the *next* iteration's prompt so the
//! LLM can call the newly-discovered tool immediately.
//!
//! ## Usage (LLM perspective)
//! ```json
//! {
//!   "action_type": "invoke_tool",
//!   "tool_name": "tool_search",
//!   "tool_args": { "query": "execute shell commands", "namespace": "cairn" }
//! }
//! ```
//!
//! ## Response shape
//! ```json
//! {
//!   "matches": [
//!     {
//!       "name":        "shell_exec",
//!       "description": "Run a shell command …",
//!       "parameters_schema": { … }
//!     }
//!   ],
//!   "total": 1,
//!   "tip":   "Call these tools using invoke_tool with tool_name set to their name."
//! }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde_json::Value;

use super::{BuiltinToolRegistry, ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

// ── ToolSearchTool ────────────────────────────────────────────────────────────

/// Core-tier tool that searches the Deferred registry by capability query.
///
/// Always present in the system prompt (one compact line) so the LLM always
/// knows it can discover more tools.
pub struct ToolSearchTool {
    registry: Arc<BuiltinToolRegistry>,
}

impl ToolSearchTool {
    pub fn new(registry: Arc<BuiltinToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ToolHandler for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Core
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }

    fn description(&self) -> &str {
        "Discover available tools by capability. \
         Call this when you need a tool that is not listed above. \
         Returns matching tool descriptors that are added to your next turn."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language description of the capability you need, \
                                    e.g. 'execute shell commands' or 'read files from disk'"
                },
                "namespace": {
                    "type": "string",
                    "description": "Optional namespace prefix to narrow results, \
                                    e.g. 'cairn', 'mcp', 'plugin'"
                }
            }
        })
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "query".into(),
                message: "must be a non-empty string".into(),
            })?
            .trim();

        if query.is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "query".into(),
                message: "query must not be empty".into(),
            });
        }

        // Optional namespace filter applied after the deferred search.
        let namespace = args["namespace"].as_str().map(str::to_lowercase);

        let mut matches = self.registry.search_deferred(query);

        // Apply namespace filter when provided.
        if let Some(ref ns) = namespace {
            matches.retain(|d| d.name.to_lowercase().starts_with(ns.as_str()));
        }

        let total = matches.len();
        let result_json: Vec<Value> = matches
            .into_iter()
            .map(|d| {
                serde_json::json!({
                    "name":              d.name,
                    "description":       d.description,
                    "parameters_schema": d.parameters_schema,
                })
            })
            .collect();

        Ok(ToolResult::ok(serde_json::json!({
            "matches": result_json,
            "total":   total,
            "tip":     "Use invoke_tool with tool_name set to the tool's name to call it.",
        })))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use cairn_domain::ProjectKey;
    use serde_json::Value;

    use super::*;
    use crate::builtins::{ToolError, ToolHandler, ToolResult, ToolTier};

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    // ── Deferred test tool ────────────────────────────────────────────────────

    struct ShellExecStub;
    #[async_trait]
    impl ToolHandler for ShellExecStub {
        fn name(&self) -> &str {
            "shell_exec"
        }
        fn tier(&self) -> ToolTier {
            ToolTier::Deferred
        }
        fn description(&self) -> &str {
            "Execute a shell command on the host system."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object","required":["cmd"],"properties":{"cmd":{"type":"string"}}})
        }
        async fn execute(&self, _: &ProjectKey, _: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::ok(serde_json::json!({"exit_code":0})))
        }
    }

    struct WebFetchStub;
    #[async_trait]
    impl ToolHandler for WebFetchStub {
        fn name(&self) -> &str {
            "web_fetch"
        }
        fn tier(&self) -> ToolTier {
            ToolTier::Deferred
        }
        fn description(&self) -> &str {
            "Fetch a URL and return its contents."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object","required":["url"],"properties":{"url":{"type":"string"}}})
        }
        async fn execute(&self, _: &ProjectKey, _: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::ok(serde_json::json!({"body":""})))
        }
    }

    fn make_registry() -> Arc<BuiltinToolRegistry> {
        Arc::new(
            BuiltinToolRegistry::new()
                .register(Arc::new(ShellExecStub))
                .register(Arc::new(WebFetchStub)),
        )
    }

    fn make_tool() -> ToolSearchTool {
        ToolSearchTool::new(make_registry())
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn tier_is_core() {
        assert_eq!(make_tool().tier(), ToolTier::Core);
    }

    #[test]
    fn schema_has_required_query() {
        let schema = make_tool().parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
        assert!(
            schema["properties"]["namespace"].is_object(),
            "namespace must be optional"
        );
    }

    // ── Successful search ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn finds_deferred_tool_by_description() {
        let tool = make_tool();
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "execute shell commands"
                }),
            )
            .await
            .unwrap();

        let matches = res.output["matches"].as_array().unwrap();
        assert!(!matches.is_empty(), "should find shell_exec");
        let names: Vec<&str> = matches
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"shell_exec"),
            "shell_exec must be in results"
        );
    }

    #[tokio::test]
    async fn finds_deferred_tool_by_name() {
        let tool = make_tool();
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "web_fetch"
                }),
            )
            .await
            .unwrap();

        let matches = res.output["matches"].as_array().unwrap();
        let names: Vec<&str> = matches
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"web_fetch"));
    }

    #[tokio::test]
    async fn result_includes_parameters_schema() {
        let tool = make_tool();
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "shell"
                }),
            )
            .await
            .unwrap();

        let matches = res.output["matches"].as_array().unwrap();
        assert!(!matches.is_empty());
        // Each match must carry the full schema so the LLM can call the tool
        let schema = &matches[0]["parameters_schema"];
        assert!(schema.is_object(), "parameters_schema must be present");
    }

    #[tokio::test]
    async fn no_match_returns_empty_list() {
        let tool = make_tool();
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "quantum_teleportation_xyz_nonexistent"
                }),
            )
            .await
            .unwrap();

        assert_eq!(res.output["total"], 0);
        assert!(res.output["matches"].as_array().unwrap().is_empty());
    }

    // ── Namespace filter ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn namespace_filter_narrows_results() {
        let tool = make_tool();

        // Both tools match "fetch" broadly, but namespace=shell narrows to none
        // because neither starts with "shell_" — shell_exec starts with "shell"
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "execute",
                    "namespace": "shell"
                }),
            )
            .await
            .unwrap();

        let names: Vec<&str> = res.output["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        // "shell_exec" starts with "shell" so should be included
        assert!(
            names.iter().all(|n| n.starts_with("shell")),
            "namespace filter must only return tools starting with 'shell'"
        );
    }

    #[tokio::test]
    async fn namespace_filter_excludes_non_matching() {
        let tool = make_tool();
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "fetch",
                    "namespace": "shell"   // web_fetch does not start with "shell"
                }),
            )
            .await
            .unwrap();

        let names: Vec<&str> = res.output["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert!(
            !names.contains(&"web_fetch"),
            "web_fetch must be excluded by namespace=shell"
        );
    }

    // ── Error cases ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_query_returns_err() {
        let tool = make_tool();
        let err = tool
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn empty_query_returns_err() {
        let tool = make_tool();
        let err = tool
            .execute(&project(), serde_json::json!({"query": "  "}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    // ── Response shape ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn response_has_tip_field() {
        let tool = make_tool();
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "shell"
                }),
            )
            .await
            .unwrap();
        assert!(
            res.output["tip"].as_str().unwrap().contains("invoke_tool"),
            "tip must guide LLM on how to call discovered tools"
        );
    }

    #[tokio::test]
    async fn does_not_return_core_or_registered_tools() {
        // Add a Core tool to the registry and verify it never appears in results
        use crate::builtins::MemorySearchTool;
        let registry = Arc::new(
            BuiltinToolRegistry::new()
                .register(Arc::new(ShellExecStub)) // Deferred
                .register(Arc::new(MemorySearchTool::new())), // Core
        );
        let tool = ToolSearchTool::new(registry);
        // "memory" could match MemorySearchTool's description — but it's Core
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "query": "memory"
                }),
            )
            .await
            .unwrap();
        let names: Vec<&str> = res.output["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert!(
            !names.contains(&"memory_search"),
            "Core/Registered tools must never appear in tool_search results"
        );
    }
}
