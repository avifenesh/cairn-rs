use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::permissions::Permission;

/// Configuration for supervised-process execution class.
///
/// Supervised processes run as host-managed child processes with:
/// - process boundary isolation
/// - scoped permission grants
/// - allowlisted environment inheritance
/// - host-enforced timeout and cancellation
/// - host-enforced resource limits
///
/// Filesystem and network scope are policy-bounded but NOT guaranteed
/// by an additional OS-level sandbox boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisedProcessConfig {
    pub working_dir: Option<PathBuf>,
    pub allowed_env: Vec<String>,
    pub granted_permissions: Vec<Permission>,
    pub timeout_ms: u64,
    pub max_memory_bytes: Option<u64>,
    pub max_file_descriptors: Option<u32>,
}

impl Default for SupervisedProcessConfig {
    fn default() -> Self {
        Self {
            working_dir: None,
            allowed_env: Vec::new(),
            granted_permissions: Vec::new(),
            timeout_ms: 30_000,
            max_memory_bytes: None,
            max_file_descriptors: None,
        }
    }
}

/// Boundary trait for supervised-process execution.
/// Implementors manage the child process lifecycle without OS-level sandboxing.
pub trait SupervisedBoundary {
    type Error;

    fn spawn(
        &mut self,
        config: &SupervisedProcessConfig,
        command: &[String],
    ) -> Result<u32, Self::Error>;
    fn is_alive(&self, pid: u32) -> bool;
    fn terminate(&mut self, pid: u32) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::Permission;

    #[test]
    fn default_config_has_sane_timeout() {
        let config = SupervisedProcessConfig::default();

        assert_eq!(config.timeout_ms, 30_000);
        assert!(config.allowed_env.is_empty());
        assert!(config.granted_permissions.is_empty());
        assert!(config.working_dir.is_none());
    }

    #[test]
    fn config_carries_permission_grants() {
        let config = SupervisedProcessConfig {
            granted_permissions: vec![Permission::FsRead, Permission::ProcessExec],
            ..Default::default()
        };

        assert_eq!(config.granted_permissions.len(), 2);
    }
}
