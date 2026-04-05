use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Settings summary for operator configuration view.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsSummary {
    pub deployment_mode: String,
    pub store_backend: String,
    pub plugin_count: u32,
}

/// Aggregate health counts for the running deployment (RFC 014).
///
/// Surfaces at a glance how many providers, plugins, credentials, and
/// degraded components are currently registered, without requiring the
/// operator to query each subsystem individually.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SystemHealthSettings {
    /// Total number of provider health records tracked.
    pub provider_health_count: u32,
    /// Total number of plugins registered with the plugin host.
    pub plugin_health_count: u32,
    /// Number of providers or plugins currently in a degraded state.
    pub degraded_count: u32,
    /// Number of credentials stored in the credential vault.
    pub credential_count: u32,
}

/// Encryption key management status for the deployment (RFC 014).
///
/// Exposes whether an encryption key is configured, its current version,
/// and when it was last rotated.  Used by the operator UI to surface
/// key-hygiene warnings (e.g. stale rotation).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeyManagementStatus {
    /// Whether an encryption key has been configured for the deployment.
    pub encryption_key_configured: bool,
    /// Monotonically increasing version counter incremented on each rotation.
    pub key_version: Option<u32>,
    /// Unix milliseconds of the most recent key rotation, if any.
    pub last_rotation_at: Option<u64>,
}

/// Full operator-facing deployment settings (RFC 014).
///
/// Extends the lightweight `SettingsSummary` with health and key-management
/// sub-structs.  Both sub-structs carry `#[serde(default)]` so that existing
/// serialised `DeploymentSettings` payloads that pre-date these fields remain
/// deserializable without error.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentSettings {
    pub deployment_mode: String,
    pub store_backend: String,
    pub plugin_count: u32,
    #[serde(default)]
    pub system_health: SystemHealthSettings,
    #[serde(default)]
    pub key_management: KeyManagementStatus,
}

/// Settings endpoints per RFC 010.
#[async_trait]
pub trait SettingsEndpoints: Send + Sync {
    type Error;
    async fn get_settings(&self) -> Result<SettingsSummary, Self::Error>;
}
