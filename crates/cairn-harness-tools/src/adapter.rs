//! `HarnessTool` trait + `HarnessBuiltin<H>` wrapper.
//!
//! Each upstream tool (bash, read, grep, ...) is represented by an empty
//! type implementing `HarnessTool`. `HarnessBuiltin<H>` is the generic
//! wrapper that adapts `H` onto cairn's `ToolHandler` — register one
//! `Arc::new(HarnessBuiltin::<H>::new())` per tool.

use std::marker::PhantomData;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolHandler, ToolResult,
    ToolTier,
};
use harness_core::PermissionHook;
use serde_json::Value;

/// Contract every harness-backed tool implements.
///
/// Types in `tools/` implement this for a unit struct (e.g. `HarnessBash`),
/// then `HarnessBuiltin<HarnessBash>` wraps it for cairn's registry.
#[async_trait]
pub trait HarnessTool: Send + Sync + 'static {
    /// Upstream session config (e.g. `BashSessionConfig`).
    type Session: Send + Sync;

    /// Upstream result union (e.g. `BashResult`).
    type Result: Send;

    /// Stable snake_case name used for dispatch.
    fn name() -> &'static str;

    /// One-sentence description for the LLM system prompt.
    fn description() -> &'static str;

    /// JSON Schema for the tool's argument payload.
    fn parameters_schema() -> Value;

    /// Prompt-inclusion tier.
    fn tier() -> ToolTier {
        ToolTier::Registered
    }

    /// Orchestrator approval gate.
    fn execution_class() -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    /// Granular permission level for policy enforcement.
    fn permission_level() -> PermissionLevel {
        PermissionLevel::None
    }

    /// Logical grouping.
    fn category() -> ToolCategory {
        ToolCategory::Custom
    }

    /// RFC 018 side-effect classification.
    fn tool_effect() -> ToolEffect {
        ToolEffect::External
    }

    /// Retry policy.
    fn retry_safety() -> RetrySafety {
        RetrySafety::DangerousPause
    }

    /// Build a session config from cairn's execution context.
    fn build_session(ctx: &ToolContext, project: &ProjectKey, hook: PermissionHook) -> Self::Session;

    /// Invoke the upstream async entrypoint.
    async fn call(args: Value, session: &Self::Session) -> Self::Result;

    /// Convert the upstream result union into cairn's `ToolResult` / `ToolError`.
    fn result_to_tool_result(result: Self::Result) -> Result<ToolResult, ToolError>;
}

/// Wrapper implementing `ToolHandler` for any `HarnessTool`.
///
/// Zero-sized — holds only a `PhantomData<H>`. Registration looks like:
/// ```ignore
/// registry.register(Arc::new(HarnessBuiltin::<HarnessBash>::new()));
/// ```
pub struct HarnessBuiltin<H: HarnessTool> {
    _phantom: PhantomData<fn() -> H>,
}

impl<H: HarnessTool> HarnessBuiltin<H> {
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<H: HarnessTool> Default for HarnessBuiltin<H> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<H: HarnessTool> ToolHandler for HarnessBuiltin<H> {
    fn name(&self) -> &str {
        H::name()
    }

    fn tier(&self) -> ToolTier {
        H::tier()
    }

    fn description(&self) -> &str {
        H::description()
    }

    fn parameters_schema(&self) -> Value {
        H::parameters_schema()
    }

    fn execution_class(&self) -> ExecutionClass {
        H::execution_class()
    }

    fn permission_level(&self) -> PermissionLevel {
        H::permission_level()
    }

    fn category(&self) -> ToolCategory {
        H::category()
    }

    fn tool_effect(&self) -> ToolEffect {
        H::tool_effect()
    }

    fn retry_safety(&self) -> RetrySafety {
        H::retry_safety()
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let ctx = ToolContext::default();
        self.execute_with_context(project, args, &ctx).await
    }

    async fn execute_with_context(
        &self,
        project: &ProjectKey,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let hook = crate::hook::build_cairn_hook();
        let session = H::build_session(ctx, project, hook);
        let result = H::call(args, &session).await;
        H::result_to_tool_result(result)
    }
}
