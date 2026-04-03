//! Streaming output types for the agent execution layer.
//!
//! These types define the stable seam Worker 8 (API/SSE) uses to stream
//! assistant output to clients. The SSE publisher maps these to preserved
//! event names: `assistant_delta`, `assistant_end`, `assistant_reasoning`.

use cairn_domain::{RunId, SessionId};
use serde::{Deserialize, Serialize};

/// A streaming output chunk from the agent.
///
/// Worker 8 maps these to SSE frames with preserved event names.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamingOutput {
    /// Incremental text delta from the assistant.
    /// SSE event name: `assistant_delta`
    AssistantDelta(AssistantDelta),

    /// Assistant reasoning/thinking trace (non-output).
    /// SSE event name: `assistant_reasoning`
    AssistantReasoning(AssistantReasoning),

    /// Assistant turn is complete.
    /// SSE event name: `assistant_end`
    AssistantEnd(AssistantEnd),

    /// Tool call requested by the assistant.
    /// SSE event name: `assistant_tool_call`
    ToolCallRequested(ToolCallRequested),

    /// Tool call result returned.
    /// SSE event name: `tool_result`
    ToolResult(ToolResult),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantDelta {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub content: String,
    /// Monotonic index within the current turn.
    pub index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantReasoning {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub content: String,
    pub index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantEnd {
    pub session_id: SessionId,
    pub run_id: RunId,
    /// Reason the turn ended.
    pub stop_reason: StopReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallRequested {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_name: String,
    pub tool_call_id: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_call_id: String,
    pub content: serde_json::Value,
    pub is_error: bool,
}

impl StreamingOutput {
    /// The preserved SSE event name for this output type.
    pub fn sse_event_name(&self) -> &'static str {
        match self {
            StreamingOutput::AssistantDelta(_) => "assistant_delta",
            StreamingOutput::AssistantReasoning(_) => "assistant_reasoning",
            StreamingOutput::AssistantEnd(_) => "assistant_end",
            StreamingOutput::ToolCallRequested(_) => "assistant_tool_call",
            StreamingOutput::ToolResult(_) => "tool_result",
        }
    }

    pub fn session_id(&self) -> &SessionId {
        match self {
            StreamingOutput::AssistantDelta(d) => &d.session_id,
            StreamingOutput::AssistantReasoning(r) => &r.session_id,
            StreamingOutput::AssistantEnd(e) => &e.session_id,
            StreamingOutput::ToolCallRequested(t) => &t.session_id,
            StreamingOutput::ToolResult(t) => &t.session_id,
        }
    }

    pub fn run_id(&self) -> &RunId {
        match self {
            StreamingOutput::AssistantDelta(d) => &d.run_id,
            StreamingOutput::AssistantReasoning(r) => &r.run_id,
            StreamingOutput::AssistantEnd(e) => &e.run_id,
            StreamingOutput::ToolCallRequested(t) => &t.run_id,
            StreamingOutput::ToolResult(t) => &t.run_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_event_names_match_preserved_catalog() {
        let delta = StreamingOutput::AssistantDelta(AssistantDelta {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            content: "Hello".to_owned(),
            index: 0,
        });
        assert_eq!(delta.sse_event_name(), "assistant_delta");

        let reasoning = StreamingOutput::AssistantReasoning(AssistantReasoning {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            content: "thinking...".to_owned(),
            index: 0,
        });
        assert_eq!(reasoning.sse_event_name(), "assistant_reasoning");

        let end = StreamingOutput::AssistantEnd(AssistantEnd {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            stop_reason: StopReason::EndTurn,
        });
        assert_eq!(end.sse_event_name(), "assistant_end");
    }

    #[test]
    fn tool_call_types_carry_correct_names() {
        let tool_call = StreamingOutput::ToolCallRequested(ToolCallRequested {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            tool_name: "fs.read".to_owned(),
            tool_call_id: "tc_1".to_owned(),
            arguments: serde_json::json!({"path": "/tmp/file"}),
        });
        assert_eq!(tool_call.sse_event_name(), "assistant_tool_call");

        let result = StreamingOutput::ToolResult(ToolResult {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            tool_call_id: "tc_1".to_owned(),
            content: serde_json::json!({"text": "file contents"}),
            is_error: false,
        });
        assert_eq!(result.sse_event_name(), "tool_result");
    }
}
