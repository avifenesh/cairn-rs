//! GitHub integration plugin for Cairn.
//!
//! Implements the `Integration` trait using `cairn_github` for auth, webhooks,
//! and API operations. The agent prompt, tools, and event→action mappings are
//! all defaults that the operator can override.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::{
    EventAction, EventActionMapping, Integration, IntegrationError, IntegrationEvent, QueueStats,
    WorkItem, WorkItemStatus,
};

/// GitHub App integration plugin.
///
/// Holds credentials, installation token cache, work queue, and
/// concurrency controls. Created at startup when GITHUB_APP_ID +
/// GITHUB_PRIVATE_KEY_FILE + GITHUB_WEBHOOK_SECRET env vars are set.
pub struct GitHubPlugin {
    pub credentials: cairn_github::AppCredentials,
    pub webhook_secret: String,
    /// Map of installation_id → InstallationToken (auto-refreshing).
    pub installations: RwLock<HashMap<u64, cairn_github::InstallationToken>>,
    /// Work item queue for processing GitHub issues/PRs.
    pub queue: RwLock<VecDeque<WorkItem>>,
    /// Whether the queue dispatcher is paused by the operator.
    pub queue_paused: AtomicBool,
    /// Whether the queue dispatcher loop is currently running.
    pub queue_running: AtomicBool,
    /// Max concurrent orchestration runs (operator-configurable).
    pub max_concurrent: AtomicU32,
    /// Semaphore controlling concurrent run slots.
    pub run_semaphore: Arc<Semaphore>,
    pub http: reqwest::Client,
}

impl std::fmt::Debug for GitHubPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubPlugin")
            .field("app_id", &self.credentials.app_id)
            .finish()
    }
}

impl GitHubPlugin {
    /// Create a new GitHubPlugin with the given credentials and defaults.
    pub fn new(
        credentials: cairn_github::AppCredentials,
        webhook_secret: String,
        max_concurrent: u32,
    ) -> Self {
        Self {
            credentials,
            webhook_secret,
            installations: RwLock::new(HashMap::new()),
            queue: RwLock::new(VecDeque::new()),
            queue_paused: AtomicBool::new(false),
            queue_running: AtomicBool::new(false),
            max_concurrent: AtomicU32::new(max_concurrent),
            run_semaphore: Arc::new(Semaphore::new(max_concurrent as usize)),
            http: reqwest::Client::new(),
        }
    }

    /// Create a GitHubPlugin from a config payload (runtime API).
    pub fn from_config(
        _id: &str,
        config: crate::config::GitHubConfig,
    ) -> Result<Self, crate::IntegrationError> {
        let pem_bytes = std::fs::read(&config.private_key_file).map_err(|e| {
            crate::IntegrationError::KeyFormatInvalid(format!(
                "cannot read private key file {}: {e}",
                config.private_key_file
            ))
        })?;
        let credentials =
            cairn_github::AppCredentials::new(config.app_id, &pem_bytes).map_err(|e| {
                crate::IntegrationError::KeyFormatInvalid(format!("invalid GitHub App key: {e}"))
            })?;
        Ok(Self::new(
            credentials,
            config.webhook_secret,
            config.max_concurrent,
        ))
    }

    /// Get or create an InstallationToken for the given installation ID.
    pub async fn token_for_installation(
        &self,
        installation_id: u64,
    ) -> cairn_github::InstallationToken {
        {
            let cache = self.installations.read().await;
            if let Some(token) = cache.get(&installation_id) {
                return token.clone();
            }
        }
        let token = cairn_github::InstallationToken::new(
            self.credentials.clone(),
            installation_id,
            self.http.clone(),
        );
        let mut cache = self.installations.write().await;
        cache.insert(installation_id, token.clone());
        token
    }

    /// Get a GitHubClient for the given installation.
    pub async fn client_for_installation(
        &self,
        installation_id: u64,
    ) -> cairn_github::GitHubClient {
        let token = self.token_for_installation(installation_id).await;
        cairn_github::GitHubClient::with_http(token, self.http.clone())
    }

    /// Check if a webhook event key matches a pattern (supports `*` wildcard).
    pub fn event_matches(event_key: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        if let Some(prefix) = pattern.strip_suffix(".*") {
            event_key.starts_with(prefix) && event_key.len() > prefix.len()
        } else {
            event_key == pattern
        }
    }
}

/// Default agent prompt for GitHub issue→PR agents.
///
/// Based on research from SWE-agent, OpenHands, Aider, Claude Code,
/// Cursor, and Devin. See `docs/skills/system-prompt-curator/SKILL.md`.
const DEFAULT_GITHUB_AGENT_PROMPT: &str = "\
You are a senior software engineer working autonomously. You have been \
assigned a GitHub issue and must resolve it by writing code and opening \
a pull request.\n\
\n\
Follow the workflow in the goal description. Use tool_search to discover \
available tools. Explore the codebase before writing code. Write real, \
working code — not descriptions or TODO comments. Verify your changes \
compile and tests pass before opening the PR.\n\
\n\
Do not call complete_run until you have opened a pull request.";

#[async_trait]
impl Integration for GitHubPlugin {
    fn id(&self) -> &str {
        "github"
    }

    fn display_name(&self) -> &str {
        "GitHub"
    }

    fn is_configured(&self) -> bool {
        true // If this struct exists, credentials were valid at startup.
    }

    fn default_agent_prompt(&self) -> &str {
        DEFAULT_GITHUB_AGENT_PROMPT
    }

    fn default_event_actions(&self) -> Vec<EventActionMapping> {
        vec![
            EventActionMapping {
                event_pattern: "issues.opened".into(),
                label_filter: Some("cairn".into()),
                repo_filter: None,
                action: EventAction::CreateAndOrchestrate,
            },
            EventActionMapping {
                event_pattern: "issues.labeled".into(),
                label_filter: Some("cairn".into()),
                repo_filter: None,
                action: EventAction::CreateAndOrchestrate,
            },
        ]
    }

    async fn verify_webhook(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<(), IntegrationError> {
        let sig = headers
            .get("X-Hub-Signature-256")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                IntegrationError::VerificationFailed("missing X-Hub-Signature-256".into())
            })?;
        cairn_github::verify_signature(sig, self.webhook_secret.as_bytes(), body).map_err(|e| {
            IntegrationError::VerificationFailed(format!("HMAC verification failed: {e}"))
        })
    }

    async fn parse_event(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<IntegrationEvent, IntegrationError> {
        let event_type = headers
            .get("X-GitHub-Event")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        let delivery_id = headers
            .get("X-GitHub-Delivery")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let event = cairn_github::WebhookEvent::parse(event_type, delivery_id, body)
            .map_err(|e| IntegrationError::ParseError(e.to_string()))?;

        let installation_id = event
            .installation_id()
            .map(|id: u64| id.to_string())
            .unwrap_or_default();

        // Extract title, body, and labels from raw JSON (WebhookEvent doesn't
        // expose these directly — they live in the issue/PR payload).
        let raw: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        let issue_or_pr = raw.get("issue").or_else(|| raw.get("pull_request"));
        let title = issue_or_pr
            .and_then(|v| v["title"].as_str())
            .map(|s| s.to_owned());
        let body_text = issue_or_pr
            .and_then(|v| v["body"].as_str())
            .map(|s| s.to_owned());
        let labels = issue_or_pr
            .and_then(|v| v["labels"].as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l["name"].as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(IntegrationEvent {
            integration_id: "github".into(),
            event_key: event.event_key(),
            source_id: installation_id,
            repository: event.repository().map(|r: &str| r.to_owned()),
            title,
            body: body_text,
            labels,
            raw,
        })
    }

    async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError> {
        // Try to fetch the full issue from the GitHub API for richer context.
        let source_id: u64 = item
            .source_id
            .parse()
            .map_err(|_| IntegrationError::Other("invalid installation_id".into()))?;
        let (owner, repo_name) = item.repo.split_once('/').unwrap_or(("", &item.repo));
        let issue_number: u64 = item
            .external_id
            .parse()
            .map_err(|_| IntegrationError::Other("invalid issue number".into()))?;

        let client = self.client_for_installation(source_id).await;

        match client.get_issue(owner, repo_name, issue_number).await {
            Ok(issue) => {
                let body = issue.body.as_deref().unwrap_or("");
                Ok(format!(
                    "## Task\n\
                     Resolve GitHub Issue #{number} in repository `{repo}` by writing code \
                     and opening a pull request.\n\n\
                     ## Issue\n\
                     **{title}**\n\n\
                     {body}\n\n\
                     ## Workflow\n\
                     Follow these steps in order.\n\n\
                     1. **Explore** — Use tool_search to find available tools. Use file-reading \
                     and search tools to understand the repo structure and find relevant code. \
                     Read at least 3-5 files before planning changes.\n\n\
                     2. **Plan** — Identify which files need to change and what the fix or \
                     feature looks like. Think through edge cases.\n\n\
                     3. **Branch** — Create a feature branch (e.g. `cairn/issue-{number}`).\n\n\
                     4. **Implement** — Write the code. Make minimal, focused changes. Follow \
                     existing code style and conventions in the repo.\n\n\
                     5. **Verify** — If the project has tests, run them. Fix any failures.\n\n\
                     6. **Deliver** — Commit your changes, push the branch, and open a PR \
                     that references issue #{number} in the title or body.\n\n\
                     7. **Complete** — After the PR is open, call escalate_to_operator for \
                     review, then complete_run with a summary.\n\n\
                     ## Tips\n\
                     - Start by exploring. Do not write code until you understand the codebase.\n\
                     - If a tool call fails, read the error and try a different approach. \
                     A command that failed once will fail again unless you change something.\n\
                     - Write real, working code — not pseudocode or TODO comments.\n\
                     - Keep changes focused on this issue only.\n\
                     - All tool calls targeting this repo need: repo=\"{repo}\".\n\
                     - Do not call complete_run until you have opened a PR.",
                    number = issue.number,
                    repo = item.repo,
                    title = issue.title,
                    body = body,
                ))
            }
            Err(_) => Ok(format!(
                "Resolve GitHub Issue #{} in repository `{}`. \
                 Explore the codebase, write a fix, and open a pull request. \
                 Use tool_search to discover available tools.",
                issue_number, item.repo
            )),
        }
    }

    async fn prepare_tool_registry(
        &self,
        base: &cairn_tools::BuiltinToolRegistry,
        item: &WorkItem,
    ) -> Arc<cairn_tools::BuiltinToolRegistry> {
        use cairn_tools::builtins::github_api::*;

        let source_id: u64 = item.source_id.parse().unwrap_or(0);
        let gh_provider = Arc::new(GitHubClientProvider::new());

        if source_id > 0 {
            let client = self.client_for_installation(source_id).await;
            gh_provider.set(client).await;
        }

        Arc::new(
            cairn_tools::BuiltinToolRegistry::from_existing(base)
                .register(Arc::new(GhApiCreateBranchTool::new(gh_provider.clone())))
                .register(Arc::new(GhApiReadFileTool::new(gh_provider.clone())))
                .register(Arc::new(GhApiWriteFileTool::new(gh_provider.clone())))
                .register(Arc::new(GhApiCreatePrTool::new(gh_provider.clone())))
                .register(Arc::new(GhApiMergePrTool::new(gh_provider.clone())))
                .register(Arc::new(GhApiListContentsTool::new(gh_provider))),
        )
    }

    fn auth_exempt_paths(&self) -> Vec<String> {
        vec!["/v1/webhooks/github".into()]
    }

    async fn queue_stats(&self) -> QueueStats {
        let queue = self.queue.read().await;
        let mut stats = QueueStats::default();
        for item in queue.iter() {
            match &item.status {
                WorkItemStatus::Pending => stats.pending += 1,
                WorkItemStatus::Processing => stats.processing += 1,
                WorkItemStatus::WaitingApproval => stats.waiting_approval += 1,
                WorkItemStatus::Completed => stats.completed += 1,
                WorkItemStatus::Failed(_) => stats.failed += 1,
                WorkItemStatus::Skipped => {}
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_matches_exact() {
        assert!(GitHubPlugin::event_matches(
            "issues.opened",
            "issues.opened"
        ));
        assert!(!GitHubPlugin::event_matches(
            "issues.closed",
            "issues.opened"
        ));
    }

    #[test]
    fn event_matches_wildcard() {
        assert!(GitHubPlugin::event_matches("issues.opened", "issues.*"));
        assert!(GitHubPlugin::event_matches("issues.closed", "issues.*"));
        assert!(!GitHubPlugin::event_matches("push", "issues.*"));
    }

    #[test]
    fn event_matches_star_all() {
        assert!(GitHubPlugin::event_matches("anything", "*"));
    }

    #[test]
    fn default_event_actions_are_cairn_label_only() {
        let plugin = make_test_plugin();
        let actions = plugin.default_event_actions();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].label_filter.as_deref(), Some("cairn"));
    }

    #[test]
    fn plugin_is_configured() {
        let plugin = make_test_plugin();
        assert!(plugin.is_configured());
        assert_eq!(plugin.id(), "github");
        assert_eq!(plugin.display_name(), "GitHub");
    }

    #[test]
    fn default_prompt_mentions_pull_request() {
        let plugin = make_test_plugin();
        assert!(plugin.default_agent_prompt().contains("pull request"));
    }

    #[test]
    fn auth_exempt_paths_include_webhook() {
        let plugin = make_test_plugin();
        let paths = plugin.auth_exempt_paths();
        assert!(paths.contains(&"/v1/webhooks/github".to_owned()));
    }

    #[tokio::test]
    async fn queue_stats_counts_correctly() {
        let plugin = make_test_plugin();
        {
            let mut queue = plugin.queue.write().await;
            queue.push_back(make_work_item("1", WorkItemStatus::Pending));
            queue.push_back(make_work_item("2", WorkItemStatus::Processing));
            queue.push_back(make_work_item("3", WorkItemStatus::Completed));
            queue.push_back(make_work_item("4", WorkItemStatus::Failed("err".into())));
        }
        let stats = plugin.queue_stats().await;
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.processing, 1);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 1);
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_test_plugin() -> GitHubPlugin {
        // Use dummy credentials — we won't call the API in unit tests.
        // AppCredentials::new requires a real RSA key, so we skip it
        // and test only the non-auth methods.
        GitHubPlugin {
            credentials: unsafe_test_credentials(),
            webhook_secret: "test-secret".into(),
            installations: RwLock::new(HashMap::new()),
            queue: RwLock::new(VecDeque::new()),
            queue_paused: AtomicBool::new(false),
            queue_running: AtomicBool::new(false),
            max_concurrent: AtomicU32::new(3),
            run_semaphore: Arc::new(Semaphore::new(3)),
            http: reqwest::Client::new(),
        }
    }

    fn make_work_item(id: &str, status: WorkItemStatus) -> WorkItem {
        WorkItem {
            integration_id: "github".into(),
            source_id: "123".into(),
            external_id: id.into(),
            repo: "owner/repo".into(),
            title: format!("Issue {id}"),
            body: String::new(),
            run_id: format!("run_{id}"),
            session_id: format!("sess_{id}"),
            status,
        }
    }

    /// Test-only: create AppCredentials using a generated RSA key.
    fn unsafe_test_credentials() -> cairn_github::AppCredentials {
        // Generate a minimal 2048-bit RSA key in PEM format for testing.
        // This is only used for struct construction — we don't call the GitHub API.
        let rsa_pem = include_bytes!("../tests/fixtures/test_rsa_key.pem");
        cairn_github::AppCredentials::new(12345, rsa_pem).expect("test RSA key should be valid")
    }
}
