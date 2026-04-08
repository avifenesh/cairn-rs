//! Skill catalog HTTP API — GAP-012.
//!
//! Routes:
//! - `GET  /v1/skills/catalog`       — list all registered skills (optional `?tag=` filter)
//! - `POST /v1/skills/invoke/:id`    — invoke a skill by ID

use cairn_domain::skills::{Skill, SkillInvocation, SkillInvocationStatus};
use serde::{Deserialize, Serialize};

// ── Request / Response types ──────────────────────────────────────────────────

/// Response payload for `GET /v1/skills/catalog`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillCatalogResponse {
    pub skills: Vec<SkillSummary>,
    pub total: usize,
}

/// Summary of one skill for the catalog listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub enabled: bool,
}

impl From<&Skill> for SkillSummary {
    fn from(s: &Skill) -> Self {
        Self {
            skill_id: s.skill_id.clone(),
            name: s.name.clone(),
            description: s.description.clone(),
            version: s.version.clone(),
            tags: s.tags.clone(),
            enabled: s.enabled,
        }
    }
}

/// Request body for `POST /v1/skills/invoke/:id`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvokeSkillRequest {
    /// Arguments to pass to the skill (free-form JSON).
    #[serde(default)]
    pub args: serde_json::Value,
}

/// Response payload for `POST /v1/skills/invoke/:id`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvokeSkillResponse {
    pub invocation_id: String,
    pub skill_id: String,
    pub status: String,
}

impl From<&SkillInvocation> for InvokeSkillResponse {
    fn from(inv: &SkillInvocation) -> Self {
        let status = match inv.status {
            SkillInvocationStatus::Running => "running",
            SkillInvocationStatus::Completed => "completed",
            SkillInvocationStatus::Failed => "failed",
        };
        Self {
            invocation_id: inv.invocation_id.clone(),
            skill_id: inv.skill_id.clone(),
            status: status.to_owned(),
        }
    }
}

/// Build a `SkillCatalogResponse` from a slice of skills.
pub fn build_catalog_response(skills: &[&Skill]) -> SkillCatalogResponse {
    let summaries: Vec<SkillSummary> = skills.iter().map(|s| SkillSummary::from(*s)).collect();
    let total = summaries.len();
    SkillCatalogResponse {
        skills: summaries,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::skills::{Skill, SkillInvocationStatus, SkillStatus};

    fn sample_skill(id: &str, enabled: bool) -> Skill {
        Skill {
            skill_id: id.to_owned(),
            name: format!("{id} Skill"),
            description: "Does something useful".to_owned(),
            version: "1.2.0".to_owned(),
            entry_point: format!("skills/{id}/main.md"),
            required_permissions: vec![],
            tags: vec!["general".to_owned()],
            enabled,
            status: if enabled {
                SkillStatus::Active
            } else {
                SkillStatus::Proposed
            },
        }
    }

    #[test]
    fn skill_summary_from_skill() {
        let skill = sample_skill("digest", true);
        let summary = SkillSummary::from(&skill);
        assert_eq!(summary.skill_id, "digest");
        assert_eq!(summary.version, "1.2.0");
        assert!(summary.enabled);
    }

    #[test]
    fn build_catalog_response_includes_all_skills() {
        let s1 = sample_skill("a", true);
        let s2 = sample_skill("b", false);
        let response = build_catalog_response(&[&s1, &s2]);
        assert_eq!(response.total, 2);
        assert_eq!(response.skills.len(), 2);
    }

    #[test]
    fn invoke_skill_response_from_invocation() {
        let inv = SkillInvocation {
            invocation_id: "inv_42".to_owned(),
            skill_id: "content-pipeline".to_owned(),
            args: serde_json::json!({}),
            result: None,
            status: SkillInvocationStatus::Running,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };
        let resp = InvokeSkillResponse::from(&inv);
        assert_eq!(resp.invocation_id, "inv_42");
        assert_eq!(resp.skill_id, "content-pipeline");
        assert_eq!(resp.status, "running");
    }

    #[test]
    fn invoke_skill_response_serializes_correctly() {
        let inv = SkillInvocation {
            invocation_id: "inv_1".to_owned(),
            skill_id: "writer".to_owned(),
            args: serde_json::json!({}),
            result: None,
            status: SkillInvocationStatus::Completed,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        };
        let resp = InvokeSkillResponse::from(&inv);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json["invocationId"]
                .as_str()
                .or_else(|| json["invocation_id"].as_str()),
            Some("inv_1")
        );
        assert_eq!(json["status"], "completed");
    }

    #[test]
    fn catalog_response_serializes_to_json() {
        let skill = sample_skill("coder", true);
        let resp = build_catalog_response(&[&skill]);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["total"], 1);
        assert!(json["skills"].is_array());
        assert_eq!(
            json["skills"][0]["skillId"]
                .as_str()
                .or_else(|| json["skills"][0]["skill_id"].as_str()),
            Some("coder")
        );
    }
}
