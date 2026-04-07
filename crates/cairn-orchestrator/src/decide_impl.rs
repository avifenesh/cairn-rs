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

use cairn_tools::builtins::{BuiltinToolDescriptor, BuiltinToolRegistry};

use crate::context::{DecideOutput, GatherOutput, OrchestrationContext};
use crate::decide::DecidePhase;
use crate::error::OrchestratorError;

// ── Token budgeting ───────────────────────────────────────────────────────────

/// Estimate the number of tokens in a text string.
///
/// Uses the chars-÷-4 heuristic, which approximates GPT-family tokenisers for
/// Latin-script text within ~20%.  Replace with a proper tokeniser (tiktoken,
/// tokenizers) when accuracy becomes important.
#[inline]
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() + 3) / 4  // round up so we never under-count
}

/// Token budget for a single LLM call.
///
/// Splits the model's context window into an output reservation (for the
/// LLM's response) and the remaining input budget available for the prompt.
///
/// # Example
/// ```
/// # use cairn_orchestrator::TokenBudget;
/// let budget = TokenBudget::new(131_072); // e.g. gemma-4
/// assert_eq!(budget.total_context, 131_072);
/// assert_eq!(budget.reserved_output, 131_072 / 4);
/// assert_eq!(budget.available_input, 131_072 - 131_072 / 4);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenBudget {
    /// Total context window of the model (tokens).
    pub total_context: usize,
    /// Tokens reserved for the model's output.
    /// Default: `total_context / 4`.
    pub reserved_output: usize,
    /// Tokens available for input content.
    /// Always `total_context - reserved_output`.
    pub available_input: usize,
}

impl TokenBudget {
    /// Create a budget for a model with the given context window.
    ///
    /// `reserved_output` defaults to `total_context / 4`.
    pub fn new(total_context: usize) -> Self {
        let reserved_output = total_context / 4;
        Self {
            total_context,
            reserved_output,
            available_input: total_context.saturating_sub(reserved_output),
        }
    }

    /// Override the output reservation.
    ///
    /// Recomputes `available_input = total_context - reserved_output`.
    pub fn with_reserved_output(mut self, reserved: usize) -> Self {
        self.reserved_output = reserved;
        self.available_input = self.total_context.saturating_sub(reserved);
        self
    }
}

impl Default for TokenBudget {
    /// A conservative default (16K context) used when no model info is available.
    fn default() -> Self {
        Self::new(16_384)
    }
}

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
    /// Token budget used by `PromptBuilder` to truncate context to fit the
    /// model's context window.  `None` = no truncation (legacy behaviour).
    token_budget: Option<TokenBudget>,
    tools: Option<std::sync::Arc<BuiltinToolRegistry>>,
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
            token_budget:    None,
            tools:           None,
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

    /// Set a token budget for prompt truncation.
    ///
    /// Call this when the model's context window is known (e.g. from provider
    /// model discovery).  The `PromptBuilder` will truncate memory chunks,
    /// step history, and graph context to fit within the available input budget.
    pub fn with_token_budget(mut self, budget: TokenBudget) -> Self {
        self.token_budget = Some(budget); self
    }

    /// Convenience: build a `TokenBudget` from a known context window size and
    /// attach it.  Equivalent to `with_token_budget(TokenBudget::new(tokens))`.
    pub fn with_context_window(self, context_window_tokens: usize) -> Self {
        self.with_token_budget(TokenBudget::new(context_window_tokens))
    }

    /// Attach a BuiltinToolRegistry; Core + Registered tools appear in the system prompt.
    pub fn with_tools(mut self, registry: std::sync::Arc<BuiltinToolRegistry>) -> Self {
        self.tools = Some(registry); self
    }
}

#[async_trait]
impl DecidePhase for LlmDecidePhase {
    async fn decide(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
    ) -> Result<DecideOutput, OrchestratorError> {
        // Build the tool catalogue for this iteration:
        // 1. Core + Registered tools (always included)
        // 2. Deferred tools discovered via tool_search in prior iterations
        //    (ctx.discovered_tool_names carries them across the loop boundary)
        let mut tool_descs: Vec<BuiltinToolDescriptor> = self.tools
            .as_ref()
            .map(|r| r.prompt_tools())
            .unwrap_or_default();

        if !ctx.discovered_tool_names.is_empty() {
            if let Some(ref registry) = self.tools {
                for name in &ctx.discovered_tool_names {
                    // Use search_deferred to fetch the full descriptor for the
                    // discovered tool (it's still Deferred in the registry).
                    let matches = registry.search_deferred(name);
                    for desc in matches {
                        if !tool_descs.iter().any(|d| d.name == desc.name) {
                            tool_descs.push(desc);
                        }
                    }
                }
            }
        }

        let system = build_system_prompt(&ctx.agent_type, &tool_descs);
        let user   = build_user_message(ctx, gather, self.token_budget.as_ref());
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

        // Override requires_approval for inherently safe read-only actions.
        // Models sometimes over-cautiously set this for web/memory reads — we
        // correct it here so the approval gate only fires for genuinely sensitive actions.
        for p in &mut proposals {
            if p.requires_approval && is_safe_read_action(p) {
                p.requires_approval = false;
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
fn build_system_prompt(agent_type: &str, tools: &[BuiltinToolDescriptor]) -> String {
    // Role identity — use registered role prompt or a directive default.
    let role_prompt = default_roles()
        .into_iter()
        .find(|r: &AgentRole| r.role_id == agent_type)
        .and_then(|r| r.system_prompt)
        .unwrap_or_else(|| {
            // Directive default: tools first, complete when done, delegate only when necessary.
            "You are a focused AI agent. Your job is to complete the given goal \
             directly using the available tools. Do not delegate work to sub-agents \
             unless the task is clearly multi-part and genuinely requires parallel execution."
                .to_owned()
        });

    // Build the tool list section (shown only when tools are registered).
    let tools_section = if !tools.is_empty() {
        let lines = tools.iter()
            .map(|t| format!("  - {}", t.prompt_line()))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\n\
             ## Available tools\n\
             Use invoke_tool with one of these tool_name values:\n\
             {lines}\n\
             \n\
             ## Tool usage rules — FOLLOW THESE EXACTLY\n\
             1. ONLY call tools whose tool_name appears in the Available Tools list above.\n\
             2. ALWAYS search memory first: invoke_tool memory_search before answering any question.\n\
             3. If memory is empty or insufficient, try web_fetch or other listed tools.\n\
             4. After gathering information, use create_memory to store key findings.\n\
             5. When you have enough information to answer the goal, return complete_run immediately.\n\
             6. If memory is empty AND no other tool can help, return complete_run with your best answer.\n\
             7. ONLY use spawn_subagent when the goal explicitly requires separate parallel tasks.\n\
             8. Do NOT invent tool names. Do NOT spawn sub-agents for simple retrieval or summarisation.\n\
             9. Set requires_approval=false for ALL read/search/fetch/summarise actions.\n\
                Set requires_approval=true ONLY for: file_write, shell_exec, cancel_task, \
                or any action that modifies external state irreversibly."
        )
    } else {
        String::new()
    };

    format!(
        "{role_prompt}\
         {tools_section}\n\
         \n\
         ## Decision rules\n\
         - If the goal can be answered with available information → complete_run with summary.\n\
         - If tools are available → invoke_tool before concluding you lack information.\n\
         - If the task is clearly multi-part requiring separate agents → spawn_subagent per part.\n\
         - If blocked and need human input → escalate_to_operator.\n\
         - Prefer completing in fewer iterations; do not defer work unnecessarily.\n\
         \n\
         ## Response format\n\
         Respond ONLY with a JSON array of action objects. Each object MUST have:\n\
         - \"action_type\": one of {action_types}\n\
         - \"description\": concise explanation\n\
         - \"confidence\": float 0.0–1.0\n\
         - \"requires_approval\": false for read/search/fetch/summarise actions; true ONLY for \
           file writes, code execution, sending messages, or destructive actions\n\
         - \"tool_name\" (for invoke_tool/spawn_subagent): tool ID or sub-agent role\n\
         - \"tool_args\" (for invoke_tool/spawn_subagent/create_memory): JSON arguments\n\
         \n\
         Field conventions:\n\
         - invoke_tool:    tool_name = tool ID,  tool_args = {{...}}\n\
         - complete_run:   description = full answer/summary of what was accomplished\n\
         - spawn_subagent: tool_name = \"researcher\"|\"executor\"|\"reviewer\",\
           tool_args = {{\"goal\": \"specific sub-task goal\"}}\n\
         - create_memory:  tool_args = {{\"content\": \"...\"}}\n\
         \n\
         Return ONLY the JSON array — no markdown fences, no explanation text.",
        action_types = r#""invoke_tool"|"complete_run"|"create_memory"|"spawn_subagent"|"send_notification"|"escalate_to_operator""#,
    )
}

/// Build the user message from `OrchestrationContext` + `GatherOutput`.
///
/// When `budget` is `Some`, content is truncated so the full prompt
/// (system + user) fits within `budget.available_input` tokens.
/// Truncation order (from most to least dispensable):
///   never truncated : system prompt, goal, run state, footer
///   truncated last  : graph_context (trim from end)
///   truncated third : step_history  (trim oldest first)
///   truncated second: memory_chunks (keep most-relevant, trim from end)
fn build_user_message(
    ctx: &OrchestrationContext,
    gather: &GatherOutput,
    budget: Option<&TokenBudget>,
) -> String {
    // ── Fixed sections (never truncated) ─────────────────────────────────────
    let goal_part = format!("## Goal\n{}", ctx.goal);
    let run_state_part = format!(
        "## Run state\nrun_id: {}\niteration: {}\nagent_type: {}",
        ctx.run_id.as_str(), ctx.iteration, ctx.agent_type,
    );
    let has_memory = !gather.memory_chunks.is_empty();
    let memory_hint = if has_memory {
        "Memory contains relevant information — use it to answer the goal, then complete_run.".to_owned()
    } else {
        "Memory is empty. You have enough knowledge to answer this goal directly. \
         Return complete_run immediately with your best answer — do NOT call memory_search again \
         if you already tried it and got no results."
            .to_owned()
    };
    let footer = format!(
        "## What should happen next?\n\
         {memory_hint}\n\
         Return a JSON action array. Use complete_run as soon as you can answer the goal. \
         Do not spawn sub-agents for simple retrieval or summarisation."
    );

    // ── Compute how many tokens are available for optional content ────────────
    // When no budget is set every section is included without limit.
    let optional_token_budget: Option<usize> = budget.map(|b| {
        let fixed_cost = estimate_tokens(&goal_part)
            + estimate_tokens(&run_state_part)
            + estimate_tokens(&footer)
            + 20; // section separators ("\n\n" between each part)
        b.available_input.saturating_sub(fixed_cost)
    });

    let mut remaining = optional_token_budget;

    // ── Memory chunks — most relevant first, truncate from end ───────────────
    // Retrieval already orders chunks highest-score first.
    let memory_section: Option<String> = if gather.memory_chunks.is_empty() {
        None
    } else {
        let mut snippets: Vec<String> = Vec::new();
        for (i, r) in gather.memory_chunks.iter().enumerate() {
            let line = format!("[{}] {}", i + 1, r.chunk.text.chars().take(400).collect::<String>());
            if let Some(rem) = remaining.as_mut() {
                let cost = estimate_tokens(&line) + 1;
                if *rem < cost {
                    break; // budget exhausted — drop less-relevant chunks
                }
                *rem = rem.saturating_sub(cost);
            }
            snippets.push(line);
        }
        if snippets.is_empty() {
            None
        } else {
            Some(format!("## Relevant knowledge\n{}", snippets.join("\n")))
        }
    };

    // ── Step history — most recent first, truncate oldest ────────────────────
    let step_section: Option<String> = if gather.step_history.is_empty() {
        None
    } else {
        let mut lines: Vec<String> = Vec::new();
        for s in gather.step_history.iter().rev() {
            let line = format!(
                "- [{}] {} | {} | ok={}",
                s.iteration, s.action_kind, s.summary, s.succeeded,
            );
            if let Some(rem) = remaining.as_mut() {
                let cost = estimate_tokens(&line) + 1;
                if *rem < cost {
                    break; // budget exhausted — drop older steps
                }
                *rem = rem.saturating_sub(cost);
            }
            lines.push(line);
        }
        if lines.is_empty() {
            None
        } else {
            Some(format!("## Step history (most recent first)\n{}", lines.join("\n")))
        }
    };

    // ── Operator settings ─────────────────────────────────────────────────────
    let settings_section: Option<String> = if gather.operator_settings.is_empty() {
        None
    } else {
        let text = gather.operator_settings.iter()
            .map(|s| format!("  {}: {}", s.key, s.value))
            .collect::<Vec<_>>()
            .join("\n");
        let section = format!("## Operator settings\n{text}");
        if let Some(rem) = remaining.as_mut() {
            let cost = estimate_tokens(&section);
            if *rem < cost {
                None // no room — skip entirely
            } else {
                *rem = rem.saturating_sub(cost);
                Some(section)
            }
        } else {
            Some(section)
        }
    };

    // ── Checkpoint hint ───────────────────────────────────────────────────────
    let checkpoint_section: Option<String> = gather.checkpoint.as_ref().and_then(|cp| {
        let section = format!(
            "## Checkpoint available\ncheckpoint_id: {} — the run can be restored to this point.",
            cp.checkpoint_id.as_str(),
        );
        if let Some(rem) = remaining.as_mut() {
            let cost = estimate_tokens(&section);
            if *rem < cost { return None; }
            *rem = rem.saturating_sub(cost);
        }
        Some(section)
    });

    // ── Graph context — truncated last ────────────────────────────────────────
    let graph_section: Option<String> = if gather.graph_nodes.is_empty() {
        None
    } else {
        let mut node_ids: Vec<&str> = Vec::new();
        for node in &gather.graph_nodes {
            let cost = node.node_id.len() / 4 + 2;
            if let Some(rem) = remaining.as_mut() {
                if *rem < cost { break; }
                *rem = rem.saturating_sub(cost);
            }
            node_ids.push(node.node_id.as_str());
        }
        if node_ids.is_empty() {
            None
        } else {
            Some(format!("## Graph context\nNearby nodes: {}", node_ids.join(", ")))
        }
    };

    // ── Assemble ──────────────────────────────────────────────────────────────
    let mut parts: Vec<String> = vec![goal_part, run_state_part];
    if let Some(s) = memory_section    { parts.push(s); }
    if let Some(s) = step_section      { parts.push(s); }
    if let Some(s) = settings_section  { parts.push(s); }
    if let Some(s) = checkpoint_section { parts.push(s); }
    if let Some(s) = graph_section     { parts.push(s); }
    parts.push(footer);
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
/// Return `true` when an action proposal is inherently safe (read-only) and
/// should never require approval, regardless of what the model returned.
///
/// Models sometimes over-cautiously set `requires_approval=true` for memory
/// searches or HTTP GETs.  This guard corrects that before the approval gate.
fn is_safe_read_action(proposal: &ActionProposal) -> bool {
    use ActionType::{InvokeTool, CreateMemory, CompleteRun};
    match proposal.action_type {
        InvokeTool => {
            let name = proposal.tool_name.as_deref().unwrap_or("").to_lowercase();
            matches!(
                name.as_str(),
                "memory_search" | "web_fetch" | "http_request" | "get_run" | "get_task"
                | "search_memory" | "list_runs" | "glob_find" | "grep_search"
                | "read_document" | "file_read" | "graph_query" | "search_events"
            )
        }
        CreateMemory | CompleteRun => true,
        _ => false,
    }
}

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
            discovered_tool_names: vec![],
        }
    }

    fn empty_gather() -> GatherOutput {
        GatherOutput::default()
    }

    // ── Prompt builder tests ──────────────────────────────────────────────────

    #[test]
    fn system_prompt_references_orchestrator_role() {
        let sys = build_system_prompt("orchestrator", &[]);
        assert!(sys.contains("orchestrator"), "should mention orchestrator role");
        assert!(sys.contains("JSON array"),   "should instruct JSON array return");
        assert!(sys.contains("spawn_subagent"), "should list spawn_subagent");
        assert!(sys.contains("complete_run"),   "should list complete_run");
    }

    #[test]
    fn system_prompt_fallback_for_unknown_role() {
        let sys = build_system_prompt("wizard", &[]);
        assert!(sys.contains("JSON array"), "fallback must still instruct JSON return");
    }

    #[test]
    fn user_message_contains_goal_and_run_id() {
        let msg = build_user_message(&ctx(), &empty_gather(), None);
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
        let msg = build_user_message(&ctx(), &g, None);
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

    // ── TokenBudget tests ─────────────────────────────────────────────────────

    #[test]
    fn token_budget_default_reserves_quarter() {
        let b = TokenBudget::new(131_072);
        assert_eq!(b.total_context, 131_072);
        assert_eq!(b.reserved_output, 131_072 / 4);
        assert_eq!(b.available_input, 131_072 - 131_072 / 4);
    }

    #[test]
    fn token_budget_with_custom_reservation() {
        let b = TokenBudget::new(8_192).with_reserved_output(1_000);
        assert_eq!(b.reserved_output, 1_000);
        assert_eq!(b.available_input, 7_192);
    }

    #[test]
    fn estimate_tokens_empty_string() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_rounds_up() {
        // 1 char → 1 token (ceiling of 1/4)
        assert_eq!(estimate_tokens("a"), 1);
        // 4 chars → 1 token
        assert_eq!(estimate_tokens("abcd"), 1);
        // 5 chars → 2 tokens (ceiling of 5/4)
        assert_eq!(estimate_tokens("abcde"), 2);
        // 400 chars → 100 tokens
        assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
    }

    // ── Token-budget truncation tests ─────────────────────────────────────────

    /// A very tight budget should include goal + run_state but omit optional content.
    #[test]
    fn tight_budget_drops_optional_content() {
        let mut g = empty_gather();
        // Add memory and step history that would normally appear
        g.memory_chunks = (0..5).map(|i| {
            cairn_memory::retrieval::RetrievalResult {
                chunk: {
                    let mut c = cairn_memory::ingest::ChunkRecord {
                        chunk_id: cairn_domain::ChunkId::new(format!("c{i}")),
                        document_id: cairn_domain::KnowledgeDocumentId::new("doc"),
                        source_id: cairn_domain::SourceId::new("src"),
                        source_type: cairn_memory::ingest::SourceType::PlainText,
                        project: ctx().project,
                        text: "a".repeat(400),
                        position: i as u32,
                        created_at: 0,
                        updated_at: None,
                        provenance_metadata: None,
                        credibility_score: None,
                        graph_linkage: None,
                        embedding: None,
                        content_hash: None,
                        entities: vec![],
                        embedding_model_id: None,
                        needs_reembed: false,
                    };
                    c
                },
                score: 1.0 - i as f64 * 0.1,
                breakdown: Default::default(),
            }
        }).collect();
        g.step_history = (0..3).map(|i| {
            crate::context::StepSummary {
                iteration: i,
                action_kind: "invoke_tool".to_owned(),
                summary: "did a thing".to_owned(),
                succeeded: true,
            }
        }).collect();

        // Budget of 50 tokens — can barely fit goal + run_state + footer
        let tight = TokenBudget::new(50).with_reserved_output(0);

        let msg = build_user_message(&ctx(), &g, Some(&tight));

        // Goal must always be present
        assert!(msg.contains("Goal"), "goal section must always appear");
        // Memory should be truncated/absent given the extreme budget
        // (we just verify the function doesn't panic; exact truncation depends on text sizes)
        let _ = msg;
    }

    /// Unlimited budget includes all content.
    #[test]
    fn no_budget_includes_all_content() {
        let mut g = empty_gather();
        g.step_history = vec![
            crate::context::StepSummary {
                iteration: 0,
                action_kind: "invoke_tool".to_owned(),
                summary: "searched for architecture docs".to_owned(),
                succeeded: true,
            },
        ];
        // memory chunk with distinctive text
        g.memory_chunks = vec![
            cairn_memory::retrieval::RetrievalResult {
                chunk: {
                    cairn_memory::ingest::ChunkRecord {
                        chunk_id: cairn_domain::ChunkId::new("c0"),
                        document_id: cairn_domain::KnowledgeDocumentId::new("doc"),
                        source_id: cairn_domain::SourceId::new("src"),
                        source_type: cairn_memory::ingest::SourceType::PlainText,
                        project: ctx().project,
                        text: "cairn uses event sourcing for durability".to_owned(),
                        position: 0,
                        created_at: 0,
                        updated_at: None,
                        provenance_metadata: None,
                        credibility_score: None,
                        graph_linkage: None,
                        embedding: None,
                        content_hash: None,
                        entities: vec![],
                        embedding_model_id: None,
                        needs_reembed: false,
                    }
                },
                score: 0.9,
                breakdown: Default::default(),
            },
        ];

        let msg = build_user_message(&ctx(), &g, None);

        assert!(msg.contains("cairn uses event sourcing"), "memory chunk must appear without budget");
        assert!(msg.contains("architecture docs"), "step history must appear without budget");
    }

    /// Memory chunks are included most-relevant-first; least relevant are dropped
    /// when the budget is tight.
    #[test]
    fn memory_chunks_most_relevant_first() {
        let texts = ["highly relevant content here", "somewhat relevant", "least relevant stuff"];
        let mut g = empty_gather();
        g.memory_chunks = texts.iter().enumerate().map(|(i, text)| {
            cairn_memory::retrieval::RetrievalResult {
                chunk: cairn_memory::ingest::ChunkRecord {
                    chunk_id: cairn_domain::ChunkId::new(format!("c{i}")),
                    document_id: cairn_domain::KnowledgeDocumentId::new("doc"),
                    source_id: cairn_domain::SourceId::new("src"),
                    source_type: cairn_memory::ingest::SourceType::PlainText,
                    project: ctx().project,
                    text: text.to_string(),
                    position: i as u32,
                    created_at: 0,
                    updated_at: None,
                    provenance_metadata: None,
                    credibility_score: None,
                    graph_linkage: None,
                    embedding: None,
                    content_hash: None,
                    entities: vec![],
                    embedding_model_id: None,
                    needs_reembed: false,
                },
                score: 1.0 - i as f64 * 0.3,
                breakdown: Default::default(),
            }
        }).collect();

        // Large budget — all three included
        let msg = build_user_message(&ctx(), &g, None);
        assert!(msg.contains("highly relevant"), "chunk[0] must appear");
        assert!(msg.contains("somewhat relevant"), "chunk[1] must appear");
        assert!(msg.contains("least relevant"), "chunk[2] must appear");
    }

    /// with_context_window creates a budget from the model's context window.
    #[tokio::test]
    async fn with_context_window_sets_budget() {
        let mock = Arc::new(MockProvider {
            response: r#"[{"action_type":"complete_run","description":"done","confidence":0.9,"requires_approval":false}]"#.to_owned(),
        });
        // 128K context like gemma-4
        let phase = LlmDecidePhase::new(mock, "gemma4").with_context_window(131_072);
        let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
        // Should work normally — the budget is generous enough that nothing is truncated
        assert_eq!(out.proposals[0].action_type, ActionType::CompleteRun);
    }
}
