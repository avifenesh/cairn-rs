use cairn_domain::policy::ExecutionClass;

use crate::plugins::PluginManifest;
use crate::sandboxed_process::SandboxedProcessConfig;
use crate::supervised_process::SupervisedProcessConfig;

/// Deployment mode affects execution-class defaults per RFC 007.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeploymentMode {
    Local,
    SelfHostedTeam,
}

/// Selected execution configuration for a plugin invocation.
#[derive(Clone, Debug)]
pub enum SelectedConfig {
    Supervised(SupervisedProcessConfig),
    Sandboxed(SandboxedProcessConfig),
}

impl SelectedConfig {
    pub fn execution_class(&self) -> ExecutionClass {
        match self {
            SelectedConfig::Supervised(_) => ExecutionClass::SupervisedProcess,
            SelectedConfig::Sandboxed(_) => ExecutionClass::SandboxedProcess,
        }
    }

    pub fn timeout_ms(&self) -> u64 {
        match self {
            SelectedConfig::Supervised(c) => c.timeout_ms,
            SelectedConfig::Sandboxed(c) => c.timeout_ms,
        }
    }
}

/// Selects the execution-class configuration for a plugin based on:
/// 1. Manifest-declared execution class
/// 2. Deployment mode defaults
/// 3. Manifest-declared timeout/limits
///
/// Per RFC 007:
/// - Local mode: `supervised_process` is sufficient as default
/// - Self-hosted team mode: customer-installed plugins default to `sandboxed_process`
/// - Manifest may explicitly request `sandboxed_process` in any mode
pub fn select_execution_config(manifest: &PluginManifest, mode: DeploymentMode) -> SelectedConfig {
    let effective_class = match (manifest.execution_class, mode) {
        // Manifest explicitly requests sandboxed — always honor
        (ExecutionClass::SandboxedProcess, _) => ExecutionClass::SandboxedProcess,
        // In team mode, default to sandboxed for safety
        (ExecutionClass::SupervisedProcess, DeploymentMode::SelfHostedTeam) => {
            ExecutionClass::SandboxedProcess
        }
        // Local mode with supervised manifest — use supervised
        (ExecutionClass::SupervisedProcess, DeploymentMode::Local) => {
            ExecutionClass::SupervisedProcess
        }
    };

    let timeout_ms = manifest
        .limits
        .as_ref()
        .and_then(|l| l.default_timeout_ms)
        .unwrap_or(30_000);

    let _max_concurrency = manifest.limits.as_ref().and_then(|l| l.max_concurrency);

    match effective_class {
        ExecutionClass::SupervisedProcess => {
            let mut config = SupervisedProcessConfig::default();
            config.timeout_ms = timeout_ms;
            config.granted_permissions = manifest.permissions.permissions.clone();
            SelectedConfig::Supervised(config)
        }
        ExecutionClass::SandboxedProcess => {
            let mut config = SandboxedProcessConfig::default();
            config.timeout_ms = timeout_ms;
            config.granted_permissions = manifest.permissions.permissions.clone();
            SelectedConfig::Sandboxed(config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{DeclaredPermissions, Permission};
    use crate::plugins::{PluginCapability, PluginLimits, PluginManifest};
    use cairn_domain::policy::ExecutionClass;

    fn test_manifest(exec_class: ExecutionClass) -> PluginManifest {
        PluginManifest {
            id: "test.plugin".to_owned(),
            name: "Test".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["test-bin".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["test.tool".to_owned()],
            }],
            permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
            limits: Some(PluginLimits {
                max_concurrency: Some(4),
                default_timeout_ms: Some(15_000),
            }),
            execution_class: exec_class,
            description: None,
            homepage: None,
        }
    }

    #[test]
    fn local_mode_supervised_stays_supervised() {
        let manifest = test_manifest(ExecutionClass::SupervisedProcess);
        let config = select_execution_config(&manifest, DeploymentMode::Local);

        assert_eq!(config.execution_class(), ExecutionClass::SupervisedProcess);
        assert_eq!(config.timeout_ms(), 15_000);
    }

    #[test]
    fn team_mode_promotes_to_sandboxed() {
        let manifest = test_manifest(ExecutionClass::SupervisedProcess);
        let config = select_execution_config(&manifest, DeploymentMode::SelfHostedTeam);

        assert_eq!(config.execution_class(), ExecutionClass::SandboxedProcess);
    }

    #[test]
    fn explicit_sandboxed_always_honored() {
        let manifest = test_manifest(ExecutionClass::SandboxedProcess);

        let local = select_execution_config(&manifest, DeploymentMode::Local);
        assert_eq!(local.execution_class(), ExecutionClass::SandboxedProcess);

        let team = select_execution_config(&manifest, DeploymentMode::SelfHostedTeam);
        assert_eq!(team.execution_class(), ExecutionClass::SandboxedProcess);
    }

    #[test]
    fn timeout_from_manifest_limits() {
        let manifest = test_manifest(ExecutionClass::SupervisedProcess);
        let config = select_execution_config(&manifest, DeploymentMode::Local);
        assert_eq!(config.timeout_ms(), 15_000);
    }

    #[test]
    fn default_timeout_when_no_limits() {
        let mut manifest = test_manifest(ExecutionClass::SupervisedProcess);
        manifest.limits = None;
        let config = select_execution_config(&manifest, DeploymentMode::Local);
        assert_eq!(config.timeout_ms(), 30_000);
    }
}
