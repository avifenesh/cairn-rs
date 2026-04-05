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
    /// TLS configuration (optional; None disables TLS).
    #[serde(default)]
    pub tls_enabled: bool,
    #[serde(default)]
    pub tls_cert_path: Option<String>,
    #[serde(default)]
    pub tls_key_path: Option<String>,
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
            tls_enabled: false,
            tls_cert_path: None,
            tls_key_path: None,
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
            tls_enabled: false,
            tls_cert_path: None,
            tls_key_path: None,
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

/// Configuration validation errors (RFC 011).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigValidationError {
    /// SQLite is not permitted for team/production deployments.
    SqliteNotSupportedInTeamMode,
    /// Team mode requires an explicit encryption key (not LocalAuto).
    MissingEncryptionKeyInTeamMode,
}

impl std::fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigValidationError::SqliteNotSupportedInTeamMode => write!(
                f,
                "SQLite is not supported in team/production mode; use Postgres"
            ),
            ConfigValidationError::MissingEncryptionKeyInTeamMode => write!(
                f,
                "team mode requires an explicit encryption key (EnvVar or File)"
            ),
        }
    }
}

impl BootstrapConfig {
    /// RFC 011: validate deployment configuration before startup.
    ///
    /// Returns all constraint violations rather than stopping at the first.
    pub fn validate(&self) -> Result<(), Vec<ConfigValidationError>> {
        let mut errors = Vec::new();

        if self.mode == DeploymentMode::SelfHostedTeam {
            if matches!(self.storage, StorageBackend::Sqlite { .. }) {
                errors.push(ConfigValidationError::SqliteNotSupportedInTeamMode);
            }
            if matches!(
                self.encryption_key,
                EncryptionKeySource::None | EncryptionKeySource::LocalAuto
            ) {
                errors.push(ConfigValidationError::MissingEncryptionKeyInTeamMode);
            }
        }

        if errors.is_empty() { Ok(()) } else { Err(errors) }
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

    /// RFC 011: SQLite must be rejected for team/self-hosted mode.
    #[test]
    fn validate_rejects_sqlite_in_team_mode() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Sqlite { path: "cairn.db".to_owned() },
            encryption_key: EncryptionKeySource::EnvVar { var_name: "KEY".to_owned() },
            ..BootstrapConfig::default()
        };
        let errs = config.validate().unwrap_err();
        assert!(errs.contains(&ConfigValidationError::SqliteNotSupportedInTeamMode));
    }

    /// RFC 011: Postgres is accepted for team mode when a key is present.
    #[test]
    fn validate_accepts_postgres_in_team_mode_with_key() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Postgres { connection_url: "postgres://localhost/cairn".to_owned() },
            encryption_key: EncryptionKeySource::EnvVar { var_name: "CAIRN_KEY".to_owned() },
            ..BootstrapConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    /// RFC 011: local mode with SQLite is always valid.
    #[test]
    fn validate_accepts_sqlite_in_local_mode() {
        let config = BootstrapConfig::default(); // Local + SQLite + LocalAuto
        assert!(config.validate().is_ok());
    }

    /// RFC 011: team mode without any key must also fail validation.
    #[test]
    fn validate_rejects_missing_key_in_team_mode() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Postgres { connection_url: "postgres://localhost/cairn".to_owned() },
            encryption_key: EncryptionKeySource::None,
            ..BootstrapConfig::default()
        };
        let errs = config.validate().unwrap_err();
        assert!(errs.contains(&ConfigValidationError::MissingEncryptionKeyInTeamMode));
    }

    #[test]
    fn local_auto_key_rejected_in_team_mode() {
        let mut config = BootstrapConfig::team("postgres://localhost/cairn");
        config.encryption_key = EncryptionKeySource::LocalAuto;
        assert!(!config.credentials_available());
    }
}

// ── RFC 011 Gap Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod rfc011_tests {
    use super::*;

    /// RFC 011: team mode must NOT use SQLite — fail closed.
    #[test]
    fn rfc011_team_mode_rejects_sqlite_fail_closed() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Sqlite { path: "cairn.db".to_owned() },
            encryption_key: EncryptionKeySource::EnvVar { var_name: "KEY".to_owned() },
            ..BootstrapConfig::default()
        };
        let errs = config.validate().unwrap_err();
        assert!(
            errs.iter().any(|e| *e == ConfigValidationError::SqliteNotSupportedInTeamMode),
            "RFC 011: SQLite must be rejected in self-hosted team mode"
        );
    }

    /// RFC 011: team mode without encryption key must fail closed — credentials unavailable.
    #[test]
    fn rfc011_team_mode_without_key_fails_closed() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Postgres { connection_url: "postgres://localhost/cairn".to_owned() },
            encryption_key: EncryptionKeySource::None,
            ..BootstrapConfig::default()
        };
        assert!(
            !config.credentials_available(),
            "RFC 011: credentials must be unavailable when no key is configured"
        );
        let errs = config.validate().unwrap_err();
        assert!(
            errs.iter().any(|e| *e == ConfigValidationError::MissingEncryptionKeyInTeamMode),
            "RFC 011: missing encryption key must be a validation error"
        );
    }

    /// RFC 011: all four canonical server roles must be definable.
    #[test]
    fn rfc011_all_four_server_roles_are_defined() {
        use ServerRole::*;
        // RFC 011: canonical roles: API, RuntimeWorker, Scheduler, PluginHost
        let all_roles = [Api, RuntimeWorker, Scheduler, PluginHost];
        for role in &all_roles {
            // Each role must be expressible and distinct.
            let config = BootstrapConfig::default();
            assert!(config.has_role(*role),
                "RFC 011: all-in-one default must have role {:?}", role);
        }
    }

    /// RFC 011: local mode is a valid first-class deployment.
    #[test]
    fn rfc011_local_mode_is_first_class_valid_deployment() {
        let config = BootstrapConfig::default();
        assert_eq!(config.mode, DeploymentMode::Local);
        assert!(config.validate().is_ok(),
            "RFC 011: default local mode must pass validation");
        assert!(config.credentials_available(),
            "RFC 011: local mode with LocalAuto key must have credentials available");
    }

    /// RFC 011: self-hosted team mode with valid postgres+key passes.
    #[test]
    fn rfc011_team_mode_with_postgres_and_key_is_valid() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Postgres { connection_url: "postgres://localhost/cairn".to_owned() },
            encryption_key: EncryptionKeySource::EnvVar { var_name: "CAIRN_KEK".to_owned() },
            ..BootstrapConfig::default()
        };
        assert!(config.validate().is_ok(),
            "RFC 011: team mode with Postgres and env key must pass validation");
    }
}
