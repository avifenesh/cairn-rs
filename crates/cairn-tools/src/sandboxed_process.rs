use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::permissions::Permission;

/// Configuration for sandboxed-process execution class.
///
/// Sandboxed processes provide all supervised-process guarantees PLUS
/// enforced OS-level confinement per RFC 007:
/// - read-only root filesystem by default
/// - only explicitly granted writable scratch paths
/// - network disabled by default, explicit egress grants only
/// - no ambient privilege escalation
/// - seccomp-equivalent syscall restriction
/// - host-enforced CPU/memory/process/fd limits
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxedProcessConfig {
    pub writable_paths: Vec<PathBuf>,
    pub network_egress_allowed: bool,
    pub allowed_env: Vec<String>,
    pub granted_permissions: Vec<Permission>,
    pub timeout_ms: u64,
    pub max_memory_bytes: Option<u64>,
    pub max_cpu_shares: Option<u32>,
    pub max_processes: Option<u32>,
    pub max_file_descriptors: Option<u32>,
}

impl Default for SandboxedProcessConfig {
    fn default() -> Self {
        Self {
            writable_paths: Vec::new(),
            network_egress_allowed: false,
            allowed_env: Vec::new(),
            granted_permissions: Vec::new(),
            timeout_ms: 30_000,
            max_memory_bytes: None,
            max_cpu_shares: None,
            max_processes: None,
            max_file_descriptors: None,
        }
    }
}

/// Boundary trait for sandboxed-process execution.
/// Implementors provide OS-level confinement beyond simple process supervision.
pub trait SandboxedBoundary {
    type Error;

    fn spawn(
        &mut self,
        config: &SandboxedProcessConfig,
        command: &[String],
    ) -> Result<u32, Self::Error>;
    fn is_alive(&self, pid: u32) -> bool;
    fn terminate(&mut self, pid: u32) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sandbox_denies_network() {
        let config = SandboxedProcessConfig::default();

        assert!(!config.network_egress_allowed);
        assert!(config.writable_paths.is_empty());
        assert_eq!(config.timeout_ms, 30_000);
    }

    #[test]
    fn sandbox_config_explicit_grants() {
        let config = SandboxedProcessConfig {
            writable_paths: vec![PathBuf::from("/tmp/scratch")],
            network_egress_allowed: true,
            granted_permissions: vec![
                Permission::FsRead,
                Permission::FsWrite,
                Permission::NetworkEgress,
            ],
            max_memory_bytes: Some(512 * 1024 * 1024),
            max_processes: Some(16),
            ..Default::default()
        };

        assert!(config.network_egress_allowed);
        assert_eq!(config.writable_paths.len(), 1);
        assert_eq!(config.granted_permissions.len(), 3);
        assert_eq!(config.max_memory_bytes, Some(512 * 1024 * 1024));
    }
}
