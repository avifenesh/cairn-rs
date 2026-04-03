use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Settings summary for operator configuration view.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsSummary {
    pub deployment_mode: String,
    pub store_backend: String,
    pub plugin_count: u32,
}

/// Settings endpoints per RFC 010.
#[async_trait]
pub trait SettingsEndpoints: Send + Sync {
    type Error;
    async fn get_settings(&self) -> Result<SettingsSummary, Self::Error>;
}
