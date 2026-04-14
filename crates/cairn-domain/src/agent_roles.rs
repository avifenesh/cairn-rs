//! Agent role domain types (GAP-011).
//!
//! An `AgentRole` is a named, reusable capability profile that configures
//! how a run behaves: which tools it may invoke, how much context it receives,
//! and which system prompt shapes its persona.
//!
//! Mirrors `cairn/internal/agenttype` (Go).

use serde::{Deserialize, Serialize};

/// Capability tier that determines default resource limits and routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentRoleTier {
    /// Standard worker role — default context, standard tool set.
    #[default]
    Standard,
    /// Research role — extended context for multi-document retrieval.
    Research,
    /// Orchestrator role — maximum context, all tools, spawns sub-agents.
    Orchestrator,
}

/// A named capability profile attached to a run.
///
/// Roles are immutable once registered. To change a role, register a new
/// version with the same `role_id` — the registry last-write wins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRole {
    /// Stable lowercase identifier (e.g. `"orchestrator"`, `"researcher"`).
    pub role_id: String,
    /// Human-readable label.
    pub display_name: String,
    /// Optional system-prompt fragment injected at run start.
    pub system_prompt: Option<String>,
    /// Allowed tool IDs. Empty means all tools in the run's permission set.
    pub allowed_tools: Vec<String>,
    /// Hard context-window cap in tokens. `None` means use the model default.
    pub max_context_tokens: Option<u32>,
    pub tier: AgentRoleTier,
}

impl AgentRole {
    pub fn new(
        role_id: impl Into<String>,
        display_name: impl Into<String>,
        tier: AgentRoleTier,
    ) -> Self {
        Self {
            role_id: role_id.into(),
            display_name: display_name.into(),
            system_prompt: None,
            allowed_tools: Vec::new(),
            max_context_tokens: None,
            tier,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_tools(mut self, tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.allowed_tools = tools.into_iter().map(|t| t.into()).collect();
        self
    }

    pub fn with_max_context_tokens(mut self, tokens: u32) -> Self {
        self.max_context_tokens = Some(tokens);
        self
    }
}

/// Built-in default roles shipped with cairn-rs.
///
/// These are registered at startup by `AgentRoleRegistry::with_defaults()`.
pub fn default_roles() -> Vec<AgentRole> {
    vec![
        AgentRole::new("orchestrator", "Orchestrator", AgentRoleTier::Orchestrator)
            .with_system_prompt(
                "You are a senior technical lead coordinating a team of agents. \
                 Break complex goals into focused sub-tasks, assign them to the right \
                 agent roles (researcher, executor, reviewer), and synthesise their \
                 results into a coherent outcome. Monitor progress, resolve conflicts \
                 between sub-agent outputs, and escalate to the operator when blocked. \
                 You have access to all tools and can spawn sub-agents.",
            )
            .with_max_context_tokens(200_000),
        AgentRole::new("researcher", "Researcher", AgentRoleTier::Research)
            .with_system_prompt(
                "You are a thorough technical analyst. Your job is to gather, verify, \
                 and synthesise information from multiple sources — memory, files, search \
                 results, and web pages. Explore before concluding: read at least 3 \
                 relevant sources before forming an answer. Cite specific files and \
                 evidence for every finding. Store key discoveries in memory for future \
                 reference. If information is conflicting, present all sides clearly.",
            )
            .with_tools([
                "cairn.search",
                "cairn.retrieve",
                "cairn.readFile",
                "cairn.listFiles",
                "cairn.webSearch",
                "cairn.fetchUrl",
            ])
            .with_max_context_tokens(128_000),
        AgentRole::new("executor", "Executor", AgentRoleTier::Standard)
            .with_system_prompt(
                "You are an autonomous engineer executing a well-defined task. \
                 Read the goal, understand what needs to change, make the changes \
                 using the available tools, and verify the result. Write real code — \
                 not descriptions or pseudocode. If a command fails, analyse the error \
                 and try a different approach. Report progress clearly and surface \
                 blockers early via escalation rather than guessing.",
            )
            .with_tools([
                "cairn.runCommand",
                "cairn.readFile",
                "cairn.writeFile",
                "cairn.listFiles",
                "cairn.search",
            ]),
        AgentRole::new("reviewer", "Reviewer", AgentRoleTier::Standard)
            .with_system_prompt(
                "You are a meticulous code reviewer. Read all relevant files, search \
                 for patterns and anti-patterns, inspect recent changes, and produce \
                 a structured review. Rate findings by severity (critical, warning, \
                 suggestion). Be constructive — explain why something is a problem \
                 and suggest a concrete fix. You use read-only tools; you do not \
                 modify code or state.",
            )
            .with_tools([
                "cairn.readFile",
                "cairn.listFiles",
                "cairn.search",
                "cairn.retrieve",
            ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_role_builder() {
        let role = AgentRole::new("custom", "Custom Role", AgentRoleTier::Standard)
            .with_system_prompt("Be helpful.")
            .with_tools(["tool_a", "tool_b"])
            .with_max_context_tokens(32_000);

        assert_eq!(role.role_id, "custom");
        assert_eq!(role.tier, AgentRoleTier::Standard);
        assert_eq!(role.allowed_tools.len(), 2);
        assert_eq!(role.max_context_tokens, Some(32_000));
    }

    #[test]
    fn default_roles_non_empty() {
        let roles = default_roles();
        assert_eq!(roles.len(), 4);
        let ids: Vec<_> = roles.iter().map(|r| r.role_id.as_str()).collect();
        assert!(ids.contains(&"orchestrator"));
        assert!(ids.contains(&"researcher"));
        assert!(ids.contains(&"executor"));
        assert!(ids.contains(&"reviewer"));
    }

    #[test]
    fn orchestrator_tier_is_orchestrator() {
        let roles = default_roles();
        let orch = roles.iter().find(|r| r.role_id == "orchestrator").unwrap();
        assert_eq!(orch.tier, AgentRoleTier::Orchestrator);
        assert!(orch.max_context_tokens.unwrap() >= 100_000);
    }

    #[test]
    fn reviewer_is_read_only_tools() {
        let roles = default_roles();
        let rev = roles.iter().find(|r| r.role_id == "reviewer").unwrap();
        // Reviewer must NOT include write tools.
        assert!(!rev
            .allowed_tools
            .iter()
            .any(|t| t.contains("write") || t.contains("Write")));
    }
}
