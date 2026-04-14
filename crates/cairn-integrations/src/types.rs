//! Shared types for the integration plugin framework.

use serde::{Deserialize, Serialize};

/// A normalised event from any integration source (GitHub, Linear, Slack, etc.).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationEvent {
    /// Which integration produced this event (e.g. "github").
    pub integration_id: String,
    /// Compound event key (e.g. "issues.opened", "pull_request.closed").
    pub event_key: String,
    /// Source-specific identifier (e.g. GitHub installation_id).
    pub source_id: String,
    /// Repository or project this event relates to (e.g. "owner/repo").
    pub repository: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Vec<String>,
    /// The raw, unparsed event payload for integration-specific processing.
    pub raw: serde_json::Value,
}

/// A work item queued for agent processing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkItem {
    /// Which integration this work item came from.
    pub integration_id: String,
    /// Source-specific identifier (e.g. GitHub installation_id as string).
    pub source_id: String,
    /// External ID (e.g. issue number, ticket ID).
    pub external_id: String,
    /// Repository or project identifier.
    pub repo: String,
    pub title: String,
    pub body: String,
    /// Cairn run ID assigned to this work item.
    pub run_id: String,
    /// Cairn session ID.
    pub session_id: String,
    /// Current processing status.
    pub status: WorkItemStatus,
}

/// Processing status of a work item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Pending,
    Processing,
    WaitingApproval,
    Completed,
    Failed(String),
    Skipped,
}

/// Generic event→action mapping — shared across all integrations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventActionMapping {
    /// Event key pattern to match (e.g. "issues.opened", "issues.*").
    /// Supports "*" as wildcard.
    pub event_pattern: String,
    /// Only trigger if the event has this label.
    #[serde(default)]
    pub label_filter: Option<String>,
    /// Only trigger for this repository.
    #[serde(default)]
    pub repo_filter: Option<String>,
    /// What to do when the event matches.
    pub action: EventAction,
}

/// What to do when an incoming event matches a mapping.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    /// Create a session + run and trigger orchestration.
    CreateAndOrchestrate,
    /// Acknowledge the event (e.g. post a comment) without creating a run.
    Acknowledge,
    /// Silently ignore the event.
    Ignore,
}

/// Operator-configurable overrides for an integration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IntegrationOverrides {
    /// Custom agent prompt (replaces the integration's default).
    #[serde(default)]
    pub agent_prompt: Option<String>,
    /// Custom event→action mappings (replaces the integration's defaults).
    #[serde(default)]
    pub event_actions: Option<Vec<EventActionMapping>>,
    /// Custom agent role (e.g. "executor" instead of default).
    #[serde(default)]
    pub agent_role: Option<String>,
    /// Max concurrent runs for this integration.
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    /// Per-integration tool set customization.
    /// When set, overrides the integration's default tool config.
    #[serde(default)]
    pub tools: Option<crate::config::ToolConfig>,
}

/// Status summary for an integration, returned by the API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationStatus {
    pub id: String,
    pub display_name: String,
    pub configured: bool,
    pub overrides: IntegrationOverrides,
    pub queue_stats: QueueStats,
}

/// Work queue statistics.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueueStats {
    pub pending: usize,
    pub processing: usize,
    pub waiting_approval: usize,
    pub completed: usize,
    pub failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_item_status_serializes() {
        let status = WorkItemStatus::Failed("timeout".into());
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("timeout"));
    }

    #[test]
    fn event_action_mapping_round_trips() {
        let mapping = EventActionMapping {
            event_pattern: "issues.opened".into(),
            label_filter: Some("cairn".into()),
            repo_filter: None,
            action: EventAction::CreateAndOrchestrate,
        };
        let json = serde_json::to_string(&mapping).unwrap();
        let back: EventActionMapping = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event_pattern, "issues.opened");
    }

    #[test]
    fn overrides_default_is_all_none() {
        let o = IntegrationOverrides::default();
        assert!(o.agent_prompt.is_none());
        assert!(o.event_actions.is_none());
        assert!(o.agent_role.is_none());
        assert!(o.max_concurrent.is_none());
    }
}
