use async_trait::async_trait;
use cairn_domain::{DefaultSetting, Scope};

use crate::error::StoreError;

#[async_trait]
pub trait DefaultsReadModel: Send + Sync {
    async fn get(
        &self,
        scope: Scope,
        scope_id: &str,
        key: &str,
    ) -> Result<Option<DefaultSetting>, StoreError>;

    async fn list_by_scope(
        &self,
        scope: Scope,
        scope_id: &str,
    ) -> Result<Vec<DefaultSetting>, StoreError>;
}
