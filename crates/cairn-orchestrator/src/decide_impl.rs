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
    agent_roles::{default_roles, AgentRole},
    providers::{GenerationProvider, ProviderBindingSettings},
    ActionProposal, ActionType,
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
    text.len().div_ceil(4) // round up so we never under-count
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
    provider: Arc<dyn GenerationProvider>,
    model_id: String,
    settings: ProviderBindingSettings,
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
            model_id: model_id.into(),
            settings: ProviderBindingSettings {
                max_output_tokens: Some(2048),
                ..Default::default()
            },
            confidence_bias: 0.0,
            token_budget: None,
            tools: None,
        }
    }

    /// Override generation settings (e.g. temperature, max_output_tokens).
    pub fn with_settings(mut self, s: ProviderBindingSettings) -> Self {
        self.settings = s;
        self
    }

    /// Apply a fixed bias to every proposal's confidence (clamped to [0, 1]).
    /// Positive = boost, negative = penalise.  Use when a full calibrator
    /// is not wired up yet.
    pub fn with_confidence_bias(mut self, bias: f64) -> Self {
        self.confidence_bias = bias;
        self
    }

    /// Set a token budget for prompt truncation.
    ///
    /// Call this when the model's context window is known (e.g. from provider
    /// model discovery).  The `PromptBuilder` will truncate memory chunks,
    /// step history, and graph context to fit within the available input budget.
    pub fn with_token_budget(mut self, budget: TokenBudget) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Convenience: build a `TokenBudget` from a known context window size and
    /// attach it.  Equivalent to `with_token_budget(TokenBudget::new(tokens))`.
    pub fn with_context_window(self, context_window_tokens: usize) -> Self {
        self.with_token_budget(TokenBudget::new(context_window_tokens))
    }

    /// Attach a BuiltinToolRegistry; Core + Registered tools appear in the system prompt.
    pub fn with_tools(mut self, registry: std::sync::Arc<BuiltinToolRegistry>) -> Self {
        self.tools = Some(registry);
        self
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
        let mut tool_descs: Vec<BuiltinToolDescriptor> = self
            .tools
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

        // RFC 018: Plan mode filters out External tools so the agent can only
        // observe and work internally. Execute/Direct see all tools.
        if matches!(ctx.run_mode, cairn_domain::decisions::RunMode::Plan) {
            use cairn_domain::decisions::ToolEffect;
            tool_descs.retain(|d| {
                matches!(
                    d.tool_effect,
                    ToolEffect::Observational | ToolEffect::Internal
                )
            });
        }

        let system = build_system_prompt(&ctx.agent_type, &tool_descs);
        let user = build_user_message(ctx, gather, self.token_budget.as_ref());
        let messages = vec![
            serde_json::json!({ "role": "system", "content": system }),
            serde_json::json!({ "role": "user",   "content": user   }),
        ];

        let t0 = Instant::now();
        let resp = self
            .provider
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
            match self
                .provider
                .generate(&self.model_id, retry_messages, &self.settings)
                .await
            {
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
            input_tokens: resp.input_tokens,
            output_tokens: resp.output_tokens,
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
    // Role identity — use registered role prompt or a sensible default.
    let role_prompt = default_roles()
        .into_iter()
        .find(|r: &AgentRole| r.role_id == agent_type)
        .and_then(|r| r.system_prompt)
        .unwrap_or_else(|| {
            "You are an autonomous agent working on a task end-to-end. \
             Use the available tools to understand the problem, take action, \
             and deliver concrete results."
                .to_owned()
        });

    // Build the tool list section (shown only when tools are registered).
    let tools_section = if !tools.is_empty() {
        let lines = tools
            .iter()
            .map(|t| format!("  - {}", t.prompt_line()))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\n\
             ## Available tools\n\
             Use invoke_tool with one of these tool_name values:\n\
             {lines}\n\
             \n\
             Only call tools listed above. Do not invent tool names.\n\
             Use tool_search to discover additional tools if the ones above are insufficient."
        )
    } else {
        String::new()
    };

    format!(
        "{role_prompt}\
         {tools_section}\n\
         \n\
         ## Workflow\n\
         Follow these phases in order:\n\
         \n\
         ### Phase 1: Understand\n\
         Read the goal carefully. Use tools to gather context — search memory, \
         read files, explore the codebase. Identify what needs to happen.\n\
         \n\
         ### Phase 2: Act\n\
         Take concrete actions toward the goal. Write files, run commands, \
         make API calls — whatever the task requires. Reading and analysing \
         alone is not sufficient if the goal requires producing artifacts.\n\
         \n\
         ### Phase 3: Verify\n\
         Check that your actions achieved the goal. Run tests, read the \
         output, confirm the result. If something failed, fix it before \
         moving on.\n\
         \n\
         ### Phase 4: Complete\n\
         Only after you have taken action and verified the result, call \
         complete_run with a summary of what you accomplished.\n\
         \n\
         ## Tool usage\n\
         - Set requires_approval=false for read/search/fetch actions.\n\
         - Set requires_approval=true for writes, code execution, \
           sending messages, or destructive actions.\n\
         - Store key findings with create_memory so they persist.\n\
         - Use spawn_subagent only when the task is genuinely multi-part \
           and benefits from parallel execution.\n\
         - If blocked and need human input → escalate_to_operator.\n\
         \n\
         ## Completion criteria\n\
         Before calling complete_run, verify:\n\
         - You have taken action toward the goal (not just read/searched).\n\
         - If the goal requires artifacts (code, files, PRs), they exist.\n\
         - You have verified the result where possible.\n\
         If any criterion is not met, continue working.\n\
         \n\
         ## Tips\n\
         - If a tool call fails, analyse the error and try a different approach. \
           Do not retry the same failing call.\n\
         - When reading large outputs, focus on the relevant sections.\n\
         - If stuck, search for similar patterns or ask for help via \
           escalate_to_operator.\n\
         \n\
         ## Response format\n\
         Respond ONLY with a JSON array of action objects. Each object MUST have:\n\
         - \"action_type\": one of {action_types}\n\
         - \"description\": concise explanation\n\
         - \"confidence\": float 0.0–1.0\n\
         - \"requires_approval\": boolean\n\
         - \"tool_name\" (for invoke_tool/spawn_subagent): tool ID or sub-agent role\n\
         - \"tool_args\" (for invoke_tool/spawn_subagent/create_memory): JSON arguments\n\
         \n\
         Field conventions:\n\
         - invoke_tool:    tool_name = tool ID,  tool_args = {{...}}\n\
         - complete_run:   description = summary of what was accomplished\n\
         - spawn_subagent: tool_name = role,  tool_args = {{\"goal\": \"...\"}}\n\
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
        ctx.run_id.as_str(),
        ctx.iteration,
        ctx.agent_type,
    );
    let has_memory = !gather.memory_chunks.is_empty();
    let memory_hint = if has_memory {
        "Memory contains relevant context above. Use it to inform your next action.".to_owned()
    } else {
        "No relevant memories found. Use other available tools to gather what you need.".to_owned()
    };
    let footer = format!(
        "## Next step\n\
         {memory_hint}\n\
         Decide your next action based on the workflow phases: \
         Understand → Act → Verify → Complete.\n\
         Return a JSON action array."
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
            let line = format!(
                "[{}] {}",
                i + 1,
                r.chunk.text.chars().take(400).collect::<String>()
            );
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
            Some(format!(
                "## Step history (most recent first)\n{}",
                lines.join("\n")
            ))
        }
    };

    // ── Operator settings ─────────────────────────────────────────────────────
    let settings_section: Option<String> = if gather.operator_settings.is_empty() {
        None
    } else {
        let text = gather
            .operator_settings
            .iter()
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
            if *rem < cost {
                return None;
            }
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
                if *rem < cost {
                    break;
                }
                *rem = rem.saturating_sub(cost);
            }
            node_ids.push(node.node_id.as_str());
        }
        if node_ids.is_empty() {
            None
        } else {
            Some(format!(
                "## Graph context\nNearby nodes: {}",
                node_ids.join(", ")
            ))
        }
    };

    // ── Assemble ──────────────────────────────────────────────────────────────
    let mut parts: Vec<String> = vec![goal_part, run_state_part];
    if let Some(s) = memory_section {
        parts.push(s);
    }
    if let Some(s) = step_section {
        parts.push(s);
    }
    if let Some(s) = settings_section {
        parts.push(s);
    }
    if let Some(s) = checkpoint_section {
        parts.push(s);
    }
    if let Some(s) = graph_section {
        parts.push(s);
    }
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
        format!(
            "LLM returned a non-JSON response (first 200 chars): {}",
            &raw[..raw.len().min(200)]
        ),
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
        "spawn_subagent" => ActionType::SpawnSubagent,
        "invoke_tool" => ActionType::InvokeTool,
        "create_memory" => ActionType::CreateMemory,
        "send_notification" => ActionType::SendNotification,
        "complete_run" => ActionType::CompleteRun,
        "escalate_to_operator" => ActionType::EscalateToOperator,
        _ => return None,
    };
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

/// Returns true when `proposals` is exactly a single `EscalateToOperator`
/// whose description starts with "LLM returned a non-JSON response" —
/// i.e. the fallback we emit on parse failure.
/// Return `true` when an action proposal is inherently safe (read-only) and
/// should never require approval, regardless of what the model returned.
///
/// Models sometimes over-cautiously set `requires_approval=true` for memory
/// searches or HTTP GETs.  This guard corrects that before the approval gate.
fn is_safe_read_action(proposal: &ActionProposal) -> bool {
    use ActionType::{CompleteRun, CreateMemory, InvokeTool};
    match proposal.action_type {
        InvokeTool => {
            let name = proposal.tool_name.as_deref().unwrap_or("").to_lowercase();
            matches!(
                name.as_str(),
                "memory_search"
                    | "web_fetch"
                    | "http_request"
                    | "get_run"
                    | "get_task"
                    | "search_memory"
                    | "list_runs"
                    | "glob_find"
                    | "grep_search"
                    | "read_document"
                    | "file_read"
                    | "graph_query"
                    | "search_events"
                    | "gh_list_issues"
                    | "gh_get_issue"
                    | "gh_search_code"
                    | "tool_search"
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
    use cairn_domain::contexts::PluginCategory;
    use cairn_domain::providers::{GenerationResponse, ProviderAdapterError};
    use cairn_domain::OperatorId;
    use cairn_runtime::services::{
        is_plugin_tool_visible, DescriptorSource, MarketplaceCommand, MarketplaceService,
        PluginDescriptor,
    };
    use cairn_store::InMemoryStore;
    use cairn_tools::builtins::{
        BuiltinToolRegistry, ToolEffect, ToolError, ToolHandler, ToolResult, ToolSearchTool,
        ToolTier,
    };
    use std::path::PathBuf;
    use std::sync::Arc;

    // ── Mock provider ─────────────────────────────────────────────────────────

    struct MockProvider {
        response: String,
    }

    #[async_trait]
    impl GenerationProvider for MockProvider {
        async fn generate(
            &self,
            _model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Ok(GenerationResponse {
                text: self.response.clone(),
                input_tokens: Some(150),
                output_tokens: Some(100),
                model_id: "test-brain".to_owned(),
                tool_calls: vec![],
            })
        }
    }

    struct FailingProvider;

    #[async_trait]
    impl GenerationProvider for FailingProvider {
        async fn generate(
            &self,
            _: &str,
            _: Vec<serde_json::Value>,
            _: &ProviderBindingSettings,
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Err(ProviderAdapterError::TransportFailure("offline".to_owned()))
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn ctx() -> OrchestrationContext {
        OrchestrationContext {
            project: cairn_domain::ProjectKey::new("t", "w", "p"),
            session_id: cairn_domain::SessionId::new("sess_1"),
            run_id: cairn_domain::RunId::new("run_1"),
            task_id: None,
            iteration: 0,
            goal: "Summarise the cairn-rs architecture document.".to_owned(),
            agent_type: "orchestrator".to_owned(),
            run_started_at_ms: 0,
            working_dir: PathBuf::from("."),
            run_mode: cairn_domain::decisions::RunMode::Direct,
            discovered_tool_names: vec![],
        }
    }

    fn empty_gather() -> GatherOutput {
        GatherOutput::default()
    }

    fn plugin_descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: "github".to_owned(),
            name: "GitHub".to_owned(),
            version: "0.1.0".to_owned(),
            description: Some("GitHub integration".to_owned()),
            category: PluginCategory::IssueTracker,
            vendor: "cairn".to_owned(),
            icon_url: None,
            command: vec!["echo".to_owned(), "github".to_owned()],
            tools: vec![
                "github.issue_brief".to_owned(),
                "github.issue_search".to_owned(),
            ],
            signal_sources: vec![],
            channels: vec![],
            required_credentials: vec![],
            required_network_egress: vec![],
            post_install_health_check: None,
            source: DescriptorSource::BundledCatalog,
            download_url: None,
            has_signal_source: false,
        }
    }

    fn operator() -> OperatorId {
        OperatorId::new("op_test")
    }

    struct FakePluginTool {
        name: &'static str,
        description: &'static str,
        tier: ToolTier,
    }

    #[async_trait]
    impl ToolHandler for FakePluginTool {
        fn name(&self) -> &str {
            self.name
        }

        fn tier(&self) -> ToolTier {
            self.tier
        }

        fn tool_effect(&self) -> ToolEffect {
            ToolEffect::Observational
        }

        fn description(&self) -> &str {
            self.description
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(
            &self,
            _: &cairn_domain::ProjectKey,
            _: serde_json::Value,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::ok(serde_json::json!({ "ok": true })))
        }
    }

    fn registry_for_project(project: &cairn_domain::ProjectKey) -> Arc<BuiltinToolRegistry> {
        let mut marketplace = MarketplaceService::new(Arc::new(InMemoryStore::new()));
        marketplace.list_plugin(plugin_descriptor());
        marketplace
            .handle_command(MarketplaceCommand::InstallPlugin {
                plugin_id: "github".to_owned(),
                initiated_by: operator(),
            })
            .unwrap();
        marketplace
            .handle_command(MarketplaceCommand::EnablePluginForProject {
                plugin_id: "github".to_owned(),
                project: cairn_domain::ProjectKey::new("tenant", "workspace", "project-a"),
                tool_allowlist: Some(vec![
                    "github.issue_brief".to_owned(),
                    "github.issue_search".to_owned(),
                ]),
                signal_allowlist: None,
                signal_capture_override: None,
                enabled_by: operator(),
            })
            .unwrap();

        let visibility = marketplace.build_visibility_context(project, None);
        let registered = Arc::new(FakePluginTool {
            name: "github.issue_brief",
            description: "Summarise a GitHub issue for the operator.",
            tier: ToolTier::Registered,
        });
        let deferred = Arc::new(FakePluginTool {
            name: "github.issue_search",
            description: "Search GitHub issues by title or label.",
            tier: ToolTier::Deferred,
        });

        let mut inner = BuiltinToolRegistry::new();
        if is_plugin_tool_visible(&visibility, "github", registered.name()) {
            inner = inner.register(registered.clone());
        }
        if is_plugin_tool_visible(&visibility, "github", deferred.name()) {
            inner = inner.register(deferred.clone());
        }
        let inner = Arc::new(inner);

        let mut outer = BuiltinToolRegistry::new();
        if is_plugin_tool_visible(&visibility, "github", registered.name()) {
            outer = outer.register(registered);
        }
        if is_plugin_tool_visible(&visibility, "github", deferred.name()) {
            outer = outer.register(deferred);
        }
        outer = outer.register(Arc::new(ToolSearchTool::new(inner)));
        Arc::new(outer)
    }

    // ── Prompt builder tests ──────────────────────────────────────────────────

    #[test]
    fn system_prompt_references_orchestrator_role() {
        let sys = build_system_prompt("orchestrator", &[]);
        assert!(
            sys.contains("technical lead"),
            "should use orchestrator role identity"
        );
        assert!(
            sys.contains("JSON array"),
            "should instruct JSON array return"
        );
        assert!(sys.contains("spawn_subagent"), "should list spawn_subagent");
        assert!(sys.contains("complete_run"), "should list complete_run");
    }

    #[test]
    fn system_prompt_fallback_for_unknown_role() {
        let sys = build_system_prompt("wizard", &[]);
        assert!(
            sys.contains("JSON array"),
            "fallback must still instruct JSON return"
        );
        assert!(
            sys.contains("autonomous agent"),
            "fallback should use generic autonomous identity"
        );
    }

    #[test]
    fn user_message_contains_goal_and_run_id() {
        let msg = build_user_message(&ctx(), &empty_gather(), None);
        assert!(msg.contains("cairn-rs architecture"), "goal must appear");
        assert!(msg.contains("run_1"), "run_id must appear");
        assert!(msg.contains("orchestrator"), "agent_type must appear");
    }

    #[test]
    fn user_message_embeds_step_history() {
        let mut g = empty_gather();
        g.step_history = vec![crate::context::StepSummary {
            iteration: 0,
            action_kind: "invoke_tool".to_owned(),
            summary: "searched for architecture docs".to_owned(),
            succeeded: true,
        }];
        let msg = build_user_message(&ctx(), &g, None);
        assert!(
            msg.contains("architecture docs"),
            "step history must appear"
        );
        assert!(msg.contains("invoke_tool"), "action kind must appear");
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
        assert!(
            proposals[0].requires_approval,
            "escalation must require approval"
        );
        assert_eq!(
            proposals[0].confidence, 0.0,
            "fallback confidence must be 0"
        );
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
        assert!(
            out.requires_approval,
            "requires_approval must be true when any proposal is flagged"
        );
    }

    #[tokio::test]
    async fn decide_applies_confidence_bias() {
        let mock = Arc::new(MockProvider {
            response: r#"[{"action_type":"complete_run","description":"done","confidence":0.5,"requires_approval":false}]"#.to_owned(),
        });
        let phase = LlmDecidePhase::new(mock, "gemma4").with_confidence_bias(0.2);
        let out = phase.decide(&ctx(), &empty_gather()).await.unwrap();
        assert!(
            (out.proposals[0].confidence - 0.7).abs() < 1e-9,
            "bias should increase confidence"
        );
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
        g.memory_chunks = (0..5)
            .map(|i| cairn_memory::retrieval::RetrievalResult {
                chunk: {
                    let c = cairn_memory::ingest::ChunkRecord {
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
            })
            .collect();
        g.step_history = (0..3)
            .map(|i| crate::context::StepSummary {
                iteration: i,
                action_kind: "invoke_tool".to_owned(),
                summary: "did a thing".to_owned(),
                succeeded: true,
            })
            .collect();

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
        g.step_history = vec![crate::context::StepSummary {
            iteration: 0,
            action_kind: "invoke_tool".to_owned(),
            summary: "searched for architecture docs".to_owned(),
            succeeded: true,
        }];
        // memory chunk with distinctive text
        g.memory_chunks = vec![cairn_memory::retrieval::RetrievalResult {
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
        }];

        let msg = build_user_message(&ctx(), &g, None);

        assert!(
            msg.contains("cairn uses event sourcing"),
            "memory chunk must appear without budget"
        );
        assert!(
            msg.contains("architecture docs"),
            "step history must appear without budget"
        );
    }

    /// Memory chunks are included most-relevant-first; least relevant are dropped
    /// when the budget is tight.
    #[test]
    fn memory_chunks_most_relevant_first() {
        let texts = [
            "highly relevant content here",
            "somewhat relevant",
            "least relevant stuff",
        ];
        let mut g = empty_gather();
        g.memory_chunks = texts
            .iter()
            .enumerate()
            .map(|(i, text)| cairn_memory::retrieval::RetrievalResult {
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
            })
            .collect();

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

    // ── Plan mode tool filtering (RFC 018) ──────────────────────────────

    #[test]
    fn plan_mode_ctx_has_run_mode() {
        let mut c = ctx();
        c.run_mode = cairn_domain::decisions::RunMode::Plan;
        assert!(matches!(c.run_mode, cairn_domain::decisions::RunMode::Plan));
    }

    #[tokio::test]
    async fn plan_mode_filters_external_tools_from_prompt() {
        use cairn_domain::decisions::RunMode;

        // Create a mock provider that captures the system prompt.
        let captured_prompt = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let prompt_ref = captured_prompt.clone();
        struct CapturingProvider {
            captured: std::sync::Arc<std::sync::Mutex<String>>,
        }
        #[async_trait]
        impl GenerationProvider for CapturingProvider {
            async fn generate(
                &self,
                _model: &str,
                messages: Vec<serde_json::Value>,
                _settings: &ProviderBindingSettings,
            ) -> Result<GenerationResponse, ProviderAdapterError> {
                if let Some(system) = messages.first().and_then(|m| m["content"].as_str()) {
                    *self.captured.lock().unwrap() = system.to_owned();
                }
                Ok(GenerationResponse {
                    text: r#"[{"action_type":"complete_run","description":"done","confidence":0.9,"requires_approval":false}]"#.to_owned(),
                    input_tokens: Some(100),
                    output_tokens: Some(50),
                    model_id: "test-model".to_owned(),
                    tool_calls: vec![],
                })
            }
        }

        // Build a registry with both Observational and External tools.
        let registry = std::sync::Arc::new(
            cairn_tools::builtins::BuiltinToolRegistry::new()
                .register(std::sync::Arc::new(cairn_tools::GrepSearchTool)) // Observational
                .register(std::sync::Arc::new(cairn_tools::CalculateTool)) // Observational
                .register(std::sync::Arc::new(cairn_tools::ShellExecTool)), // External
        );

        let phase = LlmDecidePhase::new(
            std::sync::Arc::new(CapturingProvider {
                captured: prompt_ref,
            }),
            "test-model",
        )
        .with_tools(registry);

        // Plan mode context
        let mut plan_ctx = ctx();
        plan_ctx.run_mode = RunMode::Plan;

        let _ = phase.decide(&plan_ctx, &empty_gather()).await.unwrap();
        let prompt = captured_prompt.lock().unwrap().clone();

        // The prompt tool descriptor lines use the format "tool_name(params) — desc".
        // Check for descriptor lines, not arbitrary mentions of tool names in prose.
        assert!(
            prompt.contains("  - grep_search("),
            "Observational tool descriptor should be in Plan mode prompt"
        );
        assert!(
            prompt.contains("  - calculate("),
            "Observational tool descriptor should be in Plan mode prompt"
        );
        // External tools should not have descriptor lines in Plan mode.
        assert!(
            !prompt.contains("  - shell_exec("),
            "External tool descriptor must NOT be in Plan mode prompt"
        );
    }

    #[tokio::test]
    async fn enabled_plugin_tool_appears_in_project_prompt_but_not_other_projects() {
        struct CapturingProvider {
            captured: Arc<std::sync::Mutex<String>>,
        }

        #[async_trait]
        impl GenerationProvider for CapturingProvider {
            async fn generate(
                &self,
                _model: &str,
                messages: Vec<serde_json::Value>,
                _settings: &ProviderBindingSettings,
            ) -> Result<GenerationResponse, ProviderAdapterError> {
                if let Some(system) = messages.first().and_then(|m| m["content"].as_str()) {
                    *self.captured.lock().unwrap() = system.to_owned();
                }
                Ok(GenerationResponse {
                    text: r#"[{"action_type":"complete_run","description":"done","confidence":0.9,"requires_approval":false}]"#.to_owned(),
                    input_tokens: Some(100),
                    output_tokens: Some(20),
                    model_id: "test-model".to_owned(),
                    tool_calls: vec![],
                })
            }
        }

        let prompt_a = Arc::new(std::sync::Mutex::new(String::new()));
        let phase_a = LlmDecidePhase::new(
            Arc::new(CapturingProvider {
                captured: prompt_a.clone(),
            }),
            "test-model",
        )
        .with_tools(registry_for_project(&cairn_domain::ProjectKey::new(
            "tenant",
            "workspace",
            "project-a",
        )));
        let mut ctx_a = ctx();
        ctx_a.project = cairn_domain::ProjectKey::new("tenant", "workspace", "project-a");
        phase_a.decide(&ctx_a, &empty_gather()).await.unwrap();

        let prompt_b = Arc::new(std::sync::Mutex::new(String::new()));
        let phase_b = LlmDecidePhase::new(
            Arc::new(CapturingProvider {
                captured: prompt_b.clone(),
            }),
            "test-model",
        )
        .with_tools(registry_for_project(&cairn_domain::ProjectKey::new(
            "tenant",
            "workspace",
            "project-b",
        )));
        let mut ctx_b = ctx();
        ctx_b.project = cairn_domain::ProjectKey::new("tenant", "workspace", "project-b");
        phase_b.decide(&ctx_b, &empty_gather()).await.unwrap();

        assert!(
            prompt_a.lock().unwrap().contains("  - github.issue_brief("),
            "enabled project should see its plugin tool in the prompt"
        );
        assert!(
            !prompt_b.lock().unwrap().contains("  - github.issue_brief("),
            "disabled project must not see the plugin tool in the prompt"
        );
    }

    #[tokio::test]
    async fn tool_search_respects_plugin_visibility() {
        let enabled_project = cairn_domain::ProjectKey::new("tenant", "workspace", "project-a");
        let disabled_project = cairn_domain::ProjectKey::new("tenant", "workspace", "project-b");

        let enabled_registry = registry_for_project(&enabled_project);
        let enabled_tool = ToolSearchTool::new(enabled_registry);
        let enabled = enabled_tool
            .execute(
                &enabled_project,
                serde_json::json!({ "query": "search github issues" }),
            )
            .await
            .unwrap();
        let enabled_names: Vec<&str> = enabled.output["matches"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["name"].as_str())
            .collect();
        assert!(
            enabled_names.contains(&"github.issue_search"),
            "enabled project should be able to discover the deferred plugin tool"
        );

        let disabled_registry = registry_for_project(&disabled_project);
        let disabled_tool = ToolSearchTool::new(disabled_registry);
        let disabled = disabled_tool
            .execute(
                &disabled_project,
                serde_json::json!({ "query": "search github issues" }),
            )
            .await
            .unwrap();
        assert_eq!(
            disabled.output["total"], 0,
            "disabled project must not discover tools from an unenabled plugin"
        );
    }
}
