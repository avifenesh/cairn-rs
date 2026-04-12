//! GitHub REST API client for code operations.
//!
//! All operations use an installation access token (auto-refreshed).
//! The client is stateless — it doesn't cache repo state.

use serde::{Deserialize, Serialize};

use crate::auth::InstallationToken;
use crate::error::GitHubError;

const API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "cairn-github/0.1";
const API_VERSION: &str = "2022-11-28";

/// GitHub REST API client authenticated via an installation access token.
#[derive(Clone, Debug)]
pub struct GitHubClient {
    token: InstallationToken,
    http: reqwest::Client,
}

impl GitHubClient {
    pub fn new(token: InstallationToken) -> Self {
        Self {
            http: reqwest::Client::new(),
            token,
        }
    }

    pub fn with_http(token: InstallationToken, http: reqwest::Client) -> Self {
        Self { http, token }
    }

    // ── Issues ──────────────────────────────────────────────────────────────

    /// Get a single issue by number.
    pub async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<IssueResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/issues/{number}");
        self.get_json(&url).await
    }

    /// List issues for a repo with optional filters.
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
        labels: Option<&str>,
        per_page: u32,
    ) -> Result<Vec<IssueResponse>, GitHubError> {
        let mut url = format!("{API_BASE}/repos/{owner}/{repo}/issues?per_page={per_page}");
        if let Some(state) = state {
            url.push_str(&format!("&state={state}"));
        }
        if let Some(labels) = labels {
            url.push_str(&format!("&labels={labels}"));
        }
        self.get_json(&url).await
    }

    /// Post a comment on an issue or pull request.
    pub async fn create_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<CommentResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/comments");
        self.post_json(&url, &serde_json::json!({ "body": body }))
            .await
    }

    /// Add labels to an issue.
    pub async fn add_labels(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: &[&str],
    ) -> Result<Vec<LabelResponse>, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/labels");
        self.post_json(&url, &serde_json::json!({ "labels": labels }))
            .await
    }

    // ── Branches ────────────────────────────────────────────────────────────

    /// Get a branch reference (returns the SHA).
    pub async fn get_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<GitRefResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/git/ref/heads/{branch}");
        self.get_json(&url).await
    }

    /// Create a new branch from a SHA.
    pub async fn create_branch(
        &self,
        owner: &str,
        repo: &str,
        branch_name: &str,
        from_sha: &str,
    ) -> Result<GitRefResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/git/refs");
        let body = serde_json::json!({
            "ref": format!("refs/heads/{branch_name}"),
            "sha": from_sha,
        });
        self.post_json(&url, &body).await
    }

    // ── Files ───────────────────────────────────────────────────────────────

    /// Get the contents of a file (base64-encoded for binary, UTF-8 for text).
    pub async fn get_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        branch: Option<&str>,
    ) -> Result<FileContentResponse, GitHubError> {
        let mut url = format!("{API_BASE}/repos/{owner}/{repo}/contents/{path}");
        if let Some(branch) = branch {
            url.push_str(&format!("?ref={branch}"));
        }
        self.get_json(&url).await
    }

    /// Create or update a file in the repo.
    pub async fn put_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        content: &str,
        message: &str,
        branch: &str,
        sha: Option<&str>,
    ) -> Result<FileUpdateResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/contents/{path}");
        let encoded = base64_encode(content.as_bytes());
        let mut body = serde_json::json!({
            "message": message,
            "content": encoded,
            "branch": branch,
        });
        if let Some(sha) = sha {
            body["sha"] = serde_json::Value::String(sha.to_owned());
        }
        self.put_json(&url, &body).await
    }

    /// Delete a file in the repo.
    pub async fn delete_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        message: &str,
        sha: &str,
        branch: &str,
    ) -> Result<serde_json::Value, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/contents/{path}");
        let body = serde_json::json!({
            "message": message,
            "sha": sha,
            "branch": branch,
        });
        let token = self.token.get().await?;
        let resp = self
            .http
            .delete(&url)
            .headers(self.default_headers(&token))
            .json(&body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    // ── Trees & Commits (batch file operations) ─────────────────────────────

    /// Create a tree with multiple file changes in one API call.
    pub async fn create_tree(
        &self,
        owner: &str,
        repo: &str,
        base_tree: &str,
        items: &[TreeItem],
    ) -> Result<TreeResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/git/trees");
        let body = serde_json::json!({
            "base_tree": base_tree,
            "tree": items,
        });
        self.post_json(&url, &body).await
    }

    /// Create a commit pointing to a tree.
    pub async fn create_commit(
        &self,
        owner: &str,
        repo: &str,
        message: &str,
        tree_sha: &str,
        parent_shas: &[&str],
    ) -> Result<CommitResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/git/commits");
        let body = serde_json::json!({
            "message": message,
            "tree": tree_sha,
            "parents": parent_shas,
        });
        self.post_json(&url, &body).await
    }

    /// Update a branch ref to point to a new commit.
    pub async fn update_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<GitRefResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/git/refs/heads/{branch}");
        let body = serde_json::json!({ "sha": sha });
        self.patch_json(&url, &body).await
    }

    // ── Pull Requests ───────────────────────────────────────────────────────

    /// Create a pull request.
    pub async fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<PullRequestResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/pulls");
        let payload = serde_json::json!({
            "title": title,
            "body": body,
            "head": head,
            "base": base,
        });
        self.post_json(&url, &payload).await
    }

    /// Merge a pull request.
    pub async fn merge_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        commit_title: Option<&str>,
        merge_method: Option<&str>,
    ) -> Result<MergeResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/pulls/{number}/merge");
        let mut body = serde_json::json!({});
        if let Some(title) = commit_title {
            body["commit_title"] = serde_json::Value::String(title.to_owned());
        }
        if let Some(method) = merge_method {
            body["merge_method"] = serde_json::Value::String(method.to_owned());
        }
        self.put_json(&url, &body).await
    }

    /// Close a pull request without merging.
    pub async fn close_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<PullRequestResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/pulls/{number}");
        let body = serde_json::json!({ "state": "closed" });
        self.patch_json(&url, &body).await
    }

    // ── Repository info ─────────────────────────────────────────────────────

    /// Get repository metadata.
    pub async fn get_repo(&self, owner: &str, repo: &str) -> Result<RepoResponse, GitHubError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}");
        self.get_json(&url).await
    }

    /// List repo directory contents.
    pub async fn list_contents(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        branch: Option<&str>,
    ) -> Result<Vec<ContentEntry>, GitHubError> {
        let mut url = format!("{API_BASE}/repos/{owner}/{repo}/contents/{path}");
        if let Some(branch) = branch {
            url.push_str(&format!("?ref={branch}"));
        }
        self.get_json(&url).await
    }

    // ── HTTP helpers ────────────────────────────────────────────────────────

    fn default_headers(&self, token: &str) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Authorization", format!("Bearer {token}").parse().unwrap());
        headers.insert("Accept", "application/vnd.github+json".parse().unwrap());
        headers.insert("User-Agent", USER_AGENT.parse().unwrap());
        headers.insert("X-GitHub-Api-Version", API_VERSION.parse().unwrap());
        headers
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, GitHubError> {
        let token = self.token.get().await?;
        let resp = self
            .http
            .get(url)
            .headers(self.default_headers(&token))
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<T, GitHubError> {
        let token = self.token.get().await?;
        let resp = self
            .http
            .post(url)
            .headers(self.default_headers(&token))
            .json(body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn put_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<T, GitHubError> {
        let token = self.token.get().await?;
        let resp = self
            .http
            .put(url)
            .headers(self.default_headers(&token))
            .json(body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn patch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<T, GitHubError> {
        let token = self.token.get().await?;
        let resp = self
            .http
            .patch(url)
            .headers(self.default_headers(&token))
            .json(body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn handle_response<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, GitHubError> {
        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api { status, body });
        }
        Ok(resp.json().await?)
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssueResponse {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    pub user: UserResponse,
    #[serde(default)]
    pub labels: Vec<LabelResponse>,
    pub html_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommentResponse {
    pub id: u64,
    pub body: String,
    pub html_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LabelResponse {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitRefResponse {
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub object: GitObject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitObject {
    pub sha: String,
    #[serde(rename = "type")]
    pub object_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileContentResponse {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(default)]
    pub content: Option<String>,
    pub encoding: Option<String>,
    #[serde(rename = "type")]
    pub content_type: String,
}

impl FileContentResponse {
    /// Decode the base64 content to a UTF-8 string.
    pub fn decode_content(&self) -> Option<String> {
        use base64::Engine;
        let raw = self.content.as_ref()?;
        let cleaned: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&cleaned)
            .ok()?;
        String::from_utf8(bytes).ok()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileUpdateResponse {
    pub content: ContentEntry,
    pub commit: CommitSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentEntry {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(rename = "type")]
    pub content_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitSummary {
    pub sha: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeItem {
    pub path: String,
    pub mode: String,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl TreeItem {
    /// Create a tree item for a new/modified file with inline content.
    pub fn file(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            mode: "100644".to_owned(),
            item_type: "blob".to_owned(),
            sha: None,
            content: Some(content.into()),
        }
    }

    /// Create a tree item that deletes a file (null sha).
    pub fn delete(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            mode: "100644".to_owned(),
            item_type: "blob".to_owned(),
            sha: Some("null".to_owned()), // GitHub interprets this as deletion
            content: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeResponse {
    pub sha: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitResponse {
    pub sha: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequestResponse {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub state: String,
    #[serde(default)]
    pub merged: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeResponse {
    pub sha: String,
    pub merged: bool,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoResponse {
    pub full_name: String,
    pub default_branch: String,
    #[serde(default)]
    pub private: bool,
    pub html_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserResponse {
    pub login: String,
    pub id: u64,
}
