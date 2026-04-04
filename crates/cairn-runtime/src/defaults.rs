use async_trait::async_trait;
use cairn_domain::{DefaultSetting, ProjectKey, Scope};

use crate::error::RuntimeError;

#[async_trait]
pub trait DefaultsService: Send + Sync {
    async fn set(
        &self,
        scope: Scope,
        scope_id: String,
        key: String,
        value: serde_json::Value,
    ) -> Result<DefaultSetting, RuntimeError>;

    async fn clear(&self, scope: Scope, scope_id: String, key: String) -> Result<(), RuntimeError>;

    async fn resolve(
        &self,
        project_key: &ProjectKey,
        key: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError>;
}
