//! Skill catalog service boundary — GAP-012.

use cairn_domain::skills::{Skill, SkillInvocation};

use crate::error::RuntimeError;

/// Service trait for managing and invoking skills.
pub trait SkillCatalogService: Send + Sync {
    /// Register a skill.
    fn register(&mut self, skill: Skill);

    /// Get a skill by ID.
    fn get(&self, skill_id: &str) -> Option<&Skill>;

    /// List skills, optionally filtered by tags.
    fn list(&self, tags_filter: &[&str]) -> Vec<&Skill>;

    /// Enable a skill (Active + enabled=true).
    fn enable(&mut self, skill_id: &str) -> Result<(), RuntimeError>;

    /// Disable a skill.
    fn disable(&mut self, skill_id: &str) -> Result<(), RuntimeError>;

    /// Invoke a skill, returning a `SkillInvocation` with the generated invocation_id.
    ///
    /// In v1, invocation is synchronous and the skill is immediately marked
    /// Completed (the actual execution is caller-supplied via `executor`).
    fn invoke(
        &self,
        skill_id: &str,
        args: serde_json::Value,
    ) -> Result<SkillInvocation, RuntimeError>;
}
