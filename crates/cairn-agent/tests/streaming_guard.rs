//! Guard: StreamingOutput SSE event names match the preserved assistant
//! SSE families that Worker 8 consumes. If these names change, the SSE
//! publisher breaks.

// Import from crate root — proves re-exports exist without submodule reach-in.
use cairn_agent::{AssistantDelta, AssistantEnd, AssistantReasoning, StopReason, StreamingOutput};
// ToolCallRequested and ToolResult need submodule (not re-exported at root — that's fine,
// Worker 8 uses them via cairn_agent::streaming::*)
use cairn_agent::streaming::{ToolCallRequested, ToolResult};
use cairn_domain::{RunId, SessionId};

fn session() -> SessionId {
    SessionId::new("s1")
}
fn run() -> RunId {
    RunId::new("r1")
}

/// The preserved SSE event names are a hard contract.
/// This test breaks if any name drifts.
#[test]
fn preserved_sse_event_names_are_stable() {
    let cases: Vec<(StreamingOutput, &str)> = vec![
        (
            StreamingOutput::AssistantDelta(AssistantDelta {
                session_id: session(),
                run_id: run(),
                content: String::new(),
                index: 0,
            }),
            "assistant_delta",
        ),
        (
            StreamingOutput::AssistantReasoning(AssistantReasoning {
                session_id: session(),
                run_id: run(),
                content: String::new(),
                index: 0,
            }),
            "assistant_reasoning",
        ),
        (
            StreamingOutput::AssistantEnd(AssistantEnd {
                session_id: session(),
                run_id: run(),
                stop_reason: StopReason::EndTurn,
            }),
            "assistant_end",
        ),
        (
            StreamingOutput::ToolCallRequested(ToolCallRequested {
                session_id: session(),
                run_id: run(),
                tool_name: "t".to_owned(),
                tool_call_id: "tc".to_owned(),
                arguments: serde_json::json!({}),
            }),
            "assistant_tool_call",
        ),
        (
            StreamingOutput::ToolResult(ToolResult {
                session_id: session(),
                run_id: run(),
                tool_call_id: "tc".to_owned(),
                content: serde_json::json!({}),
                is_error: false,
            }),
            "tool_result",
        ),
    ];

    for (output, expected_name) in &cases {
        assert_eq!(
            output.sse_event_name(),
            *expected_name,
            "SSE event name mismatch for {:?}",
            std::mem::discriminant(output)
        );
    }
}

/// All stop reasons are serializable and distinct.
#[test]
fn stop_reasons_are_stable() {
    let reasons = [
        StopReason::EndTurn,
        StopReason::ToolUse,
        StopReason::MaxTokens,
        StopReason::StopSequence,
    ];
    let serialized: Vec<String> = reasons
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();

    assert_eq!(serialized[0], "\"end_turn\"");
    assert_eq!(serialized[1], "\"tool_use\"");
    assert_eq!(serialized[2], "\"max_tokens\"");
    assert_eq!(serialized[3], "\"stop_sequence\"");
}

/// session_id and run_id accessors work for all variants.
#[test]
fn all_variants_carry_session_and_run() {
    let outputs: Vec<StreamingOutput> = vec![
        StreamingOutput::AssistantDelta(AssistantDelta {
            session_id: session(),
            run_id: run(),
            content: String::new(),
            index: 0,
        }),
        StreamingOutput::AssistantReasoning(AssistantReasoning {
            session_id: session(),
            run_id: run(),
            content: String::new(),
            index: 0,
        }),
        StreamingOutput::AssistantEnd(AssistantEnd {
            session_id: session(),
            run_id: run(),
            stop_reason: StopReason::EndTurn,
        }),
        StreamingOutput::ToolCallRequested(ToolCallRequested {
            session_id: session(),
            run_id: run(),
            tool_name: "t".to_owned(),
            tool_call_id: "tc".to_owned(),
            arguments: serde_json::json!({}),
        }),
        StreamingOutput::ToolResult(ToolResult {
            session_id: session(),
            run_id: run(),
            tool_call_id: "tc".to_owned(),
            content: serde_json::json!({}),
            is_error: false,
        }),
    ];

    for output in &outputs {
        assert_eq!(output.session_id(), &session());
        assert_eq!(output.run_id(), &run());
    }
}
