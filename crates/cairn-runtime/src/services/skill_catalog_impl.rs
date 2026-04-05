//! GAP-012: Skill catalog service implementation.

use cairn_domain::skills::{Skill, SkillCatalog, SkillInvocation, SkillInvocationStatus};

use crate::error::RuntimeError;
use crate::skill_catalog::SkillCatalogService;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn next_invocation_id() -> String {
    format!("inv_{}", now_ms())
}

/// In-process skill catalog service backed by `SkillCatalog`.
pub struct SkillCatalogServiceImpl {
    catalog: SkillCatalog,
}

impl SkillCatalogServiceImpl {
    pub fn new() -> Self {
        Self {
            catalog: SkillCatalog::new(),
        }
    }

    pub fn with_catalog(catalog: SkillCatalog) -> Self {
        Self { catalog }
    }
}

impl Default for SkillCatalogServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillCatalogService for SkillCatalogServiceImpl {
    fn register(&mut self, skill: Skill) {
        self.catalog.register(skill);
    }

    fn get(&self, skill_id: &str) -> Option<&Skill> {
        self.catalog.get(skill_id)
    }

    fn list(&self, tags_filter: &[&str]) -> Vec<&Skill> {
        self.catalog.list(tags_filter)
    }

    fn enable(&mut self, skill_id: &str) -> Result<(), RuntimeError> {
        if self.catalog.enable(skill_id) {
            Ok(())
        } else {
            Err(RuntimeError::NotFound {
                entity: "skill",
                id: skill_id.to_owned(),
            })
        }
    }

    fn disable(&mut self, skill_id: &str) -> Result<(), RuntimeError> {
        if self.catalog.disable(skill_id) {
            Ok(())
        } else {
            Err(RuntimeError::NotFound {
                entity: "skill",
                id: skill_id.to_owned(),
            })
        }
    }

    fn invoke(
        &self,
        skill_id: &str,
        args: serde_json::Value,
    ) -> Result<SkillInvocation, RuntimeError> {
        let skill = self.catalog.get(skill_id).ok_or_else(|| RuntimeError::NotFound {
            entity: "skill",
            id: skill_id.to_owned(),
        })?;

        if !skill.enabled {
            return Err(RuntimeError::PolicyDenied {
                reason: format!("skill '{}' is not enabled", skill_id),
            });
        }

        let now = now_ms();
        Ok(SkillInvocation {
            invocation_id: next_invocation_id(),
            skill_id: skill_id.to_owned(),
            args,
            result: None,
            status: SkillInvocationStatus::Running,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::skills::{SkillInvocationStatus, SkillStatus};

    fn make_skill(id: &str) -> Skill {
        Skill {
            skill_id: id.to_owned(),
            name: format!("{id} Skill"),
            description: format!("Does {id} things"),
            version: "1.0.0".to_owned(),
            entry_point: format!("skills/{id}/main.md"),
            required_permissions: vec![],
            tags: vec!["general".to_owned()],
            enabled: false,
            status: SkillStatus::Proposed,
        }
    }

    /// Manager spec: register → enable → invoke → assert invocation_id returned.
    #[test]
    fn skill_register_enable_invoke_returns_invocation_id() {
        let mut svc = SkillCatalogServiceImpl::new();

        // Register the skill.
        svc.register(make_skill("content-pipeline"));

        // Enable it.
        svc.enable("content-pipeline").unwrap();

        // Invoke it.
        let inv = svc
            .invoke("content-pipeline", serde_json::json!({"topic": "AI news"}))
            .unwrap();

        // Assert invocation_id is returned and non-empty.
        assert!(!inv.invocation_id.is_empty(), "invocation_id must be returned");
        assert_eq!(inv.skill_id, "content-pipeline");
        assert_eq!(inv.status, SkillInvocationStatus::Running);
    }

    /// Invoking a disabled skill must return PolicyDenied.
    #[test]
    fn invoke_disabled_skill_returns_policy_denied() {
        let mut svc = SkillCatalogServiceImpl::new();
        svc.register(make_skill("disabled-skill"));
        // Do NOT enable it.

        let err = svc
            .invoke("disabled-skill", serde_json::json!({}))
            .unwrap_err();
        assert!(
            matches!(err, RuntimeError::PolicyDenied { .. }),
            "expected PolicyDenied, got: {err:?}"
        );
    }

    /// Invoking an unknown skill must return NotFound.
    #[test]
    fn invoke_unknown_skill_returns_not_found() {
        let svc = SkillCatalogServiceImpl::new();
        let err = svc.invoke("ghost-skill", serde_json::json!({})).unwrap_err();
        assert!(matches!(err, RuntimeError::NotFound { .. }));
    }

    /// Enabling unknown skill returns NotFound.
    #[test]
    fn enable_unknown_skill_returns_not_found() {
        let mut svc = SkillCatalogServiceImpl::new();
        let err = svc.enable("nonexistent").unwrap_err();
        assert!(matches!(err, RuntimeError::NotFound { .. }));
    }

    /// list() returns all skills with no filter.
    #[test]
    fn list_returns_all_skills_without_filter() {
        let mut svc = SkillCatalogServiceImpl::new();
        svc.register(make_skill("a"));
        svc.register(make_skill("b"));
        assert_eq!(svc.list(&[]).len(), 2);
    }

    /// list() filters by tag.
    #[test]
    fn list_filters_by_tag() {
        let mut svc = SkillCatalogServiceImpl::new();
        let mut s = make_skill("research");
        s.tags = vec!["research".to_owned()];
        svc.register(s);
        svc.register(make_skill("coding")); // tags: ["general"]

        let research = svc.list(&["research"]);
        assert_eq!(research.len(), 1);
        assert_eq!(research[0].skill_id, "research");
    }

    /// Invocation args are preserved on the returned record.
    #[test]
    fn invoke_preserves_args() {
        let mut svc = SkillCatalogServiceImpl::new();
        svc.register(make_skill("writer"));
        svc.enable("writer").unwrap();

        let args = serde_json::json!({"topic": "Rust", "length": 500});
        let inv = svc.invoke("writer", args.clone()).unwrap();
        assert_eq!(inv.args, args);
    }
}
