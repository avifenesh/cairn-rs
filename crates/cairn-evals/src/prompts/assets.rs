use cairn_domain::{PromptAssetId, Scope, TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Prompt asset kind per RFC 006.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    System,
    UserTemplate,
    ToolPrompt,
    Critic,
    Router,
}

/// Prompt asset status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptAssetStatus {
    Active,
    Deprecated,
    Archived,
}

/// A prompt asset is the stable logical identity for a prompt family.
///
/// Assets are library objects scoped to tenant or workspace, not project.
/// They hold identity and metadata; content lives in `PromptVersion`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptAsset {
    pub prompt_asset_id: PromptAssetId,
    pub scope: Scope,
    pub tenant_id: TenantId,
    pub workspace_id: Option<WorkspaceId>,
    pub name: String,
    pub kind: PromptKind,
    pub status: PromptAssetStatus,
    pub created_at: u64,
    pub updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::{PromptAsset, PromptAssetStatus, PromptKind};
    use cairn_domain::{PromptAssetId, Scope, TenantId};

    #[test]
    fn prompt_asset_carries_identity() {
        let asset = PromptAsset {
            prompt_asset_id: PromptAssetId::new("prompt_planner_system"),
            scope: Scope::Tenant,
            tenant_id: TenantId::new("tenant_acme"),
            workspace_id: None,
            name: "planner.system".to_owned(),
            kind: PromptKind::System,
            status: PromptAssetStatus::Active,
            created_at: 1000,
            updated_at: 1000,
        };
        assert_eq!(asset.name, "planner.system");
        assert_eq!(asset.kind, PromptKind::System);
    }
}
