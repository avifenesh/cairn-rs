//! Orchestrator LLM integration — PromptBuilder + ResponseParser + BrainLlmClient.
//!
//! This module is the thin layer between the orchestrator control loop and the
//! brain LLM provider.  It owns three responsibilities:
//!
//! 1. **PromptBuilder** — converts a `ContextBundle` into an OpenAI-compatible
//!    messages array that the brain LLM can reason over.
//! 2. **ResponseParser** — extracts `ActionProposal` values from the LLM's raw
//!    text output, handling malformed JSON gracefully.
//! 3. **BrainLlmClient** — wraps a `GenerationProvider` and orchestrates the
//!    full call: build prompt → generate → parse → return proposals.
//!
//! ### Type ownership
//! `ActionType` and `ActionProposal` live in `cairn-domain::orchestrator` (W2).
//! `ContextBundle` and `TaskSummary` are runtime-layer concerns defined here
//! because they aggregate information from store projections at call time.

use std::sync::Arc;

use cairn_domain::{
    orchestrator::{ActionProposal, ActionType},
    providers::{GenerationProvider, ProviderAdapterError, ProviderBindingSettings},
};
use serde::{Deserialize, Serialize};

// ── Context bundle ────────────────────────────────────────────────────────────

/// Summary of one completed (or in-flight) sub-task, included in the context
/// fed to the brain LLM so it can reason about progress.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: String,
    /// Optional short description of what the task was supposed to do.
    pub description: Option<String>,
    /// Terminal or current state string (e.g. "completed", "failed", "leased").
    pub state: String,
    /// Free-form result text returned by the worker, if any.
    pub result: Option<String>,
}

/// All context the orchestrator needs to decide its next move.
///
/// Assembled by the control loop from store projections before each LLM call.
/// W2 may enrich this with additional projection fields as read-models mature.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContextBundle {
    /// Run identifier this orchestration step belongs to.
    pub run_id: String,
    /// Parent session.
    pub session_id: String,
    /// The high-level goal or task description provided at run creation.
    pub goal: String,
    /// Summarised history of tasks completed (or failed) so far.
    pub task_history: Vec<TaskSummary>,
    /// Tool IDs visible to this run.  Empty = all tools permitted.
    pub available_tools: Vec<String>,
    /// Relevant knowledge snippets retrieved from the memory store.
    pub memory_snippets: Vec<String>,
    /// IDs of approvals currently awaiting a decision.
    pub pending_approvals: Vec<String>,
    /// Agent role attached to this run, if any (e.g. "orchestrator").
    pub agent_role: Option<String>,
    /// How many times the orchestrator loop has run for this run so far.
    pub iteration: u32,
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors from the orchestrator LLM layer.
#[derive(Debug)]
pub enum OrchestratorError {
    /// The LLM call itself failed.
    ProviderError(ProviderAdapterError),
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::ProviderError(e) => write!(f, "provider error: {e}"),
        }
    }
}

impl std::error::Error for OrchestratorError {}

// ── PromptBuilder ─────────────────────────────────────────────────────────────

/// Builds an OpenAI-compatible messages array from a `ContextBundle`.
///
/// The system message establishes the orchestrator persona and instructs the
/// LLM to return a JSON array of `ActionProposal` objects matching
/// `cairn_domain::orchestrator::ActionProposal`.
pub struct PromptBuilder;

impl PromptBuilder {
    /// Build the messages array for the brain LLM.
    ///
    /// Returns `Vec<serde_json::Value>` in OpenAI chat format:
    /// `[{"role": "system", "content": "..."}, {"role": "user", "content": "..."}]`
    pub fn build(ctx: &ContextBundle) -> Vec<serde_json::Value> {
        let system = Self::system_prompt(ctx);
        let user = Self::user_message(ctx);
        vec![
            serde_json::json!({ "role": "system", "content": system }),
            serde_json::json!({ "role": "user",   "content": user   }),
        ]
    }

    fn system_prompt(ctx: &ContextBundle) -> String {
        let role_hint = ctx.agent_role.as_deref().unwrap_or("orchestrator");
        format!(
            "You are a cairn {role_hint} — an autonomous agent that takes action \
             to achieve goals. Work through the task step by step: understand what \
             is needed, use tools to gather context and take action, verify the \
             result, then complete.\n\
             \n\
             ## Workflow\n\
             1. **Understand** — Read the goal and run state. Gather context with \
                available tools.\n\
             2. **Act** — Take concrete steps: invoke tools, spawn sub-agents for \
                parallel work, store knowledge.\n\
             3. **Verify** — Check that your actions achieved the goal.\n\
             4. **Complete** — Call complete_run with a summary of what was accomplished. \
                Only complete after taking action and verifying.\n\
             \n\
             ## Response format\n\
             Respond ONLY with a JSON array of action objects. Each object must have:\n\
             - \"action_type\": one of {action_types}\n\
             - \"description\": short explanation of why you chose this action\n\
             - \"confidence\": float 0.0–1.0\n\
             - \"requires_approval\": true only for writes or destructive actions\n\
             - \"tool_name\" (optional): tool ID or sub-agent role\n\
             - \"tool_args\" (optional): JSON arguments\n\
             \n\
             Field conventions:\n\
             - spawn_subagent: tool_name = role, tool_args = {{\"goal\": \"...\"}}\n\
             - invoke_tool:    tool_name = tool ID, tool_args = {{...}}\n\
             - create_memory:  tool_args = {{\"content\": \"...\", \"source\": \"...\"}}\n\
             - send_notification: tool_args = {{\"to\": \"...\", \"message\": \"...\"}}\n\
             - complete_run:   description = summary of what was accomplished\n\
             - escalate_to_operator: description = why, requires_approval = true\n\
             \n\
             Return only the JSON array — no prose, no markdown fences.",
            action_types = r#""spawn_subagent" | "invoke_tool" | "create_memory" | "send_notification" | "complete_run" | "escalate_to_operator""#,
        )
    }

    fn user_message(ctx: &ContextBundle) -> String {
        let mut parts = Vec::new();

        parts.push(format!("## Goal\n{}", ctx.goal));

        parts.push(format!(
            "## Run state\nrun_id: {}\nsession_id: {}\niteration: {}",
            ctx.run_id, ctx.session_id, ctx.iteration,
        ));

        if !ctx.task_history.is_empty() {
            let lines: Vec<String> = ctx
                .task_history
                .iter()
                .map(|t| {
                    let result = t.result.as_deref().unwrap_or("—");
                    let desc = t.description.as_deref().unwrap_or("(no description)");
                    format!(
                        "- [{}] {} | {} | result: {}",
                        t.state, t.task_id, desc, result
                    )
                })
                .collect();
            parts.push(format!("## Task history\n{}", lines.join("\n")));
        }

        if !ctx.available_tools.is_empty() {
            parts.push(format!(
                "## Available tools\n{}",
                ctx.available_tools.join(", ")
            ));
        }

        if !ctx.memory_snippets.is_empty() {
            let snippets = ctx
                .memory_snippets
                .iter()
                .enumerate()
                .map(|(i, s)| format!("[{}] {}", i + 1, s))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("## Relevant knowledge\n{snippets}"));
        }

        if !ctx.pending_approvals.is_empty() {
            parts.push(format!(
                "## Pending approvals\n{} approval(s) awaiting decision: {}",
                ctx.pending_approvals.len(),
                ctx.pending_approvals.join(", "),
            ));
        }

        parts.push("## What should happen next?\nReturn a JSON action array.".to_owned());

        parts.join("\n\n")
    }
}

// ── ResponseParser ────────────────────────────────────────────────────────────

/// Parses the brain LLM's raw text into `ActionProposal` values from
/// `cairn_domain::orchestrator`.
///
/// Handles:
/// - Valid JSON arrays of action objects
/// - A JSON array/object wrapped in markdown fences
/// - Completely malformed text → escalate-to-operator fallback
pub struct ResponseParser;

impl ResponseParser {
    /// Parse raw LLM output into zero or more `ActionProposal` values.
    ///
    /// Never returns `Err` — malformed responses yield a single
    /// `EscalateToOperator` proposal so the control loop can surface the issue.
    pub fn parse(raw: &str) -> Vec<ActionProposal> {
        let cleaned = Self::strip_markdown_fence(raw.trim());

        // Try: JSON array of action objects
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(cleaned) {
            let proposals: Vec<ActionProposal> = arr
                .into_iter()
                .filter_map(Self::parse_action_value)
                .collect();
            if !proposals.is_empty() {
                return proposals;
            }
        }

        // Try: single action object (LLM returned an object instead of array)
        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(cleaned) {
            if let Some(proposal) = Self::parse_action_value(obj) {
                return vec![proposal];
            }
        }

        // Fallback: completely unparsable → escalate so the operator can intervene
        vec![ActionProposal::escalate(
            format!(
                "LLM returned a non-JSON response: {}",
                &raw[..raw.len().min(200)]
            ),
            0.0,
        )]
    }

    /// Strip optional markdown code fences (```json … ``` or ``` … ```).
    fn strip_markdown_fence(s: &str) -> &str {
        let s = s.trim();
        if let Some(inner) = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")) {
            if let Some(inner) = inner.strip_suffix("```") {
                return inner.trim();
            }
        }
        s
    }

    /// Convert a raw `serde_json::Value` (one element of the LLM's array) into
    /// an `ActionProposal`.  Returns `None` for values with no `action_type`.
    fn parse_action_value(v: serde_json::Value) -> Option<ActionProposal> {
        let obj = v.as_object()?;
        let type_str = obj.get("action_type")?.as_str()?;

        let action_type = Self::parse_action_type(type_str)?;

        let description = obj
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_owned();
        let confidence = obj
            .get("confidence")
            .and_then(|c| c.as_f64())
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let requires_approval = obj
            .get("requires_approval")
            .and_then(|r| r.as_bool())
            .unwrap_or(false);
        let tool_name = obj
            .get("tool_name")
            .and_then(|n| n.as_str())
            .map(str::to_owned);
        let tool_args = obj.get("tool_args").cloned();

        Some(ActionProposal {
            action_type,
            description,
            confidence,
            tool_name,
            tool_args,
            requires_approval,
        })
    }

    fn parse_action_type(s: &str) -> Option<ActionType> {
        match s {
            "spawn_subagent" => Some(ActionType::SpawnSubagent),
            "invoke_tool" => Some(ActionType::InvokeTool),
            "create_memory" => Some(ActionType::CreateMemory),
            "send_notification" => Some(ActionType::SendNotification),
            "complete_run" => Some(ActionType::CompleteRun),
            "escalate_to_operator" => Some(ActionType::EscalateToOperator),
            _ => {
                // Unknown type — caller will filter None → escalate at call site
                None
            }
        }
    }
}

// ── BrainLlmClient ────────────────────────────────────────────────────────────

/// Orchestrates a full brain LLM call: build prompt → generate → parse.
///
/// Wraps any `GenerationProvider` implementation. In production this is
/// typically an OpenAI-compatible provider resolved from `conn_cairn_brain`.
pub struct BrainLlmClient {
    provider: Arc<dyn GenerationProvider>,
    model_id: String,
    settings: ProviderBindingSettings,
}

impl BrainLlmClient {
    /// Create a client backed by the given provider and model.
    pub fn new(provider: Arc<dyn GenerationProvider>, model_id: impl Into<String>) -> Self {
        Self {
            provider,
            model_id: model_id.into(),
            settings: ProviderBindingSettings {
                max_output_tokens: Some(2048),
                ..Default::default()
            },
        }
    }

    /// Override the generation settings (e.g. max_output_tokens, temperature).
    pub fn with_settings(mut self, settings: ProviderBindingSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Run the full orchestration cycle for one context bundle.
    ///
    /// Returns a non-empty list of `ActionProposal` values on success.
    /// On provider failure returns `Err(OrchestratorError::ProviderError)`.
    /// On parse failure (malformed LLM output) the list contains a single
    /// `EscalateToOperator` proposal — never `Err`.
    pub async fn propose_actions(
        &self,
        ctx: &ContextBundle,
    ) -> Result<Vec<ActionProposal>, OrchestratorError> {
        let messages = PromptBuilder::build(ctx);
        let response = self
            .provider
            .generate(&self.model_id, messages, &self.settings, &[])
            .await
            .map_err(OrchestratorError::ProviderError)?;
        Ok(ResponseParser::parse(&response.text))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::providers::{GenerationResponse, ProviderAdapterError};

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn base_ctx() -> ContextBundle {
        ContextBundle {
            run_id: "run_orch_1".to_owned(),
            session_id: "sess_1".to_owned(),
            goal: "Research and summarise the cairn-rs architecture.".to_owned(),
            iteration: 1,
            agent_role: Some("orchestrator".to_owned()),
            available_tools: vec!["cairn.search".to_owned(), "cairn.retrieve".to_owned()],
            ..Default::default()
        }
    }

    struct MockProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl GenerationProvider for MockProvider {
        async fn generate(
            &self,
            _model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
            _tools: &[serde_json::Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Ok(GenerationResponse {
                text: self.response.clone(),
                input_tokens: Some(120),
                output_tokens: Some(80),
                model_id: "test-model".to_owned(),
                tool_calls: vec![],
                finish_reason: None,
            })
        }
    }

    struct FailingProvider;

    #[async_trait::async_trait]
    impl GenerationProvider for FailingProvider {
        async fn generate(
            &self,
            _model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
            _tools: &[serde_json::Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Err(ProviderAdapterError::TransportFailure(
                "mock failure".to_owned(),
            ))
        }
    }

    // ── PromptBuilder ─────────────────────────────────────────────────────────

    #[test]
    fn prompt_builder_produces_system_and_user() {
        let msgs = PromptBuilder::build(&base_ctx());
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn prompt_builder_system_lists_all_action_types() {
        let msgs = PromptBuilder::build(&base_ctx());
        let sys = msgs[0]["content"].as_str().unwrap();
        assert!(sys.contains("spawn_subagent"), "missing spawn_subagent");
        assert!(sys.contains("invoke_tool"), "missing invoke_tool");
        assert!(sys.contains("create_memory"), "missing create_memory");
        assert!(sys.contains("complete_run"), "missing complete_run");
        assert!(
            sys.contains("escalate_to_operator"),
            "missing escalate_to_operator"
        );
        assert!(
            sys.contains("JSON array"),
            "must instruct JSON array return"
        );
    }

    #[test]
    fn prompt_builder_user_contains_goal_and_run_id() {
        let msgs = PromptBuilder::build(&base_ctx());
        let user = msgs[1]["content"].as_str().unwrap();
        assert!(
            user.contains("cairn-rs architecture"),
            "goal must be embedded"
        );
        assert!(user.contains("run_orch_1"), "run_id must be embedded");
        assert!(user.contains("cairn.search"), "tools must be embedded");
    }

    #[test]
    fn prompt_builder_embeds_task_history() {
        let mut ctx = base_ctx();
        ctx.task_history = vec![TaskSummary {
            task_id: "task_1".to_owned(),
            description: Some("fetch RFC docs".to_owned()),
            state: "completed".to_owned(),
            result: Some("found 12 RFC files".to_owned()),
        }];
        let user = PromptBuilder::build(&ctx)[1]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(user.contains("task_1"), "task_id must appear");
        assert!(
            user.contains("found 12 RFC files"),
            "task result must appear"
        );
    }

    #[test]
    fn prompt_builder_embeds_memory_snippets() {
        let mut ctx = base_ctx();
        ctx.memory_snippets = vec!["cairn-rs uses event sourcing".to_owned()];
        let user = PromptBuilder::build(&ctx)[1]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            user.contains("event sourcing"),
            "memory snippet must appear"
        );
    }

    #[test]
    fn prompt_builder_embeds_pending_approvals() {
        let mut ctx = base_ctx();
        ctx.pending_approvals = vec!["appr_abc".to_owned()];
        let user = PromptBuilder::build(&ctx)[1]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(user.contains("appr_abc"), "approval id must appear");
    }

    // ── ResponseParser ────────────────────────────────────────────────────────

    #[test]
    fn parser_valid_spawn_and_complete() {
        let raw = r#"[
          {"action_type":"spawn_subagent","description":"Need research","confidence":0.9,
           "tool_name":"researcher","tool_args":{"goal":"summarise RFCs"},"requires_approval":false},
          {"action_type":"complete_run","description":"All done","confidence":0.95,"requires_approval":false}
        ]"#;
        let ps = ResponseParser::parse(raw);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].action_type, ActionType::SpawnSubagent);
        assert_eq!(ps[0].tool_name.as_deref(), Some("researcher"));
        assert!((ps[0].confidence - 0.9).abs() < 1e-9);
        assert_eq!(ps[1].action_type, ActionType::CompleteRun);
    }

    #[test]
    fn parser_strips_markdown_fence() {
        let raw = "```json\n[{\"action_type\":\"invoke_tool\",\"description\":\"search\",\"confidence\":0.7,\"tool_name\":\"cairn.search\",\"tool_args\":{},\"requires_approval\":false}]\n```";
        let ps = ResponseParser::parse(raw);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::InvokeTool);
    }

    #[test]
    fn parser_single_object_accepted() {
        let raw = r#"{"action_type":"complete_run","description":"done","confidence":1.0,"requires_approval":false}"#;
        let ps = ResponseParser::parse(raw);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::CompleteRun);
    }

    #[test]
    fn parser_malformed_returns_escalate() {
        let raw = "Sorry, I cannot help with that right now.";
        let ps = ResponseParser::parse(raw);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::EscalateToOperator);
        assert!(ps[0].requires_approval, "escalation must require approval");
        assert!(ps[0].description.contains("non-JSON response"));
    }

    #[test]
    fn parser_unknown_action_type_is_filtered() {
        // Unknown types return None from parse_action_type → filtered out
        let raw = r#"[{"action_type":"destroy_everything","description":"chaos","confidence":0.99,"requires_approval":false}]"#;
        let ps = ResponseParser::parse(raw);
        // All filtered → empty array → fallback escalate
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::EscalateToOperator);
    }

    #[test]
    fn parser_empty_array_returns_escalate() {
        let ps = ResponseParser::parse("[]");
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::EscalateToOperator);
    }

    #[test]
    fn parser_confidence_clamped_to_0_1() {
        let raw = r#"[{"action_type":"complete_run","description":"ok","confidence":99.0,"requires_approval":false}]"#;
        let ps = ResponseParser::parse(raw);
        assert!(
            ps[0].confidence <= 1.0,
            "confidence must be clamped to [0,1]"
        );
    }

    #[test]
    fn parser_missing_optional_fields_default_safely() {
        let raw = r#"[{"action_type":"complete_run"}]"#;
        let ps = ResponseParser::parse(raw);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::CompleteRun);
        assert_eq!(ps[0].description, "");
        assert!(
            (ps[0].confidence - 0.5).abs() < 1e-9,
            "missing confidence defaults to 0.5"
        );
        assert!(!ps[0].requires_approval);
        assert!(ps[0].tool_name.is_none());
        assert!(ps[0].tool_args.is_none());
    }

    // ── BrainLlmClient ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn brain_client_calls_provider_and_parses() {
        let mock = Arc::new(MockProvider {
            response: r#"[{"action_type":"spawn_subagent","description":"split the work","confidence":0.85,"tool_name":"researcher","tool_args":{"goal":"analyse RFCs"},"requires_approval":false}]"#.to_owned(),
        });
        let client = BrainLlmClient::new(mock, "cyankiwi/gemma-4-31B-it-AWQ-4bit");
        let ps = client.propose_actions(&base_ctx()).await.unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::SpawnSubagent);
        assert_eq!(ps[0].tool_name.as_deref(), Some("researcher"));
        assert_eq!(ps[0].tool_args.as_ref().unwrap()["goal"], "analyse RFCs");
    }

    #[tokio::test]
    async fn brain_client_propagates_provider_error() {
        let client = BrainLlmClient::new(Arc::new(FailingProvider), "gemma4-31b");
        let err = client.propose_actions(&base_ctx()).await.unwrap_err();
        assert!(matches!(err, OrchestratorError::ProviderError(_)));
    }

    #[tokio::test]
    async fn brain_client_malformed_json_escalates() {
        let mock = Arc::new(MockProvider {
            response: "I don't know what to do here.".to_owned(),
        });
        let client = BrainLlmClient::new(mock, "gemma4-31b");
        let ps = client.propose_actions(&base_ctx()).await.unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].action_type, ActionType::EscalateToOperator);
    }

    #[tokio::test]
    async fn brain_client_multi_action_response() {
        let mock = Arc::new(MockProvider {
            response: r#"[
                {"action_type":"spawn_subagent","description":"research step","confidence":0.8,"tool_name":"researcher","tool_args":{"goal":"find RFCs"},"requires_approval":false},
                {"action_type":"send_notification","description":"notify operator","confidence":0.6,"tool_args":{"to":"mailbox_ops","message":"started"},"requires_approval":false}
            ]"#.to_owned(),
        });
        let client = BrainLlmClient::new(mock, "gemma4-31b");
        let ps = client.propose_actions(&base_ctx()).await.unwrap();
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].action_type, ActionType::SpawnSubagent);
        assert_eq!(ps[1].action_type, ActionType::SendNotification);
        assert_eq!(ps[1].tool_args.as_ref().unwrap()["to"], "mailbox_ops");
    }
}
