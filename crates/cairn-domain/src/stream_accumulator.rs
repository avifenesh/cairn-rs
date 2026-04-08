//! Stream accumulator — collects SSE streaming events from any
//! OpenAI-compatible or Anthropic provider into a complete response.
//!
//! Used by the Playground streaming endpoint and the orchestrate handler
//! to accumulate partial deltas into final content blocks.
//!
//! Adopted from cersei-provider's StreamAccumulator (MIT, pacifio/cersei).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Stream events (provider-agnostic) ────────────────────────────────────────

/// Events emitted by an LLM provider's SSE stream.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Stream started — carries message ID and model.
    MessageStart { id: String, model: String },
    /// A new content block is beginning at `index`.
    ContentBlockStart {
        index: usize,
        block_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Incremental text for a content block.
    TextDelta { index: usize, text: String },
    /// Incremental JSON for a tool_use input argument.
    InputJsonDelta { index: usize, partial_json: String },
    /// Incremental thinking/reasoning text.
    ThinkingDelta { index: usize, thinking: String },
    /// A content block has finished.
    ContentBlockStop { index: usize },
    /// Message-level delta (stop reason, usage update).
    MessageDelta {
        stop_reason: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    /// Message complete.
    MessageStop,
    /// Keep-alive ping.
    Ping,
    /// Provider error.
    Error { message: String },
}

// ── Accumulated content blocks ───────────────────────────────────────────────

/// A complete content block after accumulation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text content.
    Text { text: String },
    /// Tool invocation with accumulated JSON input.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Extended thinking/chain-of-thought.
    Thinking { thinking: String },
}

// ── Accumulator ──────────────────────────────────────────────────────────────

/// Accumulates streaming events into a complete response.
///
/// Feed events via [`process_event`] as they arrive from the SSE stream,
/// then call [`finish`] to extract the final response.
pub struct StreamAccumulator {
    content_blocks: Vec<ContentBlock>,
    partial_text: HashMap<usize, String>,
    partial_json: HashMap<usize, String>,
    partial_thinking: HashMap<usize, String>,
    block_types: HashMap<usize, String>,
    tool_use_ids: HashMap<usize, String>,
    tool_use_names: HashMap<usize, String>,
    stop_reason: Option<String>,
    input_tokens: u32,
    output_tokens: u32,
    model: Option<String>,
    message_id: Option<String>,
}

/// Final accumulated response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccumulatedResponse {
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub content_blocks: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl AccumulatedResponse {
    /// Extract the full text from all text blocks, concatenated.
    pub fn text(&self) -> String {
        self.content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Extract tool use blocks.
    pub fn tool_uses(&self) -> Vec<(&str, &str, &serde_json::Value)> {
        self.content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.as_str(), name.as_str(), input))
                }
                _ => None,
            })
            .collect()
    }

    /// Total tokens (input + output).
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

impl Default for StreamAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamAccumulator {
    pub fn new() -> Self {
        Self {
            content_blocks: Vec::new(),
            partial_text: HashMap::new(),
            partial_json: HashMap::new(),
            partial_thinking: HashMap::new(),
            block_types: HashMap::new(),
            tool_use_ids: HashMap::new(),
            tool_use_names: HashMap::new(),
            stop_reason: None,
            input_tokens: 0,
            output_tokens: 0,
            model: None,
            message_id: None,
        }
    }

    /// Process a single stream event.
    pub fn process_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::MessageStart { id, model } => {
                self.message_id = Some(id);
                self.model = Some(model);
            }
            StreamEvent::ContentBlockStart {
                index,
                block_type,
                id,
                name,
            } => {
                self.block_types.insert(index, block_type);
                if let Some(id) = id {
                    self.tool_use_ids.insert(index, id);
                }
                if let Some(name) = name {
                    self.tool_use_names.insert(index, name);
                }
            }
            StreamEvent::TextDelta { index, text } => {
                self.partial_text.entry(index).or_default().push_str(&text);
            }
            StreamEvent::InputJsonDelta {
                index,
                partial_json,
            } => {
                self.partial_json
                    .entry(index)
                    .or_default()
                    .push_str(&partial_json);
            }
            StreamEvent::ThinkingDelta { index, thinking } => {
                self.partial_thinking
                    .entry(index)
                    .or_default()
                    .push_str(&thinking);
            }
            StreamEvent::ContentBlockStop { index } => {
                let block_type = self.block_types.get(&index).cloned().unwrap_or_default();
                let block = match block_type.as_str() {
                    "tool_use" => {
                        let json_str = self.partial_json.remove(&index).unwrap_or_default();
                        let input =
                            serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
                        ContentBlock::ToolUse {
                            id: self.tool_use_ids.remove(&index).unwrap_or_default(),
                            name: self.tool_use_names.remove(&index).unwrap_or_default(),
                            input,
                        }
                    }
                    "thinking" => ContentBlock::Thinking {
                        thinking: self.partial_thinking.remove(&index).unwrap_or_default(),
                    },
                    _ => ContentBlock::Text {
                        text: self.partial_text.remove(&index).unwrap_or_default(),
                    },
                };
                while self.content_blocks.len() <= index {
                    self.content_blocks.push(ContentBlock::Text {
                        text: String::new(),
                    });
                }
                self.content_blocks[index] = block;
            }
            StreamEvent::MessageDelta {
                stop_reason,
                input_tokens,
                output_tokens,
            } => {
                if let Some(sr) = stop_reason {
                    self.stop_reason = Some(sr);
                }
                if let Some(t) = input_tokens {
                    self.input_tokens = t;
                }
                if let Some(t) = output_tokens {
                    self.output_tokens = t;
                }
            }
            StreamEvent::MessageStop | StreamEvent::Ping => {}
            StreamEvent::Error { .. } => {}
        }
    }

    /// Get accumulated text so far (for live streaming display).
    pub fn current_text(&self) -> String {
        let mut parts: Vec<(usize, &str)> = self
            .partial_text
            .iter()
            .map(|(i, t)| (*i, t.as_str()))
            .collect();
        // Also include finalized text blocks.
        for (i, block) in self.content_blocks.iter().enumerate() {
            if let ContentBlock::Text { text } = block {
                if !text.is_empty() {
                    parts.push((i, text.as_str()));
                }
            }
        }
        parts.sort_by_key(|(i, _)| *i);
        parts.into_iter().map(|(_, t)| t).collect()
    }

    /// Consume the accumulator and produce the final response.
    pub fn finish(self) -> AccumulatedResponse {
        AccumulatedResponse {
            message_id: self.message_id,
            model: self.model,
            content_blocks: self.content_blocks,
            stop_reason: self.stop_reason,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        }
    }
}

// ── OpenAI SSE line parser ───────────────────────────────────────────────────

/// Parse an OpenAI-format SSE `data:` line into a `StreamEvent`.
///
/// OpenAI streaming format sends `data: {...}\n\n` lines where the JSON
/// contains `choices[0].delta.content` for text and `choices[0].delta.tool_calls`
/// for tool use.
pub fn parse_openai_sse_line(line: &str) -> Option<StreamEvent> {
    let data = line.strip_prefix("data: ")?;
    if data == "[DONE]" {
        return Some(StreamEvent::MessageStop);
    }

    let v: serde_json::Value = serde_json::from_str(data).ok()?;

    // Check for text delta.
    if let Some(content) = v
        .pointer("/choices/0/delta/content")
        .and_then(|c| c.as_str())
    {
        return Some(StreamEvent::TextDelta {
            index: 0,
            text: content.to_owned(),
        });
    }

    // Check for tool call delta.
    if let Some(tool_calls) = v
        .pointer("/choices/0/delta/tool_calls")
        .and_then(|t| t.as_array())
    {
        for tc in tool_calls {
            let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            // Start of a new tool call.
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                    return Some(StreamEvent::ContentBlockStart {
                        index: index + 1, // offset by 1 since index 0 is text
                        block_type: "tool_use".to_owned(),
                        id: tc.get("id").and_then(|i| i.as_str()).map(str::to_owned),
                        name: Some(name.to_owned()),
                    });
                }
                // Argument delta.
                if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                    return Some(StreamEvent::InputJsonDelta {
                        index: index + 1,
                        partial_json: args.to_owned(),
                    });
                }
            }
        }
    }

    // Check for finish reason.
    if let Some(reason) = v
        .pointer("/choices/0/finish_reason")
        .and_then(|r| r.as_str())
    {
        let usage_input = v
            .pointer("/usage/prompt_tokens")
            .and_then(|t| t.as_u64())
            .map(|t| t as u32);
        let usage_output = v
            .pointer("/usage/completion_tokens")
            .and_then(|t| t.as_u64())
            .map(|t| t as u32);
        return Some(StreamEvent::MessageDelta {
            stop_reason: Some(reason.to_owned()),
            input_tokens: usage_input,
            output_tokens: usage_output,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulate_text_blocks() {
        let mut acc = StreamAccumulator::new();
        acc.process_event(StreamEvent::MessageStart {
            id: "msg_1".to_owned(),
            model: "gpt-4o".to_owned(),
        });
        acc.process_event(StreamEvent::ContentBlockStart {
            index: 0,
            block_type: "text".to_owned(),
            id: None,
            name: None,
        });
        acc.process_event(StreamEvent::TextDelta {
            index: 0,
            text: "Hello ".to_owned(),
        });
        acc.process_event(StreamEvent::TextDelta {
            index: 0,
            text: "world!".to_owned(),
        });
        acc.process_event(StreamEvent::ContentBlockStop { index: 0 });

        assert_eq!(acc.current_text(), "Hello world!");

        let resp = acc.finish();
        assert_eq!(resp.text(), "Hello world!");
        assert_eq!(resp.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn accumulate_tool_use() {
        let mut acc = StreamAccumulator::new();
        acc.process_event(StreamEvent::ContentBlockStart {
            index: 0,
            block_type: "tool_use".to_owned(),
            id: Some("call_1".to_owned()),
            name: Some("memory_search".to_owned()),
        });
        acc.process_event(StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: r#"{"query":"#.to_owned(),
        });
        acc.process_event(StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: r#""rust"}"#.to_owned(),
        });
        acc.process_event(StreamEvent::ContentBlockStop { index: 0 });

        let resp = acc.finish();
        let tools = resp.tool_uses();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "call_1");
        assert_eq!(tools[0].1, "memory_search");
        assert_eq!(tools[0].2, &serde_json::json!({"query": "rust"}));
    }

    #[test]
    fn accumulate_thinking() {
        let mut acc = StreamAccumulator::new();
        acc.process_event(StreamEvent::ContentBlockStart {
            index: 0,
            block_type: "thinking".to_owned(),
            id: None,
            name: None,
        });
        acc.process_event(StreamEvent::ThinkingDelta {
            index: 0,
            thinking: "Let me think...".to_owned(),
        });
        acc.process_event(StreamEvent::ContentBlockStop { index: 0 });

        let resp = acc.finish();
        assert!(matches!(
            &resp.content_blocks[0],
            ContentBlock::Thinking { thinking } if thinking == "Let me think..."
        ));
    }

    #[test]
    fn parse_openai_text_delta() {
        let line = r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hello"}}]}"#;
        let event = parse_openai_sse_line(line).unwrap();
        assert!(matches!(event, StreamEvent::TextDelta { text, .. } if text == "Hello"));
    }

    #[test]
    fn parse_openai_done() {
        let event = parse_openai_sse_line("data: [DONE]").unwrap();
        assert!(matches!(event, StreamEvent::MessageStop));
    }

    #[test]
    fn parse_openai_finish_reason() {
        let line = r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let event = parse_openai_sse_line(line).unwrap();
        assert!(matches!(
            event,
            StreamEvent::MessageDelta {
                stop_reason: Some(r),
                input_tokens: Some(10),
                output_tokens: Some(5),
            } if r == "stop"
        ));
    }

    #[test]
    fn usage_tracking() {
        let mut acc = StreamAccumulator::new();
        acc.process_event(StreamEvent::MessageDelta {
            stop_reason: Some("stop".to_owned()),
            input_tokens: Some(100),
            output_tokens: Some(50),
        });
        let resp = acc.finish();
        assert_eq!(resp.total_tokens(), 150);
    }
}
