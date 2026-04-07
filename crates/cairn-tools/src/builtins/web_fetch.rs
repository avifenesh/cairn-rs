//! `web_fetch` built-in tool stub — HTTP GET for external content.
//!
//! Full implementation is deferred. This stub satisfies the registry export.

use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

pub struct WebFetchTool;

impl Default for WebFetchTool { fn default() -> Self { Self } }

#[async_trait]
impl ToolHandler for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }
    fn tier(&self) -> ToolTier { ToolTier::Registered }
    fn description(&self) -> &str { "Fetch content from an HTTP URL." }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({ "type": "object", "required": ["url"],
            "properties": { "url": { "type": "string" } } })
    }
    async fn execute(&self, _project: &ProjectKey, _args: Value) -> Result<ToolResult, ToolError> {
        Err(ToolError::Permanent("web_fetch not yet implemented".into()))
    }
}
