use serde::{Deserialize, Serialize};

use crate::http::RouteClassification;

/// SSE event name classification per compatibility catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SseEventEntry {
    pub name: String,
    pub classification: RouteClassification,
}

/// Canonical SSE event names that the Rust API must emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SseEventName {
    Ready,
    FeedUpdate,
    PollCompleted,
    TaskUpdate,
    ApprovalRequired,
    AssistantDelta,
    AssistantEnd,
    AssistantReasoning,
    AssistantToolCall,
    MemoryProposed,
    MemoryAccepted,
    SoulUpdated,
    DigestReady,
    CodingSessionEvent,
    AgentProgress,
    SkillActivated,
    /// Operator notification emitted by the `notify_operator` built-in tool.
    OperatorNotification,
    /// GitHub issue queue progress — emitted as issues move through stages.
    GitHubProgress,
}

impl SseEventName {
    pub fn classification(self) -> RouteClassification {
        use SseEventName::*;
        match self {
            SoulUpdated | CodingSessionEvent | SkillActivated => RouteClassification::Transitional,
            _ => RouteClassification::Preserve,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SseEventName::Ready => "ready",
            SseEventName::FeedUpdate => "feed_update",
            SseEventName::PollCompleted => "poll_completed",
            SseEventName::TaskUpdate => "task_update",
            SseEventName::ApprovalRequired => "approval_required",
            SseEventName::AssistantDelta => "assistant_delta",
            SseEventName::AssistantEnd => "assistant_end",
            SseEventName::AssistantReasoning => "assistant_reasoning",
            SseEventName::AssistantToolCall => "assistant_tool_call",
            SseEventName::MemoryProposed => "memory_proposed",
            SseEventName::MemoryAccepted => "memory_accepted",
            SseEventName::SoulUpdated => "soul_updated",
            SseEventName::DigestReady => "digest_ready",
            SseEventName::CodingSessionEvent => "coding_session_event",
            SseEventName::AgentProgress => "agent_progress",
            SseEventName::SkillActivated => "skill_activated",
            SseEventName::OperatorNotification => "operator_notification",
            SseEventName::GitHubProgress => "github_progress",
        }
    }
}

/// Outbound SSE frame carrying a typed event name and JSON payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SseFrame {
    pub event: SseEventName,
    pub data: serde_json::Value,
    pub id: Option<String>,
}

/// Seam for SSE stream management. Implementors push frames to connected clients.
pub trait SseStream {
    type Error;

    fn send(&mut self, frame: SseFrame) -> Result<(), Self::Error>;
}

/// Returns the preserved SSE event catalog for compatibility tracking.
pub fn preserved_sse_catalog() -> Vec<SseEventEntry> {
    use SseEventName::*;
    let all = [
        Ready,
        FeedUpdate,
        PollCompleted,
        TaskUpdate,
        ApprovalRequired,
        AssistantDelta,
        AssistantEnd,
        AssistantReasoning,
        AssistantToolCall,
        MemoryProposed,
        MemoryAccepted,
        SoulUpdated,
        DigestReady,
        CodingSessionEvent,
        AgentProgress,
        SkillActivated,
    ];
    all.into_iter()
        .map(|name| SseEventEntry {
            name: name.as_str().to_owned(),
            classification: name.classification(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_catalog_has_all_events() {
        let catalog = preserved_sse_catalog();
        assert_eq!(catalog.len(), 16);
    }

    #[test]
    fn transitional_sse_events_marked() {
        assert_eq!(
            SseEventName::SoulUpdated.classification(),
            RouteClassification::Transitional
        );
        assert_eq!(
            SseEventName::Ready.classification(),
            RouteClassification::Preserve
        );
    }

    #[test]
    fn sse_frame_serialization() {
        let frame = SseFrame {
            event: SseEventName::Ready,
            data: serde_json::json!({"clientId": "c1"}),
            id: Some("evt_1".to_owned()),
        };
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["event"], "ready");
    }
}
