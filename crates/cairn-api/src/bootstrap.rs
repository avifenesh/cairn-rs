use serde::{Deserialize, Serialize};

/// Deployment mode determines which features and defaults are active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentMode {
    Local,
    SelfHostedTeam,
}

/// Minimal bootstrap configuration for the API server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapConfig {
    pub mode: DeploymentMode,
    pub listen_addr: String,
    pub listen_port: u16,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            mode: DeploymentMode::Local,
            listen_addr: "127.0.0.1".to_owned(),
            listen_port: 3000,
        }
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
    fn default_bootstrap_is_local() {
        let config = BootstrapConfig::default();
        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.listen_port, 3000);
    }

    #[test]
    fn team_mode_config() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            listen_addr: "0.0.0.0".to_owned(),
            listen_port: 8080,
        };
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
    }
}
