//! Built-in tool infrastructure — ToolHandler trait, BuiltinToolRegistry, tiers.
//!
//! ## Three-tier tool discovery
//!
//! | Tier          | In prompt?         | Example tools                         |
//! |---------------|--------------------|---------------------------------------|
//! | `Core`        | Always             | memory_search, tool_search, complete  |
//! | `Registered`  | If total ≤ budget  | web_fetch, shell_exec, graph_query    |
//! | `Deferred`    | Never (discovered) | MCP server tools, plugin tools        |
//!
//! The orchestrator's `PromptBuilder` calls `registry.prompt_tools()` which
//! returns Core + Registered descriptors.  When the LLM calls `tool_search`,
//! the result is a list of Deferred descriptors that get injected into the
//! *next* iteration's prompt.

pub mod calculate;
pub mod cancel_task;
pub mod create_task;
pub mod eval_score;
pub mod file_read;
pub mod file_write;
pub mod get_approvals;
pub mod get_run;
pub mod get_task;
pub mod git_operations;
pub mod glob_find;
pub mod graph_query;
pub mod grep_search;
pub mod http_request;
pub mod json_extract;
pub mod list_runs;
pub mod memory_search;
pub mod memory_store;
pub mod notify_operator;
pub mod resolve_approval;
pub mod schedule_task;
pub mod scratch_pad;
pub mod search_events;
pub mod shell_exec;
pub mod summarize_text;
pub mod tool_search;
pub mod update_memory;
pub mod wait_for_task;
pub mod web_fetch;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use calculate::CalculateTool;
pub use cancel_task::CancelTaskTool;
pub use create_task::CreateTaskTool;
pub use eval_score::EvalScoreTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use get_approvals::GetApprovalsTool;
pub use get_run::GetRunTool;
pub use get_task::GetTaskTool;
pub use git_operations::GitOperationsTool;
pub use glob_find::GlobFindTool;
pub use graph_query::GraphQueryTool;
pub use grep_search::GrepSearchTool;
pub use http_request::HttpRequestTool;
pub use json_extract::JsonExtractTool;
pub use list_runs::ListRunsTool;
pub use memory_search::MemorySearchTool;
pub use memory_store::MemoryStoreTool;
pub use notify_operator::{NoopSink, NotificationSink, NotifyOperatorTool};
pub use resolve_approval::ResolveApprovalTool;
pub use schedule_task::ScheduleTaskTool;
pub use scratch_pad::ScratchPadTool;
pub use search_events::SearchEventsTool;
pub use shell_exec::ShellExecTool;
pub use summarize_text::SummarizeTextTool;
pub use tool_search::ToolSearchTool;
pub use update_memory::{DeleteFn, DeleteMemoryTool, ReingestFn, UpdateMemoryTool};
pub use wait_for_task::WaitForTaskTool;
pub use web_fetch::WebFetchTool;

// ── ToolTier ──────────────────────────────────────────────────────────────────

/// Determines when a tool's descriptor is included in the LLM system prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTier {
    /// Always injected into the system prompt.
    /// Use for tools the agent needs every turn (memory_search, complete_run).
    Core,
    /// Injected at startup if the total prompt-token budget allows.
    /// Use for frequently-useful but not always-needed tools.
    Registered,
    /// Never injected automatically — discovered via the `tool_search` built-in.
    /// Use for MCP server tools, plugin tools, and rarely-used utilities.
    Deferred,
}

// ── ToolResult ────────────────────────────────────────────────────────────────

/// The structured output of a successful tool execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    /// JSON observation fed back to the LLM.  Shape is tool-specific.
    pub output: Value,
    /// `true` if the output was trimmed to fit context limits.
    /// The orchestrator should warn the LLM when this flag is set.
    pub truncated: bool,
}

impl ToolResult {
    /// Create a complete (non-truncated) result.
    pub fn ok(output: Value) -> Self {
        Self {
            output,
            truncated: false,
        }
    }
    /// Create a result flagged as truncated.
    pub fn truncated(output: Value) -> Self {
        Self {
            output,
            truncated: true,
        }
    }
}

// ── ToolError ─────────────────────────────────────────────────────────────────

/// Why a tool invocation failed.
#[derive(Debug)]
pub enum ToolError {
    /// Argument payload was malformed or missing a required field.
    InvalidArgs { field: String, message: String },
    /// Transient failure — the orchestrator may retry.
    Transient(String),
    /// Permanent failure — do not retry.
    Permanent(String),
    /// Invocation was cancelled before it completed.
    Cancelled,
    /// Tool exceeded its wall-clock budget.
    TimedOut,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::InvalidArgs { field, message } => {
                write!(f, "invalid argument '{}': {}", field, message)
            }
            ToolError::Transient(m) => write!(f, "transient error: {}", m),
            ToolError::Permanent(m) => write!(f, "permanent error: {}", m),
            ToolError::Cancelled => write!(f, "cancelled"),
            ToolError::TimedOut => write!(f, "timed out"),
        }
    }
}

impl std::error::Error for ToolError {}

// Convenience conversions so existing code that uses String errors still compiles.
impl From<String> for ToolError {
    fn from(s: String) -> Self {
        ToolError::Permanent(s)
    }
}
impl From<&str> for ToolError {
    fn from(s: &str) -> Self {
        ToolError::Permanent(s.to_owned())
    }
}

// ── ToolHandler trait ─────────────────────────────────────────────────────────

/// Async interface that every built-in tool must implement.
///
/// # Implementing a tool
///
/// ```rust,ignore
/// use cairn_tools::builtins::{ToolHandler, ToolResult, ToolError, ToolTier};
/// use async_trait::async_trait;
///
/// struct WebSearchTool;
///
/// #[async_trait]
/// impl ToolHandler for WebSearchTool {
///     fn name(&self)        -> &str { "web_search" }
///     fn tier(&self)        -> ToolTier { ToolTier::Registered }
///     fn description(&self) -> &str { "Search the web for up-to-date information." }
///     fn parameters_schema(&self) -> serde_json::Value {
///         serde_json::json!({
///             "type": "object",
///             "required": ["query"],
///             "properties": { "query": { "type": "string" } }
///         })
///     }
///     async fn execute(
///         &self, project: &ProjectKey, args: serde_json::Value,
///     ) -> Result<ToolResult, ToolError> {
///         let q = args["query"].as_str()
///             .ok_or(ToolError::InvalidArgs { field: "query".into(), message: "required".into() })?;
///         Ok(ToolResult::ok(serde_json::json!({ "results": [] })))
///     }
/// }
/// ```
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Stable snake_case name used for dispatch (e.g. `"memory_search"`).
    fn name(&self) -> &str;

    /// Prompt-inclusion tier (Core / Registered / Deferred).
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    /// One-sentence description shown to the LLM.
    fn description(&self) -> &str;

    /// JSON Schema object for the tool's argument payload.
    fn parameters_schema(&self) -> Value;

    /// Execution class for the orchestrator approval gate.
    ///
    /// Returns `Sensitive` to require operator approval before execution.
    /// Default is `SupervisedProcess` (no approval required).
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    /// Execute the tool with the given project context and parsed arguments.
    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError>;
}

// ── BuiltinToolDescriptor ─────────────────────────────────────────────────────

/// Rich descriptor used in both the LLM prompt and the operator API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuiltinToolDescriptor {
    pub name: String,
    pub tier: ToolTier,
    pub description: String,
    pub parameters_schema: Value,
    /// `Sensitive` tools require operator approval before the execute phase
    /// dispatches them.  The orchestrator reads this field to set
    /// `ActionProposal::requires_approval`.
    pub execution_class: ExecutionClass,
}

impl BuiltinToolDescriptor {
    pub fn from_handler(h: &dyn ToolHandler) -> Self {
        Self {
            name: h.name().to_owned(),
            tier: h.tier(),
            description: h.description().to_owned(),
            parameters_schema: h.parameters_schema(),
            execution_class: h.execution_class(),
        }
    }

    /// Compact one-line representation for injection into a system prompt.
    ///
    /// Example: `memory_search(query: string, limit?: integer) — Search memory.`
    pub fn prompt_line(&self) -> String {
        let required: Vec<&str> = self
            .parameters_schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let param_summary = self
            .parameters_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| {
                props
                    .iter()
                    .map(|(k, v)| {
                        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("any");
                        let optional = if required.contains(&k.as_str()) {
                            ""
                        } else {
                            "?"
                        };
                        format!("{k}{optional}: {ty}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();

        if param_summary.is_empty() {
            format!("{}() — {}", self.name, self.description)
        } else {
            format!("{}({}) — {}", self.name, param_summary, self.description)
        }
    }
}

// ── BuiltinToolRegistry ───────────────────────────────────────────────────────

/// Thread-safe registry of installed [`ToolHandler`] implementations.
///
/// Tools are stored with their tier.  `prompt_tools()` returns only Core +
/// Registered descriptors; Deferred tools are discovered via `tool_search`.
pub struct BuiltinToolRegistry {
    /// Ordered map: name → (handler, tier)
    tools: HashMap<String, (Arc<dyn ToolHandler>, ToolTier)>,
}

impl BuiltinToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Builder-style registration.  Last-write wins on name collision.
    pub fn register(mut self, handler: Arc<dyn ToolHandler>) -> Self {
        let tier = handler.tier();
        self.tools
            .insert(handler.name().to_owned(), (handler, tier));
        self
    }

    /// Non-consuming registration (for use with `Arc<BuiltinToolRegistry>`).
    pub fn add(&mut self, handler: Arc<dyn ToolHandler>) {
        let tier = handler.tier();
        self.tools
            .insert(handler.name().to_owned(), (handler, tier));
    }

    /// Look up a handler by name (works for all tiers).
    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.tools.get(name).map(|(h, _)| h.clone())
    }

    /// Execute a tool by name, returning the observation or an error.
    pub async fn execute(
        &self,
        tool_name: &str,
        project: &ProjectKey,
        args: Value,
    ) -> Result<ToolResult, ToolError> {
        match self.tools.get(tool_name) {
            Some((handler, _)) => handler.execute(project, args).await,
            None => Err(ToolError::Permanent(format!("unknown tool: {tool_name}"))),
        }
    }

    /// Descriptors for Core + Registered tools — injected into the LLM system prompt.
    /// Deferred tools are deliberately excluded to save context tokens.
    pub fn prompt_tools(&self) -> Vec<BuiltinToolDescriptor> {
        let mut tools: Vec<BuiltinToolDescriptor> = self
            .tools
            .values()
            .filter(|(_, tier)| matches!(tier, ToolTier::Core | ToolTier::Registered))
            .map(|(h, _)| BuiltinToolDescriptor::from_handler(h.as_ref()))
            .collect();
        tools.sort_by(|a, b| {
            // Core first, then Registered, then alphabetical within tier
            let tier_ord = |t: &ToolTier| match t {
                ToolTier::Core => 0u8,
                ToolTier::Registered => 1,
                ToolTier::Deferred => 2,
            };
            tier_ord(&a.tier)
                .cmp(&tier_ord(&b.tier))
                .then_with(|| a.name.cmp(&b.name))
        });
        tools
    }

    /// Descriptors for Deferred tools matching the given capability query.
    /// Used by the `tool_search` built-in to surface on-demand tools.
    pub fn search_deferred(&self, query: &str) -> Vec<BuiltinToolDescriptor> {
        let q_lower = query.to_lowercase();
        // Split into words so "execute shell commands" matches "Execute a shell command".
        let words: Vec<&str> = q_lower.split_whitespace().filter(|w| w.len() > 2).collect();

        let matches_query = |h: &Arc<dyn ToolHandler>| -> bool {
            let name = h.name().to_lowercase();
            let desc = h.description().to_lowercase();
            // Full-query substring match (fast path).
            if name.contains(q_lower.as_str()) || desc.contains(q_lower.as_str()) {
                return true;
            }
            // Word-level match: any meaningful query word appears in name or description.
            words.iter().any(|w| name.contains(w) || desc.contains(w))
        };

        let mut tools: Vec<BuiltinToolDescriptor> = self
            .tools
            .values()
            .filter(|(h, tier)| *tier == ToolTier::Deferred && matches_query(h))
            .map(|(h, _)| BuiltinToolDescriptor::from_handler(h.as_ref()))
            .collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    /// All tool descriptors regardless of tier — for the operator API.
    pub fn list_all(&self) -> Vec<BuiltinToolDescriptor> {
        let mut tools: Vec<BuiltinToolDescriptor> = self
            .tools
            .values()
            .map(|(h, _)| BuiltinToolDescriptor::from_handler(h.as_ref()))
            .collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Legacy compat: list names for the discovery endpoint.
    pub fn tool_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.tools.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Legacy compat: JSON catalogue for the system prompt.
    /// Prefer `prompt_tools()` in new code.
    pub fn catalogue_json(&self) -> Value {
        let tools: Vec<Value> = self
            .prompt_tools()
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name":        d.name,
                    "description": d.description,
                    "parameters":  d.parameters_schema,
                })
            })
            .collect();
        Value::Array(tools)
    }
}

impl Default for BuiltinToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    struct CoreEcho;
    #[async_trait]
    impl ToolHandler for CoreEcho {
        fn name(&self) -> &str {
            "echo"
        }
        fn tier(&self) -> ToolTier {
            ToolTier::Core
        }
        fn description(&self) -> &str {
            "Echo the message."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object","required":["msg"],"properties":{"msg":{"type":"string"}}})
        }
        async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
            let msg = args["msg"].as_str().ok_or_else(|| ToolError::InvalidArgs {
                field: "msg".into(),
                message: "required".into(),
            })?;
            Ok(ToolResult::ok(serde_json::json!({ "echo": msg })))
        }
    }

    struct RegisteredSearch;
    #[async_trait]
    impl ToolHandler for RegisteredSearch {
        fn name(&self) -> &str {
            "web_search"
        }
        fn tier(&self) -> ToolTier {
            ToolTier::Registered
        }
        fn description(&self) -> &str {
            "Search the web."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object","required":["query"],"properties":{"query":{"type":"string"}}})
        }
        async fn execute(&self, _: &ProjectKey, _: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::ok(serde_json::json!({"results":[]})))
        }
    }

    struct DeferredPlugin;
    #[async_trait]
    impl ToolHandler for DeferredPlugin {
        fn name(&self) -> &str {
            "plugin_tool"
        }
        fn tier(&self) -> ToolTier {
            ToolTier::Deferred
        }
        fn description(&self) -> &str {
            "A deferred plugin tool for special tasks."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object","properties":{}})
        }
        async fn execute(&self, _: &ProjectKey, _: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::ok(serde_json::json!({})))
        }
    }

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn make_registry() -> BuiltinToolRegistry {
        BuiltinToolRegistry::new()
            .register(Arc::new(CoreEcho))
            .register(Arc::new(RegisteredSearch))
            .register(Arc::new(DeferredPlugin))
    }

    // ── ToolResult ────────────────────────────────────────────────────────────

    #[test]
    fn tool_result_ok_not_truncated() {
        let r = ToolResult::ok(serde_json::json!(42));
        assert!(!r.truncated);
    }

    #[test]
    fn tool_result_truncated_flag() {
        let r = ToolResult::truncated(serde_json::json!("..."));
        assert!(r.truncated);
    }

    // ── ToolError ─────────────────────────────────────────────────────────────

    #[test]
    fn tool_error_display() {
        let e = ToolError::InvalidArgs {
            field: "query".into(),
            message: "required".into(),
        };
        assert!(e.to_string().contains("query") && e.to_string().contains("required"));
        assert!(ToolError::Transient("net".into())
            .to_string()
            .contains("net"));
        assert!(ToolError::Cancelled.to_string() == "cancelled");
        assert!(ToolError::TimedOut.to_string() == "timed out");
    }

    // ── BuiltinToolDescriptor ─────────────────────────────────────────────────

    #[test]
    fn descriptor_prompt_line_required_vs_optional() {
        let desc = BuiltinToolDescriptor::from_handler(&RegisteredSearch);
        let line = desc.prompt_line();
        assert!(line.contains("query: string"), "required param has no '?'");
    }

    #[test]
    fn descriptor_prompt_line_empty_params() {
        let desc = BuiltinToolDescriptor::from_handler(&DeferredPlugin);
        let line = desc.prompt_line();
        assert!(
            line.contains("plugin_tool()"),
            "no-arg tool gets empty parens"
        );
    }

    // ── BuiltinToolRegistry ───────────────────────────────────────────────────

    #[test]
    fn registry_get_any_tier() {
        let reg = make_registry();
        assert!(reg.get("echo").is_some());
        assert!(reg.get("web_search").is_some());
        assert!(reg.get("plugin_tool").is_some());
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn prompt_tools_excludes_deferred() {
        let reg = make_registry();
        let prompt = reg.prompt_tools();
        let names: Vec<&str> = prompt.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"echo"), "Core must be in prompt tools");
        assert!(
            names.contains(&"web_search"),
            "Registered must be in prompt tools"
        );
        assert!(
            !names.contains(&"plugin_tool"),
            "Deferred must NOT be in prompt tools"
        );
    }

    #[test]
    fn prompt_tools_core_before_registered() {
        let reg = make_registry();
        let tools = reg.prompt_tools();
        let core_pos = tools.iter().position(|d| d.tier == ToolTier::Core).unwrap();
        let reg_pos = tools
            .iter()
            .position(|d| d.tier == ToolTier::Registered)
            .unwrap();
        assert!(core_pos < reg_pos, "Core tools must come before Registered");
    }

    #[test]
    fn search_deferred_finds_matching_tools() {
        let reg = make_registry();
        let found = reg.search_deferred("plugin");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "plugin_tool");
    }

    #[test]
    fn search_deferred_no_match_returns_empty() {
        let reg = make_registry();
        assert!(reg.search_deferred("nonexistent_capability_xyz").is_empty());
    }

    #[test]
    fn search_deferred_does_not_return_core_or_registered() {
        let reg = make_registry();
        let found = reg.search_deferred("echo");
        assert!(
            found.is_empty(),
            "Core 'echo' must not appear in deferred search"
        );
    }

    #[test]
    fn list_all_returns_all_tiers() {
        let reg = make_registry();
        assert_eq!(reg.list_all().len(), 3);
    }

    #[test]
    fn last_write_wins() {
        struct Echo2;
        #[async_trait]
        impl ToolHandler for Echo2 {
            fn name(&self) -> &str {
                "echo"
            }
            fn tier(&self) -> ToolTier {
                ToolTier::Core
            }
            fn description(&self) -> &str {
                "v2"
            }
            fn parameters_schema(&self) -> Value {
                serde_json::json!({})
            }
            async fn execute(&self, _: &ProjectKey, _: Value) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::ok(serde_json::json!({})))
            }
        }
        let reg = BuiltinToolRegistry::new()
            .register(Arc::new(CoreEcho))
            .register(Arc::new(Echo2));
        assert_eq!(reg.get("echo").unwrap().description(), "v2");
    }

    #[tokio::test]
    async fn execute_via_registry_success() {
        let reg = make_registry();
        let res = reg
            .execute("echo", &project(), serde_json::json!({"msg":"hi"}))
            .await
            .unwrap();
        assert_eq!(res.output["echo"], "hi");
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_permanent_error() {
        let reg = make_registry();
        let err = reg
            .execute("no_such_tool", &project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
    }
}
