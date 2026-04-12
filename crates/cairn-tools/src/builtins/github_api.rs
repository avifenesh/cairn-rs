//! GitHub API tools — use GitHub App installation tokens instead of `gh` CLI.
//!
//! These tools wrap `cairn_github::GitHubClient` operations for use in
//! the orchestrator. They are Deferred-tier and discovered via `tool_search`.
//!
//! Unlike the `gh` CLI tools in `github.rs`, these require no local CLI
//! installation — they use the App's installation token directly.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::recovery::RetrySafety;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;
use tokio::sync::RwLock;

use super::{
    PermissionLevel, ToolCategory, ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier,
};

/// Shared GitHub client provider — injected at registration time.
///
/// The tools call `get()` to obtain a client for the configured installation.
/// This avoids each tool needing to know about App credentials or installation IDs.
#[derive(Default)]
pub struct GitHubClientProvider {
    client: RwLock<Option<cairn_github::GitHubClient>>,
}

impl GitHubClientProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_client(client: cairn_github::GitHubClient) -> Self {
        Self {
            client: RwLock::new(Some(client)),
        }
    }

    pub async fn set(&self, client: cairn_github::GitHubClient) {
        *self.client.write().await = Some(client);
    }

    pub async fn get(&self) -> Result<cairn_github::GitHubClient, ToolError> {
        self.client
            .read()
            .await
            .clone()
            .ok_or_else(|| ToolError::Permanent("GitHub App not configured".into()))
    }
}

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    args[field].as_str().ok_or_else(|| ToolError::InvalidArgs {
        field: field.into(),
        message: "required string".into(),
    })
}

fn split_repo(repo: &str) -> Result<(&str, &str), ToolError> {
    repo.split_once('/').ok_or_else(|| ToolError::InvalidArgs {
        field: "repo".into(),
        message: "must be owner/repo format".into(),
    })
}

// ── github_api.create_branch ─────────────────────────────────────────────────

pub struct GhApiCreateBranchTool {
    provider: Arc<GitHubClientProvider>,
}

impl GhApiCreateBranchTool {
    pub fn new(provider: Arc<GitHubClientProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ToolHandler for GhApiCreateBranchTool {
    fn name(&self) -> &str {
        "github_api.create_branch"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Deferred
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::External
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "Create a new branch in a GitHub repository from the default branch HEAD."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo", "branch_name"],
            "properties": {
                "repo":        { "type": "string", "description": "owner/repo" },
                "branch_name": { "type": "string", "description": "New branch name" },
                "from_branch": { "type": "string", "description": "Source branch (default: repo default branch)" }
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
        let client = self.provider.get().await?;
        let repo = require_str(&args, "repo")?;
        let branch_name = require_str(&args, "branch_name")?;
        let (owner, repo_name) = split_repo(repo)?;

        let from_branch = match args["from_branch"].as_str() {
            Some(b) => b.to_owned(),
            None => {
                let repo_info = client
                    .get_repo(owner, repo_name)
                    .await
                    .map_err(|e| ToolError::Transient(e.to_string()))?;
                repo_info.default_branch
            }
        };

        let base_ref = client
            .get_ref(owner, repo_name, &from_branch)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        let new_ref = client
            .create_branch(owner, repo_name, branch_name, &base_ref.object.sha)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        Ok(ToolResult::ok(serde_json::json!({
            "branch": branch_name,
            "sha": new_ref.object.sha,
            "from": from_branch,
        })))
    }
}

// ── github_api.write_file ────────────────────────────────────────────────────

pub struct GhApiWriteFileTool {
    provider: Arc<GitHubClientProvider>,
}

impl GhApiWriteFileTool {
    pub fn new(provider: Arc<GitHubClientProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ToolHandler for GhApiWriteFileTool {
    fn name(&self) -> &str {
        "github_api.write_file"
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
        "Create or update a file in a GitHub repository. Commits directly to the specified branch."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo", "path", "content", "message", "branch"],
            "properties": {
                "repo":    { "type": "string", "description": "owner/repo" },
                "path":    { "type": "string", "description": "File path in the repo" },
                "content": { "type": "string", "description": "File content (UTF-8)" },
                "message": { "type": "string", "description": "Commit message" },
                "branch":  { "type": "string", "description": "Target branch" }
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
        let client = self.provider.get().await?;
        let repo = require_str(&args, "repo")?;
        let path = require_str(&args, "path")?;
        let content = require_str(&args, "content")?;
        let message = require_str(&args, "message")?;
        let branch = require_str(&args, "branch")?;
        let (owner, repo_name) = split_repo(repo)?;

        // Check if file exists to get its SHA (needed for updates).
        let existing_sha = match client.get_file(owner, repo_name, path, Some(branch)).await {
            Ok(f) => Some(f.sha),
            Err(_) => None,
        };

        let result = client
            .put_file(
                owner,
                repo_name,
                path,
                content,
                message,
                branch,
                existing_sha.as_deref(),
            )
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        Ok(ToolResult::ok(serde_json::json!({
            "path": result.content.path,
            "sha": result.commit.sha,
            "message": result.commit.message,
        })))
    }
}

// ── github_api.read_file ─────────────────────────────────────────────────────

pub struct GhApiReadFileTool {
    provider: Arc<GitHubClientProvider>,
}

impl GhApiReadFileTool {
    pub fn new(provider: Arc<GitHubClientProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ToolHandler for GhApiReadFileTool {
    fn name(&self) -> &str {
        "github_api.read_file"
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
        "Read a file from a GitHub repository. Returns decoded UTF-8 content."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo", "path"],
            "properties": {
                "repo":   { "type": "string", "description": "owner/repo" },
                "path":   { "type": "string", "description": "File path in the repo" },
                "branch": { "type": "string", "description": "Branch (default: repo default)" }
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
        let client = self.provider.get().await?;
        let repo = require_str(&args, "repo")?;
        let path = require_str(&args, "path")?;
        let branch = args["branch"].as_str();
        let (owner, repo_name) = split_repo(repo)?;

        let file = client
            .get_file(owner, repo_name, path, branch)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        let content = file
            .decode_content()
            .unwrap_or_else(|| "[binary or empty file]".to_owned());

        // Truncate for LLM context.
        let truncated = if content.len() > 8000 {
            format!("{}...[truncated at 8000 chars]", &content[..8000])
        } else {
            content
        };

        Ok(ToolResult::ok(serde_json::json!({
            "path": file.path,
            "sha": file.sha,
            "content": truncated,
        })))
    }
}

// ── github_api.create_pr ─────────────────────────────────────────────────────

pub struct GhApiCreatePrTool {
    provider: Arc<GitHubClientProvider>,
}

impl GhApiCreatePrTool {
    pub fn new(provider: Arc<GitHubClientProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ToolHandler for GhApiCreatePrTool {
    fn name(&self) -> &str {
        "github_api.create_pr"
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
        "Create a pull request in a GitHub repository."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo", "title", "head", "base"],
            "properties": {
                "repo":  { "type": "string", "description": "owner/repo" },
                "title": { "type": "string", "description": "PR title" },
                "body":  { "type": "string", "description": "PR description (markdown)" },
                "head":  { "type": "string", "description": "Head branch (with changes)" },
                "base":  { "type": "string", "description": "Base branch to merge into" }
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
        let client = self.provider.get().await?;
        let repo = require_str(&args, "repo")?;
        let title = require_str(&args, "title")?;
        let body = args["body"].as_str().unwrap_or("");
        let head = require_str(&args, "head")?;
        let base = require_str(&args, "base")?;
        let (owner, repo_name) = split_repo(repo)?;

        let pr = client
            .create_pull_request(owner, repo_name, title, body, head, base)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        Ok(ToolResult::ok(serde_json::json!({
            "number": pr.number,
            "title": pr.title,
            "url": pr.html_url,
            "state": pr.state,
        })))
    }
}

// ── github_api.merge_pr ──────────────────────────────────────────────────────

pub struct GhApiMergePrTool {
    provider: Arc<GitHubClientProvider>,
}

impl GhApiMergePrTool {
    pub fn new(provider: Arc<GitHubClientProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ToolHandler for GhApiMergePrTool {
    fn name(&self) -> &str {
        "github_api.merge_pr"
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
        "Merge a pull request. SENSITIVE — should only be called after operator approval."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo", "number"],
            "properties": {
                "repo":         { "type": "string", "description": "owner/repo" },
                "number":       { "type": "integer", "description": "PR number" },
                "merge_method": { "type": "string", "enum": ["merge", "squash", "rebase"], "default": "squash" }
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
        let client = self.provider.get().await?;
        let repo = require_str(&args, "repo")?;
        let number = args["number"]
            .as_u64()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "number".into(),
                message: "required integer".into(),
            })?;
        let merge_method = args["merge_method"].as_str().unwrap_or("squash");
        let (owner, repo_name) = split_repo(repo)?;

        let result = client
            .merge_pull_request(owner, repo_name, number, None, Some(merge_method))
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        Ok(ToolResult::ok(serde_json::json!({
            "merged": result.merged,
            "sha": result.sha,
            "message": result.message,
        })))
    }
}

// ── github_api.list_contents ─────────────────────────────────────────────────

pub struct GhApiListContentsTool {
    provider: Arc<GitHubClientProvider>,
}

impl GhApiListContentsTool {
    pub fn new(provider: Arc<GitHubClientProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ToolHandler for GhApiListContentsTool {
    fn name(&self) -> &str {
        "github_api.list_contents"
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
        "List files and directories in a GitHub repository path."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["repo"],
            "properties": {
                "repo":   { "type": "string", "description": "owner/repo" },
                "path":   { "type": "string", "description": "Directory path (default: root)", "default": "" },
                "branch": { "type": "string", "description": "Branch (default: repo default)" }
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
        let client = self.provider.get().await?;
        let repo = require_str(&args, "repo")?;
        let path = args["path"].as_str().unwrap_or("");
        let branch = args["branch"].as_str();
        let (owner, repo_name) = split_repo(repo)?;

        let entries = client
            .list_contents(owner, repo_name, path, branch)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        let items: Vec<Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "name": e.name,
                    "path": e.path,
                    "type": e.content_type,
                })
            })
            .collect();

        Ok(ToolResult::ok(serde_json::json!({
            "path": path,
            "entries": items,
            "count": items.len(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_api_tools_are_deferred_tier() {
        let provider = Arc::new(GitHubClientProvider::new());
        assert_eq!(
            GhApiCreateBranchTool::new(provider.clone()).tier(),
            ToolTier::Deferred
        );
        assert_eq!(
            GhApiWriteFileTool::new(provider.clone()).tier(),
            ToolTier::Deferred
        );
        assert_eq!(
            GhApiReadFileTool::new(provider.clone()).tier(),
            ToolTier::Deferred
        );
        assert_eq!(
            GhApiCreatePrTool::new(provider.clone()).tier(),
            ToolTier::Deferred
        );
        assert_eq!(
            GhApiMergePrTool::new(provider.clone()).tier(),
            ToolTier::Deferred
        );
        assert_eq!(
            GhApiListContentsTool::new(provider).tier(),
            ToolTier::Deferred
        );
    }

    #[test]
    fn write_tools_are_sensitive() {
        let provider = Arc::new(GitHubClientProvider::new());
        assert_eq!(
            GhApiCreateBranchTool::new(provider.clone()).execution_class(),
            ExecutionClass::Sensitive
        );
        assert_eq!(
            GhApiWriteFileTool::new(provider.clone()).execution_class(),
            ExecutionClass::Sensitive
        );
        assert_eq!(
            GhApiCreatePrTool::new(provider.clone()).execution_class(),
            ExecutionClass::Sensitive
        );
        assert_eq!(
            GhApiMergePrTool::new(provider).execution_class(),
            ExecutionClass::Sensitive
        );
    }

    #[test]
    fn read_tools_are_read_only() {
        let provider = Arc::new(GitHubClientProvider::new());
        assert_eq!(
            GhApiReadFileTool::new(provider.clone()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            GhApiListContentsTool::new(provider).permission_level(),
            PermissionLevel::ReadOnly
        );
    }
}
