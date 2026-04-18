//! Skill catalog domain types.
//!
//! A skill is a composable capability bundle that teaches the agent how to
//! accomplish a category of tasks. Skills are registered in the `SkillCatalog`
//! and can be invoked by the runtime.

use serde::{Deserialize, Serialize};

/// Status of a skill in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    /// Available to agents — shows in catalog and can be invoked.
    Active,
    /// Awaiting operator approval before becoming Active.
    Proposed,
    /// Rejected by operator; not available.
    Rejected,
}

/// Invocation status for a `SkillInvocation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillInvocationStatus {
    Running,
    Completed,
    Failed,
}

/// A registered capability bundle.
///
/// Skills are the unit of composable behavior in the skill marketplace.
/// Each skill has a unique `skill_id`, a human-readable `name`, and metadata
/// controlling when and how it can be used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// Unique slug identifier (e.g. `content-pipeline`, `decision-support`).
    pub skill_id: String,
    /// Human-readable name.
    pub name: String,
    /// Trigger description — used to decide when to invoke.
    pub description: String,
    /// Semantic version (e.g. `1.0.0`).
    pub version: String,
    /// Entry point: path or command used to load/invoke the skill.
    pub entry_point: String,
    /// Permissions the skill requires (e.g. `["file:read", "shell:exec"]`).
    pub required_permissions: Vec<String>,
    /// Searchable tags (e.g. `["coding", "research"]`).
    pub tags: Vec<String>,
    /// Whether the skill is active and available for invocation.
    pub enabled: bool,
    /// Lifecycle status.
    pub status: SkillStatus,
}

impl Skill {
    pub fn new(
        skill_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        entry_point: impl Into<String>,
    ) -> Self {
        Self {
            skill_id: skill_id.into(),
            name: name.into(),
            description: description.into(),
            version: version.into(),
            entry_point: entry_point.into(),
            required_permissions: vec![],
            tags: vec![],
            enabled: false,
            status: SkillStatus::Proposed,
        }
    }
}

/// Durable record of one skill invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillInvocation {
    /// Unique ID for this invocation.
    pub invocation_id: String,
    /// Which skill was invoked.
    pub skill_id: String,
    /// Arguments passed to the skill (free-form JSON).
    pub args: serde_json::Value,
    /// Result from the skill, set when status is Completed/Failed.
    pub result: Option<serde_json::Value>,
    /// Current execution status.
    pub status: SkillInvocationStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// In-process skill catalog — register, look up, filter, and enable/disable skills.
///
/// Thread-safety: wrap in `Arc<RwLock<SkillCatalog>>` for shared use.
#[derive(Debug, Default)]
pub struct SkillCatalog {
    skills: std::collections::HashMap<String, Skill>,
}

impl SkillCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a skill, replacing any existing entry with the same `skill_id`.
    pub fn register(&mut self, skill: Skill) {
        self.skills.insert(skill.skill_id.clone(), skill);
    }

    /// Look up a skill by ID.
    pub fn get(&self, skill_id: &str) -> Option<&Skill> {
        self.skills.get(skill_id)
    }

    /// List skills, optionally filtered by tags.
    ///
    /// When `tags_filter` is non-empty, only skills that have **all** the
    /// requested tags are returned.
    pub fn list(&self, tags_filter: &[&str]) -> Vec<&Skill> {
        let mut results: Vec<&Skill> = self
            .skills
            .values()
            .filter(|s| {
                tags_filter.is_empty()
                    || tags_filter
                        .iter()
                        .all(|req| s.tags.iter().any(|t| t == req))
            })
            .collect();
        results.sort_by_key(|r| r.skill_id.clone());
        results
    }

    /// Enable a skill (set `enabled = true` and status to `Active`).
    ///
    /// Returns `false` if the skill is not registered.
    pub fn enable(&mut self, skill_id: &str) -> bool {
        if let Some(skill) = self.skills.get_mut(skill_id) {
            skill.enabled = true;
            skill.status = SkillStatus::Active;
            true
        } else {
            false
        }
    }

    /// Disable a skill (set `enabled = false`).
    ///
    /// Returns `false` if the skill is not registered.
    pub fn disable(&mut self, skill_id: &str) -> bool {
        if let Some(skill) = self.skills.get_mut(skill_id) {
            skill.enabled = false;
            true
        } else {
            false
        }
    }

    /// All enabled skills sorted by ID.
    pub fn enabled(&self) -> Vec<&Skill> {
        let mut v: Vec<_> = self.skills.values().filter(|s| s.enabled).collect();
        v.sort_by_key(|r| r.skill_id.clone());
        v
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_skill(id: &str) -> Skill {
        Skill {
            skill_id: id.to_owned(),
            name: format!("{id} skill"),
            description: format!("Use when user needs {id}"),
            version: "1.0.0".to_owned(),
            entry_point: format!("skills/{id}/main.md"),
            required_permissions: vec![],
            tags: vec!["general".to_owned()],
            enabled: false,
            status: SkillStatus::Proposed,
        }
    }

    #[test]
    fn register_and_get() {
        let mut catalog = SkillCatalog::new();
        catalog.register(sample_skill("code-review"));
        assert!(catalog.get("code-review").is_some());
        assert!(catalog.get("unknown").is_none());
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn register_replaces_existing() {
        let mut catalog = SkillCatalog::new();
        catalog.register(sample_skill("pr-writer"));
        let mut updated = sample_skill("pr-writer");
        updated.version = "2.0.0".to_owned();
        catalog.register(updated);
        assert_eq!(catalog.get("pr-writer").unwrap().version, "2.0.0");
        assert_eq!(catalog.len(), 1, "no duplicate");
    }

    #[test]
    fn enable_sets_active_status() {
        let mut catalog = SkillCatalog::new();
        catalog.register(sample_skill("digest"));

        assert!(!catalog.get("digest").unwrap().enabled);
        assert!(catalog.enable("digest"));
        let skill = catalog.get("digest").unwrap();
        assert!(skill.enabled);
        assert_eq!(skill.status, SkillStatus::Active);
    }

    #[test]
    fn disable_clears_enabled() {
        let mut catalog = SkillCatalog::new();
        catalog.register(sample_skill("s1"));
        catalog.enable("s1");
        catalog.disable("s1");
        assert!(!catalog.get("s1").unwrap().enabled);
    }

    #[test]
    fn enable_returns_false_for_unknown() {
        let mut catalog = SkillCatalog::new();
        assert!(!catalog.enable("nonexistent"));
    }

    #[test]
    fn list_no_filter_returns_all() {
        let mut catalog = SkillCatalog::new();
        catalog.register(sample_skill("a"));
        catalog.register(sample_skill("b"));
        assert_eq!(catalog.list(&[]).len(), 2);
    }

    #[test]
    fn list_with_tag_filter() {
        let mut catalog = SkillCatalog::new();
        let mut s1 = sample_skill("s1");
        s1.tags = vec!["coding".to_owned(), "review".to_owned()];
        let mut s2 = sample_skill("s2");
        s2.tags = vec!["research".to_owned()];
        catalog.register(s1);
        catalog.register(s2);

        let coding = catalog.list(&["coding"]);
        assert_eq!(coding.len(), 1);
        assert_eq!(coding[0].skill_id, "s1");

        let none = catalog.list(&["coding", "research"]); // must have BOTH
        assert_eq!(none.len(), 0);
    }

    #[test]
    fn enabled_skills_only() {
        let mut catalog = SkillCatalog::new();
        catalog.register(sample_skill("a"));
        catalog.register(sample_skill("b"));
        catalog.enable("a");
        let enabled = catalog.enabled();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].skill_id, "a");
    }

    #[test]
    fn skill_invocation_has_required_fields() {
        let inv = SkillInvocation {
            invocation_id: "inv_1".to_owned(),
            skill_id: "code-review".to_owned(),
            args: serde_json::json!({"pr_url": "https://github.com/org/repo/pull/1"}),
            result: None,
            status: SkillInvocationStatus::Running,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };
        assert_eq!(inv.status, SkillInvocationStatus::Running);
        assert!(inv.result.is_none());
        assert!(!inv.invocation_id.is_empty());
    }

    #[test]
    fn skill_invocation_status_variants_are_distinct() {
        assert_ne!(
            SkillInvocationStatus::Running,
            SkillInvocationStatus::Completed
        );
        assert_ne!(
            SkillInvocationStatus::Completed,
            SkillInvocationStatus::Failed
        );
        assert_ne!(
            SkillInvocationStatus::Running,
            SkillInvocationStatus::Failed
        );
    }
}
