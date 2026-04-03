//! Assistant message endpoint boundaries per preserved route catalog.
//!
//! Covers: POST /v1/assistant/message, GET /v1/assistant/sessions,
//! GET /v1/assistant/sessions/:sessionId

use async_trait::async_trait;
use cairn_domain::ids::SessionId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::endpoints::ListQuery;
use crate::http::ListResponse;

/// Request body for POST /v1/assistant/message.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessageRequest {
    pub message: String,
    pub mode: Option<String>,
    pub session_id: Option<String>,
}

/// Response from POST /v1/assistant/message.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessageResponse {
    pub task_id: String,
}

/// Session summary for GET /v1/assistant/sessions.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSession {
    pub session_id: String,
    pub created_at: u64,
    pub message_count: u32,
}

/// Chat message within a session for GET /v1/assistant/sessions/:sessionId.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: ChatRole,
    pub content: String,
    pub created_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

/// Assistant endpoint boundaries.
#[async_trait]
pub trait AssistantEndpoints: Send + Sync {
    type Error;

    /// `POST /v1/assistant/message` — send a message and start a task.
    async fn send_message(
        &self,
        project: &ProjectKey,
        request: &AssistantMessageRequest,
    ) -> Result<AssistantMessageResponse, Self::Error>;

    /// `GET /v1/assistant/sessions` — list sessions.
    async fn list_sessions(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<AssistantSession>, Self::Error>;

    /// `GET /v1/assistant/sessions/:sessionId` — get session messages.
    async fn get_session_messages(
        &self,
        session_id: &SessionId,
        query: &ListQuery,
    ) -> Result<ListResponse<ChatMessage>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_request_serialization() {
        let req = AssistantMessageRequest {
            message: "Hello, what's the status?".to_owned(),
            mode: Some("auto".to_owned()),
            session_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["message"], "Hello, what's the status?");
        assert!(json["sessionId"].is_null());
    }

    #[test]
    fn message_response_has_task_id() {
        let resp = AssistantMessageResponse {
            task_id: "task_42".to_owned(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["taskId"], "task_42");
    }

    #[test]
    fn chat_message_roles() {
        let msg = ChatMessage {
            id: "msg_1".to_owned(),
            role: ChatRole::Assistant,
            content: "Here's the status...".to_owned(),
            created_at: 3000,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "assistant");
    }

    #[test]
    fn session_summary_serialization() {
        let session = AssistantSession {
            session_id: "sess_1".to_owned(),
            created_at: 1000,
            message_count: 5,
        };
        let json = serde_json::to_value(&session).unwrap();
        assert_eq!(json["messageCount"], 5);
    }
}
