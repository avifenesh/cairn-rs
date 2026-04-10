//! GitHub tools — Deferred-tier builtins that wrap the `gh` CLI.
//!
//! Discovered via `tool_search` when the agent needs GitHub integration.
//! Uses `gh` CLI subprocess — requires `gh auth login` on the host.

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;

use super::{
    PermissionLevel, ToolCategory, ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier,
};
use cairn_domain::recovery::RetrySafety;

async fn run_gh(args: &[&str]) -> Result<String, ToolError> {
    let output = tokio::process::Command::new("gh")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| ToolError::Transient(format!("gh CLI spawn failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::Permanent(format!("gh: {stderr}")));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    args[field].as_str().ok_or_else(|| ToolError::InvalidArgs {
        field: field.into(),
        message: "required string".into(),
    })
}

// ── gh_list_issues ──────────────────────────────────────────────────────────

pub struct GhListIssuesTool;

#[async_trait]
impl ToolHandler for GhListIssuesTool {
    fn name(&self) -> &str {
        "gh_list_issues"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Deferred
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "List GitHub issues for a repository. Returns JSON array with number, title, labels, state."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo"],
            "properties": {
                "repo":   { "type": "string", "description": "owner/repo (e.g. avifenesh/cairn-dogfood)" },
                "state":  { "type": "string", "enum": ["open", "closed", "all"], "default": "open" },
                "labels": { "type": "string", "description": "Comma-separated label filter" },
                "limit":  { "type": "integer", "default": 20, "description": "Max issues to return" }
            }
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Orchestration
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let repo = require_str(&args, "repo")?;
        let state = args["state"].as_str().unwrap_or("open");
        let limit = args["limit"].as_u64().unwrap_or(20).min(100);

        let mut gh_args = vec![
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            state,
            "--json",
            "number,title,labels,state,assignees,createdAt",
        ];
        let limit_str = limit.to_string();
        gh_args.extend_from_slice(&["--limit", &limit_str]);

        if let Some(labels) = args["labels"].as_str() {
            gh_args.extend_from_slice(&["--label", labels]);
        }

        let output = run_gh(&gh_args).await?;
        let parsed: Value =
            serde_json::from_str(&output).unwrap_or_else(|_| Value::String(output.clone()));

        Ok(ToolResult::ok(parsed))
    }
}

// ── gh_get_issue ────────────────────────────────────────────────────────────

pub struct GhGetIssueTool;

#[async_trait]
impl ToolHandler for GhGetIssueTool {
    fn name(&self) -> &str {
        "gh_get_issue"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Deferred
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "Get a single GitHub issue with full body and comments."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo", "number"],
            "properties": {
                "repo":   { "type": "string", "description": "owner/repo" },
                "number": { "type": "integer", "description": "Issue number" }
            }
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Orchestration
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let repo = require_str(&args, "repo")?;
        let number = args["number"]
            .as_u64()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "number".into(),
                message: "required integer".into(),
            })?;

        let num_str = number.to_string();
        let output = run_gh(&[
            "issue",
            "view",
            &num_str,
            "--repo",
            repo,
            "--json",
            "number,title,body,labels,state,assignees,comments,createdAt",
        ])
        .await?;

        let mut parsed: Value =
            serde_json::from_str(&output).unwrap_or_else(|_| Value::String(output.clone()));

        // Truncate body if too long (save LLM context)
        if let Some(body) = parsed.get("body").and_then(|b| b.as_str()) {
            if body.len() > 4000 {
                parsed["body"] = Value::String(format!("{}…[truncated]", &body[..4000]));
            }
        }

        // Truncate comments to last 5
        if let Some(comments) = parsed.get_mut("comments").and_then(|c| c.as_array_mut()) {
            if comments.len() > 5 {
                let last5: Vec<Value> = comments.iter().rev().take(5).rev().cloned().collect();
                *comments = last5;
            }
        }

        Ok(ToolResult::ok(parsed))
    }
}

// ── gh_create_comment ───────────────────────────────────────────────────────

pub struct GhCreateCommentTool;

#[async_trait]
impl ToolHandler for GhCreateCommentTool {
    fn name(&self) -> &str {
        "gh_create_comment"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Deferred
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::External
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::DangerousPause
    }
    fn description(&self) -> &str {
        "Add a comment to a GitHub issue. SENSITIVE — requires approval."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo", "number", "body"],
            "properties": {
                "repo":   { "type": "string", "description": "owner/repo" },
                "number": { "type": "integer", "description": "Issue number" },
                "body":   { "type": "string", "description": "Comment body (markdown)" }
            }
        })
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::Sensitive
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Orchestration
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let repo = require_str(&args, "repo")?;
        let number = args["number"]
            .as_u64()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "number".into(),
                message: "required integer".into(),
            })?;
        let body = require_str(&args, "body")?;

        let num_str = number.to_string();
        let output =
            run_gh(&["issue", "comment", &num_str, "--repo", repo, "--body", body]).await?;

        Ok(ToolResult::ok(serde_json::json!({
            "commented": true,
            "issue": number,
            "url": output.trim(),
        })))
    }
}

// ── gh_search_code ──────────────────────────────────────────────────────────

pub struct GhSearchCodeTool;

#[async_trait]
impl ToolHandler for GhSearchCodeTool {
    fn name(&self) -> &str {
        "gh_search_code"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Deferred
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "Search code across a GitHub repository."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string", "description": "Search query (GitHub code search syntax)" },
                "repo":  { "type": "string", "description": "Limit to owner/repo" },
                "limit": { "type": "integer", "default": 10 }
            }
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Orchestration
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let query = require_str(&args, "query")?;
        let limit = args["limit"].as_u64().unwrap_or(10).min(50);

        let mut search_query = query.to_owned();
        if let Some(repo) = args["repo"].as_str() {
            search_query = format!("{search_query} repo:{repo}");
        }

        let limit_str = limit.to_string();
        let output = run_gh(&[
            "search",
            "code",
            &search_query,
            "--json",
            "path,repository,textMatches",
            "--limit",
            &limit_str,
        ])
        .await?;

        let parsed: Value =
            serde_json::from_str(&output).unwrap_or_else(|_| Value::String(output.clone()));

        Ok(ToolResult::ok(parsed))
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_are_deferred_tier() {
        assert_eq!(GhListIssuesTool.tier(), ToolTier::Deferred);
        assert_eq!(GhGetIssueTool.tier(), ToolTier::Deferred);
        assert_eq!(GhCreateCommentTool.tier(), ToolTier::Deferred);
        assert_eq!(GhSearchCodeTool.tier(), ToolTier::Deferred);
    }

    #[test]
    fn read_tools_are_read_only() {
        assert_eq!(
            GhListIssuesTool.permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(GhGetIssueTool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(
            GhSearchCodeTool.permission_level(),
            PermissionLevel::ReadOnly
        );
    }

    #[test]
    fn write_tools_are_sensitive() {
        assert_eq!(
            GhCreateCommentTool.execution_class(),
            ExecutionClass::Sensitive
        );
        assert_eq!(
            GhCreateCommentTool.permission_level(),
            PermissionLevel::Execute
        );
    }

    #[test]
    fn tool_search_finds_github_tools() {
        use super::super::{BuiltinToolDescriptor, BuiltinToolRegistry};
        use std::sync::Arc;

        let reg = BuiltinToolRegistry::new()
            .register(Arc::new(GhListIssuesTool))
            .register(Arc::new(GhGetIssueTool))
            .register(Arc::new(GhCreateCommentTool))
            .register(Arc::new(GhSearchCodeTool));

        let found = reg.search_deferred("github");
        assert_eq!(found.len(), 4, "all 4 GH tools should match 'github'");

        let found = reg.search_deferred("issue");
        assert!(found.iter().any(|d| d.name == "gh_list_issues"));
        assert!(found.iter().any(|d| d.name == "gh_get_issue"));
        assert!(found.iter().any(|d| d.name == "gh_create_comment"));
    }
}
