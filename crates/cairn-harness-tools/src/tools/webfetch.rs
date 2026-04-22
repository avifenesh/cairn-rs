//! harness-webfetch → cairn: `webfetch`.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolError, ToolResult,
};
use harness_core::{PermissionHook, PermissionPolicy};
use harness_webfetch::{
    webfetch, ReqwestEngine, WebFetchEngine, WebFetchPermissionPolicy, WebFetchResult,
    WebFetchSessionConfig, WEBFETCH_TOOL_NAME,
};
use once_cell::sync::Lazy;
use serde_json::{json, Value};

use crate::adapter::HarnessTool;
use crate::error::map_harness;
use crate::sensitive::default_sensitive_patterns;

static DEFAULT_ENGINE: Lazy<Arc<dyn WebFetchEngine>> =
    Lazy::new(|| Arc::new(ReqwestEngine::default()) as Arc<dyn WebFetchEngine>);

pub struct HarnessWebFetch;

#[async_trait]
impl HarnessTool for HarnessWebFetch {
    type Session = WebFetchSessionConfig;
    type Result = WebFetchResult;

    fn name() -> &'static str {
        WEBFETCH_TOOL_NAME
    }
    fn description() -> &'static str {
        "HTTP GET/POST with SSRF defense, redirect-loop detection, and readability extraction."
    }
    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url":        { "type": "string", "description": "Absolute http(s) URL." },
                "method":     { "type": "string", "enum": ["GET", "POST"] },
                "headers":    { "type": "object", "additionalProperties": { "type": "string" } },
                "body":       { "type": "string", "description": "Request body (POST only)." },
                "extract":    { "type": "string", "enum": ["markdown", "raw", "both"] },
                "timeout_ms": { "type": "integer" }
            }
        })
    }
    fn execution_class() -> ExecutionClass {
        // Match the removed `web_fetch` tool — non-sensitive read-only fetch.
        ExecutionClass::SupervisedProcess
    }
    fn permission_level() -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category() -> ToolCategory {
        ToolCategory::Web
    }
    fn tool_effect() -> ToolEffect {
        // Web reads are observational — must be visible in Plan mode.
        ToolEffect::Observational
    }
    fn retry_safety() -> RetrySafety {
        RetrySafety::IdempotentSafe
    }

    fn build_session(
        _ctx: &ToolContext,
        _project: &ProjectKey,
        hook: PermissionHook,
    ) -> Self::Session {
        let inner = PermissionPolicy {
            roots: Vec::new(),
            sensitive_patterns: default_sensitive_patterns(),
            hook: Some(hook),
            bypass_workspace_guard: true,
        };
        let perms = WebFetchPermissionPolicy::new(inner);
        WebFetchSessionConfig::new(perms, DEFAULT_ENGINE.clone())
    }

    async fn call(args: Value, session: &Self::Session) -> Self::Result {
        webfetch(args, session).await
    }

    fn result_to_tool_result(
        result: Self::Result,
        _ctx: &ToolContext,
        _project: &ProjectKey,
    ) -> Result<ToolResult, ToolError> {
        match result {
            WebFetchResult::Ok(ok) => {
                let truncated = ok.byte_cap;
                let v = json!({
                    "kind": "ok",
                    "output": ok.output,
                    "meta": ok.meta,
                    "body_markdown": ok.body_markdown,
                    "body_raw": ok.body_raw,
                    "log_path": ok.log_path,
                    "byte_cap": ok.byte_cap,
                });
                Ok(if truncated {
                    ToolResult::truncated(v)
                } else {
                    ToolResult::ok(v)
                })
            }
            WebFetchResult::RedirectLoop(r) => Ok(ToolResult::ok(json!({
                "kind": "redirect_loop",
                "output": r.output,
                "meta": r.meta,
            }))),
            WebFetchResult::HttpError(h) => Ok(ToolResult::ok(json!({
                "kind": "http_error",
                "output": h.output,
                "meta": h.meta,
                "body_raw": h.body_raw,
            }))),
            WebFetchResult::Error(e) => Err(map_harness(e.error)),
        }
    }
}
