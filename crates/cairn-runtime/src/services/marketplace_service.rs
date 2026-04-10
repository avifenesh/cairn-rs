//! Plugin marketplace service — RFC 015.
//!
//! This is the layer above the existing plugin host (RFC 007). It manages:
//! - marketplace lifecycle (Listed → Installing → Installed → EnabledForProject → Uninstalled)
//! - per-project plugin enablement with tool/signal allowlists
//! - credential wizard flow
//! - bundled catalog loading
//!
//! There is NO `Connected` state and NO `POST /v1/plugins/:id/connect` endpoint.
//! Process instances are managed by the plugin host, not the marketplace.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::ids::{CredentialId, OperatorId, SignalId};
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::contexts::SignalCaptureOverride;
use serde::{Deserialize, Serialize};

// ── Marketplace State ────────────────────────────────────────────────────────

/// Lifecycle state of a plugin in the marketplace (tenant-scoped).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceState {
    Listed,
    Installing,
    Installed,
    InstallationFailed { reason: String },
    Uninstalled,
}

/// Where a plugin descriptor came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptorSource {
    BundledCatalog,
    LocalFile { path: String },
    RemoteUrl { url: String },
}

/// Credential scope key — derived from credential IDs bound to an enablement.
/// Process instances are keyed 1:1 with credential scope keys.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CredentialScopeKey(pub String);

/// Reason a process instance became ready.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceReadyReason {
    EagerSignalSource,
    LazyFirstInvocation,
    Restart,
}

/// Reason a process instance stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStoppedReason {
    Drained,
    Failed { details: String },
    Uninstalled,
}

/// Outcome of an ephemeral credential verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Ok,
    Failed { reason: String },
}

// ── Marketplace Commands (RFC 015 §Commands) ────────────────────────────────

/// Commands handled by the marketplace service.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum MarketplaceCommand {
    InstallPlugin {
        plugin_id: String,
        initiated_by: OperatorId,
    },
    ProvidePluginCredentials {
        plugin_id: String,
        credentials: Vec<(String, String)>,
        provided_by: OperatorId,
    },
    VerifyPluginCredentials {
        plugin_id: String,
        credential_scope_key: Option<CredentialScopeKey>,
        verified_by: OperatorId,
    },
    EnablePluginForProject {
        plugin_id: String,
        project: ProjectKey,
        tool_allowlist: Option<Vec<String>>,
        signal_allowlist: Option<Vec<String>>,
        signal_capture_override: Option<SignalCaptureOverride>,
        enabled_by: OperatorId,
    },
    DisablePluginForProject {
        plugin_id: String,
        project: ProjectKey,
        disabled_by: OperatorId,
    },
    UninstallPlugin {
        plugin_id: String,
        uninstalled_by: OperatorId,
    },
}

// ── Marketplace Events (RFC 015 §Events) ────────────────────────────────────

/// Events emitted by the marketplace service.
/// These will be integrated into `RuntimeEvent` in cairn-domain once
/// the event structs are wired in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum MarketplaceEvent {
    PluginListed {
        plugin_id: String,
        source: DescriptorSource,
        listed_at: u64,
    },
    PluginInstallationStarted {
        plugin_id: String,
        initiated_by: OperatorId,
        started_at: u64,
    },
    PluginInstalled {
        plugin_id: String,
        manifest_hash: String,
        at: u64,
    },
    PluginInstallationFailed {
        plugin_id: String,
        reason: String,
        at: u64,
    },
    PluginCredentialsProvided {
        plugin_id: String,
        credential_ids: Vec<CredentialId>,
        provided_by: OperatorId,
        at: u64,
    },
    PluginInstanceReady {
        plugin_id: String,
        credential_scope_key: CredentialScopeKey,
        reason: InstanceReadyReason,
        at: u64,
    },
    PluginInstanceStopped {
        plugin_id: String,
        credential_scope_key: CredentialScopeKey,
        reason: InstanceStoppedReason,
        at: u64,
    },
    PluginCredentialsVerified {
        plugin_id: String,
        credential_scope_key: CredentialScopeKey,
        outcome: VerificationOutcome,
        verified_by: OperatorId,
        at: u64,
    },
    PluginEnabledForProject {
        plugin_id: String,
        project: ProjectKey,
        enabled_by: OperatorId,
        tool_allowlist: Option<Vec<String>>,
        signal_allowlist: Option<Vec<String>>,
        signal_capture_override: Option<SignalCaptureOverride>,
        at: u64,
    },
    PluginDisabledForProject {
        plugin_id: String,
        project: ProjectKey,
        disabled_by: OperatorId,
        at: u64,
    },
    PluginUninstalled {
        plugin_id: String,
        uninstalled_by: OperatorId,
        credentials_revoked: Vec<CredentialId>,
        at: u64,
    },
    SignalProjectedToGraph {
        signal_id: SignalId,
        plugin_id: String,
        project: ProjectKey,
        node_id: String,
        at: u64,
    },
    SignalIngestedToMemory {
        signal_id: SignalId,
        plugin_id: String,
        project: ProjectKey,
        source_id: String,
        chunks_created: u32,
        at: u64,
    },
}

// ── Per-Plugin Marketplace Record ────────────────────────────────────────────

/// In-memory projection of a plugin's marketplace state (tenant-scoped).
#[derive(Clone, Debug)]
pub struct MarketplaceRecord {
    pub plugin_id: String,
    pub state: MarketplaceState,
    pub source: DescriptorSource,
    pub manifest_hash: Option<String>,
    pub credential_ids: Vec<CredentialId>,
    pub listed_at: u64,
    pub installed_at: Option<u64>,
}

/// Per-project plugin enablement record (project-scoped).
#[derive(Clone, Debug)]
pub struct PluginEnablement {
    pub plugin_id: String,
    pub project: ProjectKey,
    pub enabled: bool,
    pub enabled_at: u64,
    pub enabled_by: OperatorId,
    pub tool_allowlist: Option<Vec<String>>,
    pub signal_allowlist: Option<Vec<String>>,
    pub signal_capture_override: Option<SignalCaptureOverride>,
}

// ── MarketplaceService ──────────────────────────────────────────────────────

/// The marketplace service sits above the plugin host and manages the
/// discover → install → credential → enable → disable → uninstall lifecycle.
pub struct MarketplaceService<S> {
    store: Arc<S>,
    /// Plugin marketplace records, keyed by plugin_id.
    records: HashMap<String, MarketplaceRecord>,
    /// Per-project enablements, keyed by (plugin_id, project).
    enablements: HashMap<(String, ProjectKey), PluginEnablement>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl<S> MarketplaceService<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            records: HashMap::new(),
            enablements: HashMap::new(),
        }
    }

    /// Process a marketplace command and return the resulting events.
    pub fn handle_command(
        &mut self,
        command: MarketplaceCommand,
    ) -> Result<Vec<MarketplaceEvent>, MarketplaceError> {
        match command {
            MarketplaceCommand::InstallPlugin {
                plugin_id,
                initiated_by,
            } => self.handle_install(plugin_id, initiated_by),

            MarketplaceCommand::ProvidePluginCredentials {
                plugin_id,
                credentials,
                provided_by,
            } => self.handle_provide_credentials(plugin_id, credentials, provided_by),

            MarketplaceCommand::VerifyPluginCredentials {
                plugin_id,
                credential_scope_key,
                verified_by,
            } => self.handle_verify_credentials(plugin_id, credential_scope_key, verified_by),

            MarketplaceCommand::EnablePluginForProject {
                plugin_id,
                project,
                tool_allowlist,
                signal_allowlist,
                signal_capture_override,
                enabled_by,
            } => self.handle_enable(
                plugin_id,
                project,
                tool_allowlist,
                signal_allowlist,
                signal_capture_override,
                enabled_by,
            ),

            MarketplaceCommand::DisablePluginForProject {
                plugin_id,
                project,
                disabled_by,
            } => self.handle_disable(plugin_id, project, disabled_by),

            MarketplaceCommand::UninstallPlugin {
                plugin_id,
                uninstalled_by,
            } => self.handle_uninstall(plugin_id, uninstalled_by),
        }
    }

    /// Register a plugin from the catalog as Listed.
    pub fn list_plugin(
        &mut self,
        plugin_id: String,
        source: DescriptorSource,
    ) -> MarketplaceEvent {
        let now = now_ms();
        let record = MarketplaceRecord {
            plugin_id: plugin_id.clone(),
            state: MarketplaceState::Listed,
            source: source.clone(),
            manifest_hash: None,
            credential_ids: Vec::new(),
            listed_at: now,
            installed_at: None,
        };
        self.records.insert(plugin_id.clone(), record);

        MarketplaceEvent::PluginListed {
            plugin_id,
            source,
            listed_at: now,
        }
    }

    /// Query: is a plugin installed and available?
    pub fn get_record(&self, plugin_id: &str) -> Option<&MarketplaceRecord> {
        self.records.get(plugin_id)
    }

    /// Query: is a plugin enabled for a project?
    pub fn get_enablement(
        &self,
        plugin_id: &str,
        project: &ProjectKey,
    ) -> Option<&PluginEnablement> {
        self.enablements
            .get(&(plugin_id.to_string(), project.clone()))
    }

    /// Query: list all enablements for a project.
    pub fn enablements_for_project(&self, project: &ProjectKey) -> Vec<&PluginEnablement> {
        self.enablements
            .values()
            .filter(|e| &e.project == project && e.enabled)
            .collect()
    }

    // ── Command Handlers ─────────────────────────────────────────────────

    fn handle_install(
        &mut self,
        plugin_id: String,
        initiated_by: OperatorId,
    ) -> Result<Vec<MarketplaceEvent>, MarketplaceError> {
        let record = self
            .records
            .get(&plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(plugin_id.clone()))?;

        match &record.state {
            MarketplaceState::Listed | MarketplaceState::InstallationFailed { .. } => {}
            MarketplaceState::Uninstalled => {}
            _ => {
                return Err(MarketplaceError::InvalidTransition {
                    plugin_id: plugin_id.clone(),
                    from: format!("{:?}", record.state),
                    to: "Installing".to_string(),
                });
            }
        }

        let now = now_ms();
        let started = MarketplaceEvent::PluginInstallationStarted {
            plugin_id: plugin_id.clone(),
            initiated_by,
            started_at: now,
        };

        // Transition to Installing
        if let Some(record) = self.records.get_mut(&plugin_id) {
            record.state = MarketplaceState::Installing;
        }

        // For now, immediately mark as installed (real impl will do async download)
        let installed = MarketplaceEvent::PluginInstalled {
            plugin_id: plugin_id.clone(),
            manifest_hash: String::new(), // populated during real install
            at: now,
        };

        if let Some(record) = self.records.get_mut(&plugin_id) {
            record.state = MarketplaceState::Installed;
            record.installed_at = Some(now);
        }

        Ok(vec![started, installed])
    }

    fn handle_provide_credentials(
        &mut self,
        plugin_id: String,
        _credentials: Vec<(String, String)>,
        provided_by: OperatorId,
    ) -> Result<Vec<MarketplaceEvent>, MarketplaceError> {
        let record = self
            .records
            .get(&plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(plugin_id.clone()))?;

        if record.state != MarketplaceState::Installed {
            return Err(MarketplaceError::InvalidTransition {
                plugin_id: plugin_id.clone(),
                from: format!("{:?}", record.state),
                to: "CredentialsProvided".to_string(),
            });
        }

        let now = now_ms();
        // Real impl: validate credential shape, store via CredentialService
        let event = MarketplaceEvent::PluginCredentialsProvided {
            plugin_id,
            credential_ids: Vec::new(), // populated by CredentialService
            provided_by,
            at: now,
        };
        Ok(vec![event])
    }

    fn handle_verify_credentials(
        &self,
        plugin_id: String,
        credential_scope_key: Option<CredentialScopeKey>,
        verified_by: OperatorId,
    ) -> Result<Vec<MarketplaceEvent>, MarketplaceError> {
        let record = self
            .records
            .get(&plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(plugin_id.clone()))?;

        if record.state != MarketplaceState::Installed {
            return Err(MarketplaceError::PluginNotInstalled(plugin_id.clone()));
        }

        let now = now_ms();
        let scope_key =
            credential_scope_key.unwrap_or_else(|| CredentialScopeKey("tenant-default".into()));

        // Real impl: spawns a transient process, runs health check, shuts down.
        // This is an ephemeral action — no persistent state change.
        let event = MarketplaceEvent::PluginCredentialsVerified {
            plugin_id,
            credential_scope_key: scope_key,
            outcome: VerificationOutcome::Ok,
            verified_by,
            at: now,
        };
        Ok(vec![event])
    }

    fn handle_enable(
        &mut self,
        plugin_id: String,
        project: ProjectKey,
        tool_allowlist: Option<Vec<String>>,
        signal_allowlist: Option<Vec<String>>,
        signal_capture_override: Option<SignalCaptureOverride>,
        enabled_by: OperatorId,
    ) -> Result<Vec<MarketplaceEvent>, MarketplaceError> {
        let record = self
            .records
            .get(&plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(plugin_id.clone()))?;

        if record.state != MarketplaceState::Installed {
            return Err(MarketplaceError::PluginNotInstalled(plugin_id.clone()));
        }

        let now = now_ms();
        let enablement = PluginEnablement {
            plugin_id: plugin_id.clone(),
            project: project.clone(),
            enabled: true,
            enabled_at: now,
            enabled_by: enabled_by.clone(),
            tool_allowlist: tool_allowlist.clone(),
            signal_allowlist: signal_allowlist.clone(),
            signal_capture_override: signal_capture_override.clone(),
        };
        self.enablements
            .insert((plugin_id.clone(), project.clone()), enablement);

        let event = MarketplaceEvent::PluginEnabledForProject {
            plugin_id,
            project,
            enabled_by,
            tool_allowlist,
            signal_allowlist,
            signal_capture_override,
            at: now,
        };
        Ok(vec![event])
    }

    fn handle_disable(
        &mut self,
        plugin_id: String,
        project: ProjectKey,
        disabled_by: OperatorId,
    ) -> Result<Vec<MarketplaceEvent>, MarketplaceError> {
        let key = (plugin_id.clone(), project.clone());
        let enablement = self
            .enablements
            .get_mut(&key)
            .ok_or_else(|| MarketplaceError::NotEnabledForProject {
                plugin_id: plugin_id.clone(),
                project: project.clone(),
            })?;

        enablement.enabled = false;

        let now = now_ms();
        let event = MarketplaceEvent::PluginDisabledForProject {
            plugin_id,
            project,
            disabled_by,
            at: now,
        };
        Ok(vec![event])
    }

    fn handle_uninstall(
        &mut self,
        plugin_id: String,
        uninstalled_by: OperatorId,
    ) -> Result<Vec<MarketplaceEvent>, MarketplaceError> {
        let record = self
            .records
            .get(&plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(plugin_id.clone()))?;

        if record.state == MarketplaceState::Listed {
            return Err(MarketplaceError::InvalidTransition {
                plugin_id: plugin_id.clone(),
                from: "Listed".to_string(),
                to: "Uninstalled".to_string(),
            });
        }

        let now = now_ms();
        let credentials_revoked = record.credential_ids.clone();

        // Disable all project enablements
        let affected_keys: Vec<_> = self
            .enablements
            .keys()
            .filter(|(pid, _)| pid == &plugin_id)
            .cloned()
            .collect();
        for key in affected_keys {
            self.enablements.remove(&key);
        }

        // Transition to Uninstalled
        if let Some(record) = self.records.get_mut(&plugin_id) {
            record.state = MarketplaceState::Uninstalled;
        }

        let event = MarketplaceEvent::PluginUninstalled {
            plugin_id,
            uninstalled_by,
            credentials_revoked,
            at: now,
        };
        Ok(vec![event])
    }
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Marketplace-specific errors.
#[derive(Clone, Debug)]
pub enum MarketplaceError {
    PluginNotFound(String),
    PluginNotInstalled(String),
    InvalidTransition {
        plugin_id: String,
        from: String,
        to: String,
    },
    NotEnabledForProject {
        plugin_id: String,
        project: ProjectKey,
    },
    EvalScorerReserved,
}

impl std::fmt::Display for MarketplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PluginNotFound(id) => write!(f, "plugin not found: {id}"),
            Self::PluginNotInstalled(id) => write!(f, "plugin not installed: {id}"),
            Self::InvalidTransition {
                plugin_id,
                from,
                to,
            } => write!(
                f,
                "invalid state transition for plugin {plugin_id}: {from} -> {to}"
            ),
            Self::NotEnabledForProject { plugin_id, project } => write!(
                f,
                "plugin {plugin_id} not enabled for project {project:?}"
            ),
            Self::EvalScorerReserved => write!(
                f,
                "EvalScorer capability is reserved for a future RFC; v1 does not support plugin-provided eval scorers"
            ),
        }
    }
}

impl std::error::Error for MarketplaceError {}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::tenancy::ProjectKey;

    fn test_store() -> Arc<()> {
        Arc::new(())
    }

    fn operator() -> OperatorId {
        OperatorId::new("op-1")
    }

    fn project_p1() -> ProjectKey {
        ProjectKey::new("t1", "w1", "p1")
    }

    #[test]
    fn list_then_install_succeeds() {
        let mut svc = MarketplaceService::new(test_store());
        let listed = svc.list_plugin("github".into(), DescriptorSource::BundledCatalog);
        assert!(matches!(listed, MarketplaceEvent::PluginListed { .. }));

        let events = svc
            .handle_command(MarketplaceCommand::InstallPlugin {
                plugin_id: "github".into(),
                initiated_by: operator(),
            })
            .unwrap();

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            MarketplaceEvent::PluginInstallationStarted { .. }
        ));
        assert!(matches!(
            &events[1],
            MarketplaceEvent::PluginInstalled { .. }
        ));

        let record = svc.get_record("github").unwrap();
        assert_eq!(record.state, MarketplaceState::Installed);
    }

    #[test]
    fn install_unlisted_plugin_fails() {
        let mut svc = MarketplaceService::<()>::new(test_store());
        let result = svc.handle_command(MarketplaceCommand::InstallPlugin {
            plugin_id: "unknown".into(),
            initiated_by: operator(),
        });
        assert!(matches!(result, Err(MarketplaceError::PluginNotFound(_))));
    }

    #[test]
    fn enable_requires_installed() {
        let mut svc = MarketplaceService::new(test_store());
        svc.list_plugin("github".into(), DescriptorSource::BundledCatalog);

        let result = svc.handle_command(MarketplaceCommand::EnablePluginForProject {
            plugin_id: "github".into(),
            project: project_p1(),
            tool_allowlist: None,
            signal_allowlist: None,
            signal_capture_override: None,
            enabled_by: operator(),
        });
        assert!(matches!(
            result,
            Err(MarketplaceError::PluginNotInstalled(_))
        ));
    }

    #[test]
    fn enable_disable_lifecycle() {
        let mut svc = MarketplaceService::new(test_store());
        svc.list_plugin("github".into(), DescriptorSource::BundledCatalog);
        svc.handle_command(MarketplaceCommand::InstallPlugin {
            plugin_id: "github".into(),
            initiated_by: operator(),
        })
        .unwrap();

        // Enable for project
        let events = svc
            .handle_command(MarketplaceCommand::EnablePluginForProject {
                plugin_id: "github".into(),
                project: project_p1(),
                tool_allowlist: Some(vec!["github.get_issue".into()]),
                signal_allowlist: Some(vec!["github.issue.opened".into()]),
                signal_capture_override: Some(SignalCaptureOverride {
                    graph_project: Some(false),
                    memory_ingest: None,
                }),
                enabled_by: operator(),
            })
            .unwrap();

        assert_eq!(events.len(), 1);
        if let MarketplaceEvent::PluginEnabledForProject {
            tool_allowlist,
            signal_allowlist,
            signal_capture_override,
            ..
        } = &events[0]
        {
            assert_eq!(
                tool_allowlist.as_ref().unwrap(),
                &vec!["github.get_issue".to_string()]
            );
            assert_eq!(
                signal_allowlist.as_ref().unwrap(),
                &vec!["github.issue.opened".to_string()]
            );
            assert_eq!(
                signal_capture_override.as_ref().unwrap().graph_project,
                Some(false)
            );
        } else {
            panic!("expected PluginEnabledForProject");
        }

        // Check enablement query
        let enablement = svc.get_enablement("github", &project_p1()).unwrap();
        assert!(enablement.enabled);

        // Disable
        svc.handle_command(MarketplaceCommand::DisablePluginForProject {
            plugin_id: "github".into(),
            project: project_p1(),
            disabled_by: operator(),
        })
        .unwrap();

        let enablement = svc.get_enablement("github", &project_p1()).unwrap();
        assert!(!enablement.enabled);
    }

    #[test]
    fn uninstall_removes_all_enablements() {
        let mut svc = MarketplaceService::new(test_store());
        svc.list_plugin("github".into(), DescriptorSource::BundledCatalog);
        svc.handle_command(MarketplaceCommand::InstallPlugin {
            plugin_id: "github".into(),
            initiated_by: operator(),
        })
        .unwrap();

        // Enable for two projects
        let p1 = project_p1();
        let p2 = ProjectKey::new("t1", "w1", "p2");

        for p in [&p1, &p2] {
            svc.handle_command(MarketplaceCommand::EnablePluginForProject {
                plugin_id: "github".into(),
                project: p.clone(),
                tool_allowlist: None,
                signal_allowlist: None,
                signal_capture_override: None,
                enabled_by: operator(),
            })
            .unwrap();
        }

        assert_eq!(svc.enablements_for_project(&p1).len(), 1);
        assert_eq!(svc.enablements_for_project(&p2).len(), 1);

        // Uninstall
        svc.handle_command(MarketplaceCommand::UninstallPlugin {
            plugin_id: "github".into(),
            uninstalled_by: operator(),
        })
        .unwrap();

        assert_eq!(svc.enablements_for_project(&p1).len(), 0);
        assert_eq!(svc.enablements_for_project(&p2).len(), 0);
        assert_eq!(
            svc.get_record("github").unwrap().state,
            MarketplaceState::Uninstalled
        );
    }

    #[test]
    fn per_project_isolation() {
        let mut svc = MarketplaceService::new(test_store());
        svc.list_plugin("github".into(), DescriptorSource::BundledCatalog);
        svc.handle_command(MarketplaceCommand::InstallPlugin {
            plugin_id: "github".into(),
            initiated_by: operator(),
        })
        .unwrap();

        let p1 = project_p1();
        let p2 = ProjectKey::new("t1", "w1", "p2");

        // Enable only for p1
        svc.handle_command(MarketplaceCommand::EnablePluginForProject {
            plugin_id: "github".into(),
            project: p1.clone(),
            tool_allowlist: None,
            signal_allowlist: None,
            signal_capture_override: None,
            enabled_by: operator(),
        })
        .unwrap();

        // p1 sees enablement, p2 does not
        assert_eq!(svc.enablements_for_project(&p1).len(), 1);
        assert_eq!(svc.enablements_for_project(&p2).len(), 0);
    }

    #[test]
    fn verify_credentials_is_ephemeral() {
        let mut svc = MarketplaceService::new(test_store());
        svc.list_plugin("github".into(), DescriptorSource::BundledCatalog);
        svc.handle_command(MarketplaceCommand::InstallPlugin {
            plugin_id: "github".into(),
            initiated_by: operator(),
        })
        .unwrap();

        let events = svc
            .handle_command(MarketplaceCommand::VerifyPluginCredentials {
                plugin_id: "github".into(),
                credential_scope_key: None,
                verified_by: operator(),
            })
            .unwrap();

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            MarketplaceEvent::PluginCredentialsVerified {
                outcome: VerificationOutcome::Ok,
                ..
            }
        ));

        // State unchanged — still Installed, no Connected
        assert_eq!(
            svc.get_record("github").unwrap().state,
            MarketplaceState::Installed
        );
    }
}
