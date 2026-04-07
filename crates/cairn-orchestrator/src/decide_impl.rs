//! LlmDecidePhase — concrete DECIDE phase implementation.
//!
//! Calls the brain LLM with a structured prompt built from
//! `OrchestrationContext` + `GatherOutput`, then parses the response into
//! `Vec<ActionProposal>` and wraps it in `DecideOutput`.
//!
//! # Flow
//! 1. Build system prompt — agent role identity + JSON format instruction.
//! 2. Build user message — goal + memory chunks + step history + settings.
//! 3. Call `GenerationProvider::generate` on the brain provider.
//! 4. Parse JSON response into `Vec<ActionProposal>` using `ResponseParser`.
//! 5. Retry once on parse failure (LLM sometimes needs a nudge).
//! 6. If second attempt also fails, return a `EscalateToOperator` proposal.
//! 7. Apply calibration offset if a `ConfidenceCalibrator` is provided.
//! 8. Emit `DecideOutput` with raw response retained for audit.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use cairn_domain::{
    ActionProposal, ActionType,
    agent_roles::{AgentRole, default_roles},
    providers::{GenerationProvider, ProviderBindingSettings},
};

use crate::context::{DecideOutput, GatherOutput, OrchestrationContext};
use crate::decide::DecidePhase;
use crate::error::OrchestratorError;

// ── LlmDecidePhase ────────────────────────────────────────────────────────────

/// Production implementation of the DECIDE phase.
///
/// Thread-safe: all fields are `Arc` or immutable.
pub struct LlmDecidePhase {
    provider:    Arc<dyn GenerationProvider>,
    model_id:    String,
    settings:    ProviderBindingSettings,
    /// Optional fixed confidence offset applied to every proposal
    /// (replaces a full `ConfidenceCalibrator` when historical data is absent).
    confidence_bias: f64,
}

impl LlmDecidePhase {
    /// Create with the given provider and model.
    pub fn new(provider: Arc<dyn GenerationProvider>, model_id: impl Into<String>) -> Self {
        Self {
            provider,
            model_id:        model_id.into(),
            settings:        ProviderBindingSettings {
                max_output_tokens: Some(2048),
                ..Default::default()
            },
            confidence_bias: 0.0,
        }
    }

    /// Override generation settings (e.g. temperature, max_output_tokens).
    pub fn with_settings(mut self, s: ProviderBindingSettings) -> Self {
        self.settings = s; self
    }

    /// Apply a fixed bias to every proposal's confidence (clamped to [0, 1]).
    /// Positive = boost, negative = penalise.  Use when a full calibrator
    /// is not wired up yet.
    pub fn with_confidence_bias(mut self, bias: f64) -> Self {
        self.confidence_bias = bias; self
    }
}

#[async_trait]
impl DecidePhase for LlmDecidePhase {
    async fn decide(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
    ) -> Result<DecideOutput, OrchestratorError> {
        let system = build_system_prompt(&ctx.agent_type);
        let user   = build_user_message(ctx, gather);
        let messages = vec![
            serde_json::json!({ "role": "system", "content": system }),
            serde_json::json!({ "role": "user",   "content": user   }),
        ];

        let t0 = Instant::now();
        let resp = self.provider
            .generate(&self.model_id, messages.clone(), &self.settings)
            .await
            .map_err(|e| OrchestratorError::Decide(e.to_string()))?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let raw_response = resp.text.clone();

        // Parse — retry once if the first attempt yields only an escalation
        // caused by malformed JSON (the LLM sometimes wraps in prose on first try).
        let mut proposals = parse_proposals(&resp.text);
        if is_fallback_escalation(&proposals) {
            // Retry: explicitly ask the LLM to output only JSON
            let retry_user = format!(
                "{user}\n\n⚠️ Your last response was not valid JSON. \
                 Return ONLY a JSON array of action objects — no prose, no markdown."
            );
            let retry_messages = vec![
                serde_json::json!({ "role": "system", "content": system }),
                serde_json::json!({ "role": "user",   "content": retry_user }),
            ];
            match self.provider.generate(&self.model_id, retry_messages, &self.settings).await {
                Ok(r2) => {
                    let second = parse_proposals(&r2.text);
                    if !is_fallback_escalation(&second) {
                        proposals = second;
                    }
                    // If second attempt also fails, keep the escalation from the first parse
                }
                Err(_) => {
                    // Retry LLM call failed — keep the escalation from the first parse
                }
            }
        }

        // Apply confidence bias
        if self.confidence_bias.abs() > f64::EPSILON {
            for p in &mut proposals {
                p.confidence = (p.confidence + self.confidence_bias).clamp(0.0, 1.0);
            }
        }

        let requires_approval = proposals.iter().any(|p| p.requires_approval);
        let calibrated_confidence = proposals
            .iter()
            .map(|p| p.confidence)
            .fold(0.0_f64, f64::max);

        Ok(DecideOutput {
            raw_response,
            proposals,
            calibrated_confidence,
            requires_approval,
            model_id: self.model_id.clone(),
            latency_ms,
        })
    }
}

// ── Prompt builders ───────────────────────────────────────────────────────────

/// Build the system prompt for the given agent type.
///
/// Uses `default_roles()` to look up the canonical system-prompt fragment for
/// the matching role.  Falls back to a generic orchestrator prompt if the role
/// is not registered.
fn build_system_prompt(agent_type: &str) -> String {
    let role_prompt = default_roles()
        .into_iter()
        .find(|r: &AgentRole| r.role_id == agent_type)
        .and_then(|r| r.system_prompt)
        .unwrap_or_else(|| {
            "You are an AI orchestrator. Break down complex goals, delegate to sub-agents, \
             and synthesise results."
                .to_owned()
        });

    format!(
        "{role_prompt}\n\
         \n\
         ## Response format\n\
         Respond ONLY with a JSON array of action objects. Each object MUST have:\n\
         - \"action_type\": one of {action_types}\n\
         - \"description\": concise explanation of this action\n\
         - \"confidence\": float 0.0–1.0\n\
         - \"requires_approval\": true only when operator approval is needed\n\
         - \"tool_name\" (optional): tool ID or sub-agent role for spawn/invoke actions\n\
         - \"tool_args\" (optional): JSON arguments for the tool\n\
         \n\
         Field conventions:\n\
         - spawn_subagent: tool_name = \"researcher\"|\"executor\"|\"reviewer\",\
           tool_args = {{\"goal\": \"...\"}}\n\
         - invoke_tool:    tool_name = tool ID,  tool_args = {{...}}\n\
         - create_memory:  tool_args = {{\"content\": \"...\"}}\n\
         - send_notification: tool_args = {{\"to\": \"id\", \"message\": \"...\"}}\n\
         - complete_run:   description = final summary\n\
         - escalate_to_operator: description = why, requires_approval = true\n\
         \n\
         Return ONLY the JSON array — no markdown fences, no prose.",
        action_types = r#""spawn_subagent"|"invoke_tool"|"create_memory"|"send_notification"|"complete_run"|"escalate_to_operator""#,
    )
}

/// Build the user message from `OrchestrationContext` + `GatherOutput`.
fn build_user_message(ctx: &OrchestrationContext, gather: &GatherOutput) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!("## Goal\n{}", ctx.goal));

    parts.push(format!(
        "## Run state\nrun_id: {}\niteration: {}\nagent_type: {}",
        ctx.run_id.as_str(), ctx.iteration, ctx.agent_type,
    ));

    // Step history — most recent steps first so the LLM sees recent context
    if !gather.step_history.is_empty() {
        let lines: Vec<String> = gather.step_history.iter().rev().map(|s| {
            format!(
                "- [{}] {} | {} | ok={}",
                s.iteration, s.action_kind, s.summary, s.succeeded,
            )
        }).collect();
        parts.push(format!("## Step history (most recent first)\n{}", lines.join("\n")));
    }

    // Memory chunks — show text only
    if !gather.memory_chunks.is_empty() {
        let snippets: Vec<String> = gather.memory_chunks.iter().enumerate().map(|(i, r)| {
            format!("[{}] {}", i + 1, r.chunk.text.chars().take(300).collect::<String>())
        }).collect();
        parts.push(format!("## Relevant knowledge\n{}", snippets.join("\n")));
    }

    // Operator settings — surface key defaults
    if !gather.operator_settings.is_empty() {
        let settings: Vec<String> = gather.operator_settings.iter().map(|s| {
            format!("  {}: {}", s.key, s.value)
        }).collect();
        parts.push(format!("## Operator settings\n{}", settings.join("\n")));
    }

    // Checkpoint hint
    if let Some(cp) = &gather.checkpoint {
        parts.push(format!(
            "## Checkpoint available\ncheckpoint_id: {} — the run can be restored to this point.",
            cp.checkpoint_id.as_str(),
        ));
    }

    // Graph nodes (brief)
    if !gather.graph_nodes.is_empty() {
        let node_ids: Vec<&str> = gather.graph_nodes.iter().map(|n| n.node_id.as_str()).collect();
        parts.push(format!("## Graph context\nNearby nodes: {}", node_ids.join(", ")));
    }

    parts.push("## What should happen next?\nReturn a JSON action array.".to_owned());

    parts.join("\n\n")
}

// ── Response parsing (inlined — avoids re-exporting cairn_runtime internals) ──

/// Parse the LLM's raw text into `ActionProposal` values.
///
/// On complete parse failure returns a single `EscalateToOperator` proposal.
fn parse_proposals(raw: &str) -> Vec<ActionProposal> {
    let cleaned = strip_markdown_fence(raw.trim());

    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(cleaned) {
        let proposals: Vec<ActionProposal> = arr.into_iter().filter_map(parse_one).collect();
        if !proposals.is_empty() {
            return proposals;
        }
    }

    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(cleaned) {
        if let Some(p) = parse_one(obj) {
            return vec![p];
        }
    }

    // Fallback escalation
    vec![ActionProposal::escalate(
        format!("LLM returned a non-JSON response (first 200 chars): {}", &raw[..raw.len().min(200)]),
        0.0,
    )]
}

fn strip_markdown_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")) {
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim();
        }
    }
    s
}

fn parse_one(v: serde_json::Value) -> Option<ActionProposal> {
    let obj = v.as_object()?;
    let action_type = match obj.get("action_type")?.as_str()? {
        "spawn_subagent"       => ActionType::SpawnSubagent,
        "invoke_tool"          => ActionType::InvokeTool,
        "create_memory"        => ActionType::CreateMemory,
        "send_notification"    => ActionType::SendNotification,
        "complete_run"         => ActionType::CompleteRun,
        "escalate_to_operator" => ActionType::EscalateToOperator,
        _ => return None,
    };
    let description    = obj.get("description").and_then(|d| d.as_str()).unwrap_or("").to_owned();
    let confidence     = obj.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.5).clamp(0.0, 1.0);
    let requires_approval = obj.get("requires_approval").and_then(|r| r.as_bool()).unwrap_or(false);
    let tool_name      = obj.get("tool_name").and_then(|n| n.as_str()).map(str::to_owned);
    let tool_args      = obj.get("tool_args").cloned();

    Some(ActionProposal { action_type, description, confidence, tool_name, tool_args, requires_approval })
}

/// Returns true when `proposals` is exactly a single `EscalateToOperator`
/// whose description starts with "LLM returned a non-JSON response" —
/// i.e. the fallback we emit on parse failure.
fn is_fallback_escalation(proposals: &[ActionProposal]) -> bool {
    proposals.len() == 1
        && proposals[0].action_type == ActionType::EscalateToOperator
        && proposals[0].confidence == 0.0
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::providers::{GenerationResponse, ProviderAdapterError};

    // ── Mock provider ─────────────────────────────────────────────────────────

    struct MockProvider { response: String }

    #[async_trait]
    impl GenerationProvider for MockProvider {
        async fn generate(
            &self,
            _model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Ok(GenerationResponse {
                text:          self.response.clone(),
                input_tokens:  Some(150),
                output_tokens: Some(100),
                model_id:      "test-brain".to_owned(),
                tool_calls:    vec![],
            })
        }
    }

    struct FailingProvider;

    #[async_trait]
    impl GenerationProvider for FailingProvider {
        async fn generate(
            &self, _: &str, _: Vec<serde_json::Value>, _: &ProviderBindingSettings,
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Err(ProviderAdapterError::TransportFailure("offline".to_owned()))
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn ctx() -> OrchestrationContext {
        OrchestrationContext {
            project:         cairn_domain::ProjectKey::new("t", "w", "p"),
            session_id:      cairn_domain::SessionId::new("sess_1"),
            run_id:          cairn_domain::RunId::new("run_1"),
            task_id:         None,
            iteration:       0,
            goal:            "Summarise the cairn-rs architecture document.".to_owned(),
            agent_type:      "orchestrator".to_owned(),
            run_started_at_ms: 0,
        }
    }

    fn empty_gather() -> GatherOutput {
        GatherOutput::default()
    }

    // ── Prompt builder tests ──────────────────────────────────────────────────

    #[test]
    fn system_prompt_references_orchestrator_role() {
        let sys = build_system_prompt("orchestrator");
        assert!(sys.contains("orchestrator"), "should mention orchestrator role");
        assert!(sys.contains("JSON array"),   "should instruct JSON array return");
        assert!(sys.contains("spawn_subagent"), "should list spawn_subagent");
        assert!(sys.contains("complete_run"),   "should list complete_run");
    }

    #[test]
    fn system_prompt_fallback_for_unknown_role() {
        let sys = build_system_prompt("wizard");
        assert!(sys.contains("JSON array"), "fallback must still instruct JSON return");
    }

    #[test]
    fn user_message_contains_goal_and_run_id() {
        let msg = build_user_message(&ctx(), &empty_gather());
        assert!(msg.contains("cairn-rs architecture"), "goal must appear");
        assert!(msg.contains("run_1"),                 "run_id must appear");
        assert!(msg.contains("orchestrator"),          "agent_type must appear");
    }

    #[test]
    fn user_message_embeds_step_history() {
        let mut g = empty_gather();
        g.step_history = vec![
            crate::context::StepSummary {
                iteration:   0,
                action_kind: "invoke_tool".to_owned(),
                summary:     "searched for architecture docs".to_owned(),
                succeeded:   true,
            },
        ];
        let msg = build_user_message(&ctx(), &g);
        assert!(msg.contains("architecture docs"), "step history must appear");
        assert!(msg.contains("invoke_tool"),       "action kind must appear");
    }

    // ── Response parser tests ─────────────────────────────────────────────────

    #[test]
    fn parse_well_formed_response() {
        let raw = r#"[
          {"action_type":"spawn_subagent","description":"delegate research","confidence":0.88,
           "tool_name":"researcher","tool_args":{"goal":"summarise RFCs"},"requires_approval":false},
          {"action_type":"complete_run","description":"done","confidence":0.95,"requires_approval":false}
        ]"#;
        let proposals = parse_proposals(raw);
        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].action_type, ActionType::SpawnSubagent);
        assert_eq!(proposals[0].tool_name.as_deref(), Some("researcher"));
        assert!((proposals[0].confidence - 0.88).abs() < 1e-9);
        assert_eq!(proposals[1].action_type, ActionType::CompleteRun);
    }

    #[test]
    fn parse_malformed_response_returns_escalate() {
        let raw = "I'm not sure what to do. Can you give me more context?";
        let proposals = parse_proposals(raw);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].action_type, ActionType::EscalateToOperator);
        assert!(proposals[0].requires_approval, "escalation must require approval");
        assert_eq!(proposals[0].confidence, 0.0, "fallback confidence must be 0");
        assert!(is_fallback_escalation(&proposals));
    }

    #[test]
    fn parse_strips_markdown_fence() {
        let raw = "```json\n[{\"action_type\":\"complete_run\",\"description\":\"all done\",\"confidence\":1.0,\"requires_approval\":false}]\n```";
        let proposals = parse_proposals(raw);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].action_type, ActionType::CompleteRun);
        assert!(!is_fallback_escalation(&proposals));
    }

    #[test]
    fn parse_unknown_action_type_filtered_then_escalates() {
        let raw = r#"[{"action_type":"nuke_database","description":"bad","confidence":0.9,"requires_approval":false}]"#;
        let proposals = parse_proposals(raw);
        // All filtered → escalate
        assert_eq!(proposals[0].action_type, ActionType::EscalateToOperator);
    }

    // ── BrainLlmClient integration tests ─────────────────────────────────────

    #[tokio::test]
    async fn decide_with_well_formed_json() {
        let mock = Arc::new(MockProvider {
            response: r#"[{"action_type":"spawn_subagent","description":"research step","confidence":0.82,"tool_name":"researcher","tool_args":{"goal":"analyse docs"},"requires_approval":false}]"#.to_owned(),
        });
        let phase = LlmDecidePhase::new(mock, "cyankiwi/gemma-4-31B-it-AWQ-4bit");
        let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();

        assert_eq!(out.proposals.len(), 1);
        assert_eq!(out.proposals[0].action_type, ActionType::SpawnSubagent);
        assert_eq!(out.proposals[0].tool_name.as_deref(), Some("researcher"));
        assert!(!out.requires_approval);
        assert!((out.calibrated_confidence - 0.82).abs() < 1e-9);
        assert_eq!(out.model_id, "cyankiwi/gemma-4-31B-it-AWQ-4bit");
    }

    #[tokio::test]
    async fn decide_with_malformed_json_retries_and_escalates() {
        // Both call attempts return prose — second retry also fails
        let mock = Arc::new(MockProvider {
            response: "I need more information about the task before I can proceed.".to_owned(),
        });
        let phase = LlmDecidePhase::new(mock, "gemma4");
        let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();

        // Must succeed (not Err) and produce escalation
        assert_eq!(out.proposals.len(), 1);
        assert_eq!(out.proposals[0].action_type, ActionType::EscalateToOperator);
        assert!(out.proposals[0].requires_approval);
    }

    #[tokio::test]
    async fn decide_propagates_provider_error() {
        let phase = LlmDecidePhase::new(Arc::new(FailingProvider), "gemma4");
        let err = phase.decide(&ctx(), &empty_gather()).await.unwrap_err();
        assert!(matches!(err, OrchestratorError::Decide(_)));
    }

    #[tokio::test]
    async fn decide_requires_approval_when_proposal_flagged() {
        let mock = Arc::new(MockProvider {
            response: r#"[{"action_type":"escalate_to_operator","description":"unsure","confidence":0.3,"requires_approval":true}]"#.to_owned(),
        });
        let phase = LlmDecidePhase::new(mock, "gemma4");
        let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
        assert!(out.requires_approval, "requires_approval must be true when any proposal is flagged");
    }

    #[tokio::test]
    async fn decide_applies_confidence_bias() {
        let mock = Arc::new(MockProvider {
            response: r#"[{"action_type":"complete_run","description":"done","confidence":0.5,"requires_approval":false}]"#.to_owned(),
        });
        let phase = LlmDecidePhase::new(mock, "gemma4").with_confidence_bias(0.2);
        let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
        assert!((out.proposals[0].confidence - 0.7).abs() < 1e-9, "bias should increase confidence");
    }
}
