//! GitHub webhook signature verification and event parsing.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::GitHubError;

type HmacSha256 = Hmac<Sha256>;

/// Verify the HMAC-SHA256 signature of a GitHub webhook payload.
///
/// `signature` is the `X-Hub-Signature-256` header value (format: `sha256=<hex>`).
/// `secret` is the webhook secret configured in the GitHub App.
/// `body` is the raw request body bytes.
pub fn verify_signature(signature: &str, secret: &[u8], body: &[u8]) -> Result<(), GitHubError> {
    let hex_sig = signature
        .strip_prefix("sha256=")
        .ok_or(GitHubError::InvalidSignature)?;

    let expected = hex_decode(hex_sig).ok_or(GitHubError::InvalidSignature)?;

    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| GitHubError::InvalidSignature)?;
    mac.update(body);

    mac.verify_slice(&expected)
        .map_err(|_| GitHubError::InvalidSignature)
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

// ── Webhook event types ─────────────────────────────────────────────────────

/// Parsed GitHub webhook event.
#[derive(Clone, Debug)]
pub struct WebhookEvent {
    /// The event type from `X-GitHub-Event` header.
    pub event_type: String,
    /// The delivery ID from `X-GitHub-Delivery` header.
    pub delivery_id: String,
    /// Parsed event payload.
    pub payload: WebhookEventPayload,
}

/// Supported webhook event payloads.
///
/// Events not explicitly handled are captured as `Other` with raw JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WebhookEventPayload {
    Issues(IssuesEvent),
    IssueComment(IssueCommentEvent),
    PullRequest(PullRequestEvent),
    PullRequestReview(PullRequestReviewEvent),
    Push(PushEvent),
    Other(serde_json::Value),
}

impl WebhookEvent {
    /// Parse a webhook event from headers and raw JSON body.
    pub fn parse(event_type: &str, delivery_id: &str, body: &[u8]) -> Result<Self, GitHubError> {
        let payload = match event_type {
            "issues" => {
                let evt: IssuesEvent = serde_json::from_slice(body)?;
                WebhookEventPayload::Issues(evt)
            }
            "issue_comment" => {
                let evt: IssueCommentEvent = serde_json::from_slice(body)?;
                WebhookEventPayload::IssueComment(evt)
            }
            "pull_request" => {
                let evt: PullRequestEvent = serde_json::from_slice(body)?;
                WebhookEventPayload::PullRequest(evt)
            }
            "pull_request_review" => {
                let evt: PullRequestReviewEvent = serde_json::from_slice(body)?;
                WebhookEventPayload::PullRequestReview(evt)
            }
            "push" => {
                let evt: PushEvent = serde_json::from_slice(body)?;
                WebhookEventPayload::Push(evt)
            }
            _ => {
                let val: serde_json::Value = serde_json::from_slice(body)?;
                WebhookEventPayload::Other(val)
            }
        };

        Ok(Self {
            event_type: event_type.to_owned(),
            delivery_id: delivery_id.to_owned(),
            payload,
        })
    }

    /// Returns the action field (e.g., "opened", "labeled", "created").
    pub fn action(&self) -> Option<&str> {
        match &self.payload {
            WebhookEventPayload::Issues(e) => Some(&e.action),
            WebhookEventPayload::IssueComment(e) => Some(&e.action),
            WebhookEventPayload::PullRequest(e) => Some(&e.action),
            WebhookEventPayload::PullRequestReview(e) => Some(&e.action),
            WebhookEventPayload::Push(_) => None,
            WebhookEventPayload::Other(v) => v.get("action")?.as_str(),
        }
    }

    /// Returns a compound event key like "issues.opened" or "push".
    pub fn event_key(&self) -> String {
        match self.action() {
            Some(action) => format!("{}.{}", self.event_type, action),
            None => self.event_type.clone(),
        }
    }

    /// Extract the repository full name (owner/repo).
    pub fn repository(&self) -> Option<&str> {
        match &self.payload {
            WebhookEventPayload::Issues(e) => Some(&e.repository.full_name),
            WebhookEventPayload::IssueComment(e) => Some(&e.repository.full_name),
            WebhookEventPayload::PullRequest(e) => Some(&e.repository.full_name),
            WebhookEventPayload::PullRequestReview(e) => Some(&e.repository.full_name),
            WebhookEventPayload::Push(e) => Some(&e.repository.full_name),
            WebhookEventPayload::Other(v) => v.get("repository")?.get("full_name")?.as_str(),
        }
    }

    /// Extract the installation ID from the webhook payload.
    pub fn installation_id(&self) -> Option<u64> {
        let val = match &self.payload {
            WebhookEventPayload::Issues(e) => e.installation.as_ref()?.id,
            WebhookEventPayload::IssueComment(e) => e.installation.as_ref()?.id,
            WebhookEventPayload::PullRequest(e) => e.installation.as_ref()?.id,
            WebhookEventPayload::PullRequestReview(e) => e.installation.as_ref()?.id,
            WebhookEventPayload::Push(e) => e.installation.as_ref()?.id,
            WebhookEventPayload::Other(v) => v.get("installation")?.get("id")?.as_u64()?,
        };
        Some(val)
    }
}

// ── Event payload types ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuesEvent {
    pub action: String,
    pub issue: Issue,
    pub repository: Repository,
    pub sender: User,
    #[serde(default)]
    pub installation: Option<Installation>,
    #[serde(default)]
    pub label: Option<Label>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssueCommentEvent {
    pub action: String,
    pub issue: Issue,
    pub comment: Comment,
    pub repository: Repository,
    pub sender: User,
    #[serde(default)]
    pub installation: Option<Installation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequestEvent {
    pub action: String,
    pub pull_request: PullRequest,
    pub repository: Repository,
    pub sender: User,
    #[serde(default)]
    pub installation: Option<Installation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequestReviewEvent {
    pub action: String,
    pub review: Review,
    pub pull_request: PullRequest,
    pub repository: Repository,
    pub sender: User,
    #[serde(default)]
    pub installation: Option<Installation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushEvent {
    #[serde(rename = "ref")]
    pub git_ref: String,
    #[serde(default)]
    pub commits: Vec<PushCommit>,
    pub repository: Repository,
    pub sender: User,
    #[serde(default)]
    pub installation: Option<Installation>,
}

// ── Shared sub-types ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    pub user: User,
    #[serde(default)]
    pub labels: Vec<Label>,
    pub html_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    pub user: User,
    pub html_url: String,
    #[serde(default)]
    pub head: Option<GitRef>,
    #[serde(default)]
    pub base: Option<GitRef>,
    #[serde(default)]
    pub merged: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitRef {
    pub label: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub sha: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub body: String,
    pub user: User,
    pub html_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub id: u64,
    pub state: String,
    #[serde(default)]
    pub body: Option<String>,
    pub user: User,
    pub html_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repository {
    pub full_name: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub login: String,
    pub id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installation {
    pub id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushCommit {
    pub id: String,
    pub message: String,
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub modified: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_signature_valid() {
        let secret = b"test-secret";
        let body = b"hello world";

        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let hex: String = result.iter().map(|b| format!("{b:02x}")).collect();
        let header = format!("sha256={hex}");

        assert!(verify_signature(&header, secret, body).is_ok());
    }

    #[test]
    fn verify_signature_invalid() {
        assert!(verify_signature("sha256=deadbeef", b"secret", b"body").is_err());
    }

    #[test]
    fn verify_signature_missing_prefix() {
        assert!(verify_signature("invalid-header", b"secret", b"body").is_err());
    }

    #[test]
    fn parse_issues_opened() {
        let body = serde_json::json!({
            "action": "opened",
            "issue": {
                "number": 42,
                "title": "Fix the login bug",
                "body": "The login page crashes when...",
                "state": "open",
                "user": { "login": "alice", "id": 1 },
                "labels": [{ "name": "bug", "color": "d73a4a" }],
                "html_url": "https://github.com/org/repo/issues/42"
            },
            "repository": {
                "full_name": "org/repo",
                "default_branch": "main"
            },
            "sender": { "login": "alice", "id": 1 },
            "installation": { "id": 12345 }
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let event = WebhookEvent::parse("issues", "delivery-1", &bytes).unwrap();
        assert_eq!(event.event_key(), "issues.opened");
        assert_eq!(event.repository(), Some("org/repo"));
        assert_eq!(event.installation_id(), Some(12345));

        if let WebhookEventPayload::Issues(e) = &event.payload {
            assert_eq!(e.issue.number, 42);
            assert_eq!(e.issue.title, "Fix the login bug");
            assert_eq!(e.issue.labels[0].name, "bug");
        } else {
            panic!("expected Issues payload");
        }
    }

    #[test]
    fn parse_unknown_event_as_other() {
        let body = serde_json::json!({
            "action": "completed",
            "repository": { "full_name": "org/repo" },
            "installation": { "id": 99 }
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let event = WebhookEvent::parse("check_run", "delivery-2", &bytes).unwrap();
        assert_eq!(event.event_key(), "check_run.completed");
        assert_eq!(event.installation_id(), Some(99));
        assert!(matches!(event.payload, WebhookEventPayload::Other(_)));
    }

    #[test]
    fn event_key_for_push() {
        let body = serde_json::json!({
            "ref": "refs/heads/main",
            "commits": [],
            "repository": { "full_name": "org/repo" },
            "sender": { "login": "bot", "id": 2 }
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let event = WebhookEvent::parse("push", "delivery-3", &bytes).unwrap();
        assert_eq!(event.event_key(), "push"); // no action for push events
    }
}
