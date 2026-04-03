use serde::{Deserialize, Serialize};

/// Deployment mode determines which features and defaults are active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentMode {
    Local,
    SelfHostedTeam,
}

/// Canonical deployment roles per RFC 011.
///
/// Small deployments run all roles together. Team/production deployments
/// may split roles across processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerRole {
    Api,
    RuntimeWorker,
    Scheduler,
    PluginHost,
}

/// Storage backend selection per RFC 011.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageBackend {
    Sqlite { path: String },
    Postgres { connection_url: String },
}

impl Default for StorageBackend {
    fn default() -> Self {
        StorageBackend::Sqlite {
            path: "cairn.db".to_owned(),
        }
    }
}

/// Encryption key source for credential encryption at rest (RFC 011).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionKeySource {
    /// Operator-supplied key from environment variable.
    EnvVar { var_name: String },
    /// Operator-supplied key from file path.
    File { path: String },
    /// Auto-generated local key (local mode convenience only).
    LocalAuto,
    /// No key configured — credential features fail closed in team mode.
    None,
}

impl Default for EncryptionKeySource {
    fn default() -> Self {
        EncryptionKeySource::None
    }
}

/// Full deployment configuration per RFC 011.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapConfig {
    pub mode: DeploymentMode,
    pub listen_addr: String,
    pub listen_port: u16,
    pub roles: Vec<ServerRole>,
    pub storage: StorageBackend,
    pub encryption_key: EncryptionKeySource,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            mode: DeploymentMode::Local,
            listen_addr: "127.0.0.1".to_owned(),
            listen_port: 3000,
            roles: vec![
                ServerRole::Api,
                ServerRole::RuntimeWorker,
                ServerRole::Scheduler,
                ServerRole::PluginHost,
            ],
            storage: StorageBackend::default(),
            encryption_key: EncryptionKeySource::LocalAuto,
        }
    }
}

impl BootstrapConfig {
    /// Create a team-mode config with Postgres.
    pub fn team(connection_url: impl Into<String>) -> Self {
        Self {
            mode: DeploymentMode::SelfHostedTeam,
            listen_addr: "0.0.0.0".to_owned(),
            listen_port: 3000,
            roles: vec![
                ServerRole::Api,
                ServerRole::RuntimeWorker,
                ServerRole::Scheduler,
                ServerRole::PluginHost,
            ],
            storage: StorageBackend::Postgres {
                connection_url: connection_url.into(),
            },
            encryption_key: EncryptionKeySource::None,
        }
    }

    /// Check if credential features should be available.
    pub fn credentials_available(&self) -> bool {
        match (&self.mode, &self.encryption_key) {
            (_, EncryptionKeySource::None) => false,
            (DeploymentMode::Local, EncryptionKeySource::LocalAuto) => true,
            (DeploymentMode::SelfHostedTeam, EncryptionKeySource::LocalAuto) => false,
            _ => true,
        }
    }

    /// Check if a specific role is enabled.
    pub fn has_role(&self, role: ServerRole) -> bool {
        self.roles.contains(&role)
    }
}

/// Seam for server bootstrap. Implementors start the HTTP/SSE server.
pub trait ServerBootstrap {
    type Error;

    fn start(&self, config: &BootstrapConfig) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bootstrap_is_local_with_all_roles() {
        let config = BootstrapConfig::default();
        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.listen_port, 3000);
        assert!(config.has_role(ServerRole::Api));
        assert!(config.has_role(ServerRole::RuntimeWorker));
        assert!(config.has_role(ServerRole::Scheduler));
        assert!(config.has_role(ServerRole::PluginHost));
        assert!(matches!(config.storage, StorageBackend::Sqlite { .. }));
    }

    #[test]
    fn team_mode_uses_postgres() {
        let config = BootstrapConfig::team("postgres://localhost/cairn");
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
        assert_eq!(config.listen_addr, "0.0.0.0");
        assert!(matches!(config.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn credentials_available_in_local_mode_with_auto_key() {
        let config = BootstrapConfig::default();
        assert!(config.credentials_available());
    }

    #[test]
    fn credentials_fail_closed_in_team_mode_without_key() {
        let config = BootstrapConfig::team("postgres://localhost/cairn");
        assert!(!config.credentials_available());
    }

    #[test]
    fn credentials_available_in_team_mode_with_env_key() {
        let mut config = BootstrapConfig::team("postgres://localhost/cairn");
        config.encryption_key = EncryptionKeySource::EnvVar {
            var_name: "CAIRN_KEY".to_owned(),
        };
        assert!(config.credentials_available());
    }

    #[test]
    fn local_auto_key_rejected_in_team_mode() {
        let mut config = BootstrapConfig::team("postgres://localhost/cairn");
        config.encryption_key = EncryptionKeySource::LocalAuto;
        assert!(!config.credentials_available());
    }
}
