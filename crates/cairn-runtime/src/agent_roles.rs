//! Agent role registry (GAP-011).
//!
//! `AgentRoleRegistry` is an in-memory catalog of `AgentRole` entries.
//! Roles are registered at startup (see `AgentRoleRegistry::with_defaults()`)
//! and queried at run-creation time to attach a role profile to a run.
//!
//! Thread-safe via `Arc<RwLock<HashMap>>`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use cairn_domain::agent_roles::{default_roles, AgentRole, AgentRoleTier};

/// In-memory, thread-safe agent role catalog.
#[derive(Clone)]
pub struct AgentRoleRegistry {
    inner: Arc<RwLock<HashMap<String, AgentRole>>>,
}

impl AgentRoleRegistry {
    /// Empty registry (no roles registered).
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registry pre-populated with the four built-in default roles:
    /// `orchestrator`, `researcher`, `executor`, `reviewer`.
    pub fn with_defaults() -> Self {
        let reg = Self::empty();
        for role in default_roles() {
            reg.register(role);
        }
        reg
    }

    /// Add or replace a role. Last write wins on `role_id` collision.
    pub fn register(&self, role: AgentRole) {
        self.inner
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(role.role_id.clone(), role);
    }

    /// Look up a role by its stable ID. Returns `None` if not found.
    pub fn get(&self, role_id: &str) -> Option<AgentRole> {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(role_id)
            .cloned()
    }

    /// List all roles matching the given tier, sorted by `role_id`.
    pub fn list_by_tier(&self, tier: AgentRoleTier) -> Vec<AgentRole> {
        let mut roles: Vec<AgentRole> = self
            .inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|r| r.tier == tier)
            .cloned()
            .collect();
        roles.sort_by_key(|r| r.role_id.clone());
        roles
    }

    /// List all registered roles, sorted by `role_id`.
    pub fn list_all(&self) -> Vec<AgentRole> {
        let mut roles: Vec<AgentRole> = self
            .inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        roles.sort_by_key(|r| r.role_id.clone());
        roles
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The default role for new runs when no explicit role is specified.
    ///
    /// Returns the `orchestrator` role, which has maximum context and all tools.
    /// Returns `None` only if the registry was constructed empty and never had
    /// an orchestrator registered.
    pub fn default_role(&self) -> Option<AgentRole> {
        self.get("orchestrator")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use cairn_domain::agent_roles::{AgentRole, AgentRoleTier};
    use cairn_domain::*;
    use cairn_store::projections::RunReadModel;
    use cairn_store::{EventLog, InMemoryStore};

    // ── Registry unit tests ───────────────────────────────────────────────

    #[test]
    fn agent_roles_registry_empty() {
        let reg = AgentRoleRegistry::empty();
        assert!(reg.is_empty());
        assert!(reg.get("orchestrator").is_none());
    }

    #[test]
    fn agent_roles_registry_with_defaults_has_four_roles() {
        let reg = AgentRoleRegistry::with_defaults();
        assert_eq!(reg.len(), 4, "must have exactly 4 default roles");
    }

    #[test]
    fn agent_roles_list_all_returns_all() {
        let reg = AgentRoleRegistry::with_defaults();
        let all = reg.list_all();
        let ids: Vec<_> = all.iter().map(|r| r.role_id.as_str()).collect();
        assert!(ids.contains(&"orchestrator"));
        assert!(ids.contains(&"researcher"));
        assert!(ids.contains(&"executor"));
        assert!(ids.contains(&"reviewer"));
    }

    #[test]
    fn agent_roles_get_orchestrator() {
        let reg = AgentRoleRegistry::with_defaults();
        let orch = reg.get("orchestrator").expect("orchestrator must exist");
        assert_eq!(orch.tier, AgentRoleTier::Orchestrator);
        assert!(orch.max_context_tokens.is_some());
        assert!(orch.system_prompt.is_some());
    }

    #[test]
    fn agent_roles_list_by_tier_research() {
        let reg = AgentRoleRegistry::with_defaults();
        let research = reg.list_by_tier(AgentRoleTier::Research);
        assert!(
            !research.is_empty(),
            "must have at least one Research-tier role"
        );
        assert!(research.iter().all(|r| r.tier == AgentRoleTier::Research));
        assert!(research.iter().any(|r| r.role_id == "researcher"));
    }

    #[test]
    fn agent_roles_list_by_tier_orchestrator() {
        let reg = AgentRoleRegistry::with_defaults();
        let orch_tier = reg.list_by_tier(AgentRoleTier::Orchestrator);
        assert_eq!(orch_tier.len(), 1);
        assert_eq!(orch_tier[0].role_id, "orchestrator");
    }

    #[test]
    fn agent_roles_list_by_tier_standard() {
        let reg = AgentRoleRegistry::with_defaults();
        let standard = reg.list_by_tier(AgentRoleTier::Standard);
        let ids: Vec<_> = standard.iter().map(|r| r.role_id.as_str()).collect();
        assert!(ids.contains(&"executor"), "executor must be Standard");
        assert!(ids.contains(&"reviewer"), "reviewer must be Standard");
    }

    #[test]
    fn agent_roles_register_override_wins() {
        let reg = AgentRoleRegistry::with_defaults();
        let custom = AgentRole::new(
            "orchestrator",
            "Custom Orchestrator",
            AgentRoleTier::Orchestrator,
        )
        .with_max_context_tokens(999_999);
        reg.register(custom);
        assert_eq!(reg.len(), 4, "override must not add a new entry");
        assert_eq!(
            reg.get("orchestrator").unwrap().max_context_tokens,
            Some(999_999)
        );
    }

    #[test]
    fn agent_roles_register_new_role() {
        let reg = AgentRoleRegistry::with_defaults();
        let custom = AgentRole::new("tester", "Test Agent", AgentRoleTier::Standard);
        reg.register(custom);
        assert_eq!(reg.len(), 5);
        assert!(reg.get("tester").is_some());
    }

    // ── RunRecord integration tests ───────────────────────────────────────

    /// Create a run with a role_id and verify the RunRecord carries it.
    #[tokio::test]
    async fn agent_roles_run_created_with_researcher_role() {
        let store = Arc::new(InMemoryStore::new());
        let project = ProjectKey::new("t1", "w1", "p1");
        let session_id = SessionId::new("sess-1");
        let run_id = RunId::new("run-1");

        // Register the default roles and validate researcher exists.
        let reg = AgentRoleRegistry::with_defaults();
        let role = reg.get("researcher").expect("researcher must exist");
        assert_eq!(role.tier, AgentRoleTier::Research);

        // Emit RunCreated with agent_role_id = "researcher".
        store
            .append(&[mk_run_created(
                project.clone(),
                session_id.clone(),
                run_id.clone(),
                Some("researcher".to_owned()),
            )])
            .await
            .unwrap();

        // Verify the projection carries the role.
        let run = RunReadModel::get(store.as_ref(), &run_id)
            .await
            .unwrap()
            .expect("run must exist");

        assert_eq!(
            run.agent_role_id.as_deref(),
            Some("researcher"),
            "RunRecord must carry the agent_role_id from RunCreated"
        );
        assert_eq!(run.run_id, run_id);
    }

    /// Run without a role_id has agent_role_id = None.
    #[tokio::test]
    async fn agent_roles_run_without_role_has_none() {
        let store = Arc::new(InMemoryStore::new());
        let project = ProjectKey::new("t1", "w1", "p1");
        let run_id = RunId::new("run-2");

        store
            .append(&[mk_run_created(
                project,
                SessionId::new("sess-2"),
                run_id.clone(),
                None,
            )])
            .await
            .unwrap();

        let run = RunReadModel::get(store.as_ref(), &run_id)
            .await
            .unwrap()
            .expect("run must exist");

        assert!(run.agent_role_id.is_none());
    }

    /// Multiple runs with different roles.
    #[tokio::test]
    async fn agent_roles_multiple_runs_different_roles() {
        let store = Arc::new(InMemoryStore::new());
        let project = ProjectKey::new("t1", "w1", "p1");

        let runs = [
            ("run-orch", Some("orchestrator")),
            ("run-exec", Some("executor")),
            ("run-none", None),
        ];

        for (rid, role) in &runs {
            store
                .append(&[mk_run_created(
                    project.clone(),
                    SessionId::new("sess-3"),
                    RunId::new(*rid),
                    role.map(|r| r.to_owned()),
                )])
                .await
                .unwrap();
        }

        for (rid, expected_role) in &runs {
            let run = RunReadModel::get(store.as_ref(), &RunId::new(*rid))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                run.agent_role_id.as_deref(),
                *expected_role,
                "run {rid} must have role {:?}",
                expected_role
            );
        }
    }

    // ── helpers ───────────────────────────────────────────────────────────

    fn mk_run_created(
        project: ProjectKey,
        session_id: SessionId,
        run_id: RunId,
        agent_role_id: Option<String>,
    ) -> EventEnvelope<RuntimeEvent> {
        use cairn_domain::{EventId, EventSource, OwnershipKey};
        EventEnvelope {
            event_id: EventId::new(format!("ev_{}", run_id.as_str())),
            source: EventSource::System,
            ownership: OwnershipKey::Project(project.clone()),
            causation_id: None,
            correlation_id: None,
            payload: RuntimeEvent::RunCreated(RunCreated {
                project,
                session_id,
                run_id,
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id,
            }),
        }
    }
}
