//! Agent roles integration tests (GAP-011).
//!
//! Validates the `AgentRoleRegistry` + `default_roles()` pipeline end-to-end.
//! Agent roles are capability profiles attached to runs at creation time,
//! controlling context window, allowed tools, and system prompt.
//!
//! Note: AgentRoleRegistry lives in cairn-runtime (not cairn-domain) because
//! it requires thread-safe shared state (Arc<RwLock<...>>). The domain types
//! (AgentRole, AgentRoleTier, default_roles) are in cairn-domain.
//!
//! Default roles (4 total):
//!   orchestrator  AgentRoleTier::Orchestrator  max_ctx=200k  all tools
//!   researcher    AgentRoleTier::Research       max_ctx=128k  read tools
//!   executor      AgentRoleTier::Standard       max_ctx=None  write tools
//!   reviewer      AgentRoleTier::Standard       max_ctx=None  read-only tools

use cairn_domain::agent_roles::{AgentRole, AgentRoleTier};
use cairn_runtime::agent_roles::AgentRoleRegistry;

// ── 1. Create registry with 4 default roles ───────────────────────────────────

#[test]
fn with_defaults_has_exactly_four_roles() {
    let reg = AgentRoleRegistry::with_defaults();
    assert_eq!(reg.len(), 4, "must have exactly 4 default roles");
    assert!(!reg.is_empty());
}

#[test]
fn empty_registry_has_no_roles() {
    let reg = AgentRoleRegistry::empty();
    assert_eq!(reg.len(), 0);
    assert!(reg.is_empty());
    assert!(
        reg.default_role().is_none(),
        "empty registry has no default role"
    );
}

// ── 2. list_all returns all 4, sorted by role_id ─────────────────────────────

#[test]
fn list_all_returns_four_sorted_by_role_id() {
    let reg = AgentRoleRegistry::with_defaults();
    let all = reg.list_all();

    assert_eq!(all.len(), 4);

    // Verify sorted order (executor < orchestrator < researcher < reviewer).
    for window in all.windows(2) {
        assert!(
            window[0].role_id <= window[1].role_id,
            "list_all must be sorted"
        );
    }

    let ids: Vec<_> = all.iter().map(|r| r.role_id.as_str()).collect();
    assert!(ids.contains(&"orchestrator"));
    assert!(ids.contains(&"researcher"));
    assert!(ids.contains(&"executor"));
    assert!(ids.contains(&"reviewer"));
}

// ── 3. get_by_name for each of the 4 default roles ───────────────────────────

#[test]
fn get_orchestrator_returns_correct_role() {
    let reg = AgentRoleRegistry::with_defaults();
    let role = reg.get("orchestrator").expect("orchestrator must exist");

    assert_eq!(role.role_id, "orchestrator");
    assert_eq!(role.display_name, "Orchestrator");
    assert_eq!(role.tier, AgentRoleTier::Orchestrator);
    assert!(
        role.system_prompt.is_some(),
        "orchestrator has a system prompt"
    );
    assert!(
        role.max_context_tokens.unwrap_or(0) >= 100_000,
        "orchestrator gets extended context (>= 100k tokens)"
    );
}

#[test]
fn get_researcher_returns_correct_role() {
    let reg = AgentRoleRegistry::with_defaults();
    let role = reg.get("researcher").expect("researcher must exist");

    assert_eq!(role.role_id, "researcher");
    assert_eq!(role.tier, AgentRoleTier::Research);
    assert!(role.system_prompt.is_some());
    assert!(
        role.max_context_tokens.unwrap_or(0) >= 64_000,
        "researcher gets extended context for retrieval"
    );
    // Researcher must have retrieval tools.
    assert!(
        role.allowed_tools
            .iter()
            .any(|t| t.contains("retrieve") || t.contains("search")),
        "researcher must include retrieval/search tools"
    );
}

#[test]
fn get_executor_returns_correct_role() {
    let reg = AgentRoleRegistry::with_defaults();
    let role = reg.get("executor").expect("executor must exist");

    assert_eq!(role.role_id, "executor");
    assert_eq!(role.tier, AgentRoleTier::Standard);
    assert!(role.system_prompt.is_some());
    // Executor can write.
    assert!(
        role.allowed_tools.iter().any(|t| t.contains("write")
            || t.contains("Write")
            || t.contains("run")
            || t.contains("Run")),
        "executor must include write/run tools"
    );
}

#[test]
fn get_reviewer_returns_correct_role() {
    let reg = AgentRoleRegistry::with_defaults();
    let role = reg.get("reviewer").expect("reviewer must exist");

    assert_eq!(role.role_id, "reviewer");
    assert_eq!(role.tier, AgentRoleTier::Standard);
    assert!(role.system_prompt.is_some());
    // Reviewer is read-only — must NOT have write tools.
    assert!(
        !role
            .allowed_tools
            .iter()
            .any(|t| t.to_lowercase().contains("write")),
        "reviewer must not include write tools: {:?}",
        role.allowed_tools
    );
}

#[test]
fn get_unknown_role_returns_none() {
    let reg = AgentRoleRegistry::with_defaults();
    assert!(reg.get("nonexistent").is_none());
    assert!(reg.get("").is_none());
}

// ── 4. Register custom role → list_all returns 5 ─────────────────────────────

#[test]
fn register_custom_role_grows_registry_to_five() {
    let reg = AgentRoleRegistry::with_defaults();
    assert_eq!(reg.len(), 4);

    let custom = AgentRole::new("analyst", "Data Analyst", AgentRoleTier::Research)
        .with_system_prompt("You are a data analyst. Interpret metrics and surface insights.")
        .with_tools(["cairn.retrieve", "cairn.search", "cairn.readFile"])
        .with_max_context_tokens(64_000);

    reg.register(custom);

    assert_eq!(reg.len(), 5, "custom role added to registry");
    let all = reg.list_all();
    assert_eq!(all.len(), 5);

    let ids: Vec<_> = all.iter().map(|r| r.role_id.as_str()).collect();
    assert!(ids.contains(&"analyst"), "analyst must be in list_all");
}

#[test]
fn registered_custom_role_is_retrievable() {
    let reg = AgentRoleRegistry::with_defaults();

    let custom = AgentRole::new(
        "decision-maker",
        "Decision Maker",
        AgentRoleTier::Orchestrator,
    )
    .with_system_prompt("Evaluate options and decide.")
    .with_max_context_tokens(50_000);

    reg.register(custom);

    let found = reg
        .get("decision-maker")
        .expect("custom role must be retrievable");
    assert_eq!(found.display_name, "Decision Maker");
    assert_eq!(found.tier, AgentRoleTier::Orchestrator);
    assert_eq!(found.max_context_tokens, Some(50_000));
    assert!(found.system_prompt.as_deref().unwrap().contains("Evaluate"));
}

// ── 5. Role tier assignments ──────────────────────────────────────────────────

#[test]
fn tier_orchestrator_has_exactly_one_default_role() {
    let reg = AgentRoleRegistry::with_defaults();
    let orch_tier = reg.list_by_tier(AgentRoleTier::Orchestrator);

    assert_eq!(
        orch_tier.len(),
        1,
        "exactly one Orchestrator-tier role by default"
    );
    assert_eq!(orch_tier[0].role_id, "orchestrator");
    assert!(orch_tier
        .iter()
        .all(|r| r.tier == AgentRoleTier::Orchestrator));
}

#[test]
fn tier_research_has_exactly_one_default_role() {
    let reg = AgentRoleRegistry::with_defaults();
    let research = reg.list_by_tier(AgentRoleTier::Research);

    assert_eq!(
        research.len(),
        1,
        "exactly one Research-tier role by default"
    );
    assert_eq!(research[0].role_id, "researcher");
    assert!(research.iter().all(|r| r.tier == AgentRoleTier::Research));
}

#[test]
fn tier_standard_has_two_default_roles() {
    let reg = AgentRoleRegistry::with_defaults();
    let standard = reg.list_by_tier(AgentRoleTier::Standard);

    assert_eq!(
        standard.len(),
        2,
        "executor and reviewer are both Standard tier"
    );
    let ids: Vec<_> = standard.iter().map(|r| r.role_id.as_str()).collect();
    assert!(ids.contains(&"executor"));
    assert!(ids.contains(&"reviewer"));
    assert!(standard.iter().all(|r| r.tier == AgentRoleTier::Standard));
}

#[test]
fn tier_list_is_sorted_by_role_id() {
    let reg = AgentRoleRegistry::with_defaults();
    let standard = reg.list_by_tier(AgentRoleTier::Standard);
    for window in standard.windows(2) {
        assert!(
            window[0].role_id <= window[1].role_id,
            "list_by_tier must be sorted"
        );
    }
}

#[test]
fn tier_unknown_returns_empty() {
    let reg = AgentRoleRegistry::empty();
    assert!(reg.list_by_tier(AgentRoleTier::Orchestrator).is_empty());
    assert!(reg.list_by_tier(AgentRoleTier::Research).is_empty());
    assert!(reg.list_by_tier(AgentRoleTier::Standard).is_empty());
}

// ── 6. AgentRoleTier variants are distinct ────────────────────────────────────

#[test]
fn role_tier_variants_are_distinct() {
    assert_ne!(AgentRoleTier::Orchestrator, AgentRoleTier::Research);
    assert_ne!(AgentRoleTier::Orchestrator, AgentRoleTier::Standard);
    assert_ne!(AgentRoleTier::Research, AgentRoleTier::Standard);
}

// ── 7. default_role() returns the orchestrator ────────────────────────────────

#[test]
fn default_role_returns_orchestrator() {
    let reg = AgentRoleRegistry::with_defaults();
    let default = reg
        .default_role()
        .expect("default_role must return Some for a populated registry");

    assert_eq!(
        default.role_id, "orchestrator",
        "orchestrator is the canonical default role"
    );
    assert_eq!(default.tier, AgentRoleTier::Orchestrator);
}

#[test]
fn default_role_none_on_empty_registry() {
    let reg = AgentRoleRegistry::empty();
    assert!(
        reg.default_role().is_none(),
        "empty registry has no default role"
    );
}

#[test]
fn default_role_available_after_custom_registration() {
    // Registering other roles must not displace the default.
    let reg = AgentRoleRegistry::with_defaults();
    reg.register(AgentRole::new(
        "extra",
        "Extra Role",
        AgentRoleTier::Standard,
    ));

    let default = reg.default_role().unwrap();
    assert_eq!(
        default.role_id, "orchestrator",
        "default_role still returns orchestrator after additional registrations"
    );
}

// ── 8. Override: re-registering same role_id replaces the entry ───────────────

#[test]
fn register_same_id_replaces_without_duplicate() {
    let reg = AgentRoleRegistry::with_defaults();
    assert_eq!(reg.len(), 4);

    // Override orchestrator with a restricted version.
    let restricted_orch = AgentRole::new(
        "orchestrator",
        "Restricted Orchestrator",
        AgentRoleTier::Orchestrator,
    )
    .with_max_context_tokens(32_000);
    reg.register(restricted_orch);

    assert_eq!(reg.len(), 4, "override must not duplicate the entry");
    let orch = reg.get("orchestrator").unwrap();
    assert_eq!(orch.display_name, "Restricted Orchestrator");
    assert_eq!(orch.max_context_tokens, Some(32_000));
}

// ── 9. Clone: registry clone shares state ─────────────────────────────────────

#[test]
fn cloned_registry_shares_underlying_state() {
    let reg = AgentRoleRegistry::with_defaults();
    let clone = reg.clone();

    // Register via clone — must be visible in original.
    clone.register(AgentRole::new(
        "shared-role",
        "Shared",
        AgentRoleTier::Standard,
    ));

    assert_eq!(
        reg.len(),
        5,
        "clone shares Arc — registration is visible in original"
    );
    assert!(reg.get("shared-role").is_some());
}
