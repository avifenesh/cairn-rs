use cairn_domain::{OperatorId, PromptAssetId, PromptVersionId};
use serde::{Deserialize, Serialize};

/// Content format for a prompt version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptFormat {
    PlainText,
    Mustache,
    Jinja2,
}

/// An immutable snapshot of a prompt asset's content.
///
/// Per RFC 006: versions are immutable. A changed prompt body always
/// creates a new version. `content_hash` is used for integrity and
/// dedup checks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptVersion {
    pub prompt_version_id: PromptVersionId,
    pub prompt_asset_id: PromptAssetId,
    pub version_number: u32,
    pub content: String,
    pub format: PromptFormat,
    pub content_hash: String,
    pub metadata: PromptVersionMetadata,
    pub created_by: Option<OperatorId>,
    pub created_at: u64,
}

/// Optional metadata attached to a prompt version.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PromptVersionMetadata {
    pub model_hints: Vec<String>,
    pub intended_task_types: Vec<String>,
    pub expected_tools: Vec<String>,
    pub safety_notes: Option<String>,
    pub deprecation_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{PromptFormat, PromptVersion, PromptVersionMetadata};
    use cairn_domain::{PromptAssetId, PromptVersionId};

    #[test]
    fn prompt_version_is_immutable_by_design() {
        let v = PromptVersion {
            prompt_version_id: PromptVersionId::new("pv_1"),
            prompt_asset_id: PromptAssetId::new("prompt_planner"),
            version_number: 1,
            content: "You are a planner.".to_owned(),
            format: PromptFormat::PlainText,
            content_hash: "abc123".to_owned(),
            metadata: PromptVersionMetadata::default(),
            created_by: None,
            created_at: 1000,
        };
        assert_eq!(v.version_number, 1);
        assert_eq!(v.format, PromptFormat::PlainText);
    }
}
