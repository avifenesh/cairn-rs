//! Local-filesystem integration plugin for Cairn.
//!
//! Treats a directory on disk as a pseudo-repo for issue-sync and
//! PR-proposal flows. No webhook, no network auth — the operator hands
//! over a path, and Cairn reads the directory as if it were a git host.
//!
//! This is the minimum surface needed to dogfood the UI's multi-host
//! selector against something that works without a real external
//! service. Writes happen via the regular file/git tools, not through
//! this plugin.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    EventAction, EventActionMapping, Integration, IntegrationError, IntegrationEvent, QueueStats,
    WorkItem,
};

/// Local-filesystem integration configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalFsConfig {
    /// Absolute path to the directory that should be exposed as a
    /// pseudo-repo. Must exist at configuration time.
    pub path: String,
    /// Optional human-readable label shown in the UI. Defaults to the
    /// basename of `path`.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// A minimal read-only integration backed by a local directory.
pub struct LocalFsPlugin {
    id: String,
    display_name: String,
    path: String,
}

impl LocalFsPlugin {
    pub fn new(id: &str, config: LocalFsConfig) -> Result<Self, IntegrationError> {
        if config.path.trim().is_empty() {
            return Err(IntegrationError::ConfigInvalid(
                "local_fs.path must not be empty".into(),
            ));
        }
        let p = std::path::Path::new(&config.path);
        if !p.exists() {
            return Err(IntegrationError::ConfigInvalid(format!(
                "local_fs.path does not exist: {}",
                config.path
            )));
        }
        if !p.is_dir() {
            return Err(IntegrationError::ConfigInvalid(format!(
                "local_fs.path is not a directory: {}",
                config.path
            )));
        }
        let display_name = config.display_name.unwrap_or_else(|| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| config.path.clone())
        });
        Ok(Self {
            id: id.to_owned(),
            display_name,
            path: config.path,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

#[async_trait]
impl Integration for LocalFsPlugin {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn is_configured(&self) -> bool {
        std::path::Path::new(&self.path).is_dir()
    }

    fn default_agent_prompt(&self) -> &str {
        "You are an autonomous agent working on a task rooted in a local \
         directory. Use file and shell tools to read, modify, and verify \
         changes. There is no external issue tracker — the operator \
         describes the task directly."
    }

    fn default_event_actions(&self) -> Vec<EventActionMapping> {
        // local_fs has no webhook surface; event mappings are inert but
        // non-empty so overrides can still attach.
        vec![EventActionMapping {
            event_pattern: "*".into(),
            label_filter: None,
            repo_filter: None,
            action: EventAction::Ignore,
        }]
    }

    async fn verify_webhook(
        &self,
        _headers: &http::HeaderMap,
        _body: &[u8],
    ) -> Result<(), IntegrationError> {
        // No webhook surface for a local directory.
        Err(IntegrationError::VerificationFailed(
            "local_fs integration does not accept webhooks".into(),
        ))
    }

    async fn parse_event(
        &self,
        _headers: &http::HeaderMap,
        _body: &[u8],
    ) -> Result<IntegrationEvent, IntegrationError> {
        Err(IntegrationError::ParseError(
            "local_fs integration has no event stream".into(),
        ))
    }

    async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError> {
        Ok(format!(
            "Work on task `{}` rooted at local path `{}`.\n\nDescription:\n{}",
            item.title, self.path, item.body,
        ))
    }

    async fn prepare_tool_registry(
        &self,
        base: &cairn_tools::BuiltinToolRegistry,
        _item: &WorkItem,
    ) -> Arc<cairn_tools::BuiltinToolRegistry> {
        Arc::new(cairn_tools::BuiltinToolRegistry::from_existing(base))
    }

    fn auth_exempt_paths(&self) -> Vec<String> {
        Vec::new()
    }

    async fn queue_stats(&self) -> QueueStats {
        QueueStats::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_path() {
        let res = LocalFsPlugin::new(
            "lfs",
            LocalFsConfig {
                path: "/nonexistent/path/that/really/does/not/exist".into(),
                display_name: None,
            },
        );
        assert!(matches!(res, Err(IntegrationError::ConfigInvalid(_))));
    }

    #[test]
    fn rejects_empty_path() {
        let res = LocalFsPlugin::new(
            "lfs",
            LocalFsConfig {
                path: "   ".into(),
                display_name: None,
            },
        );
        assert!(matches!(res, Err(IntegrationError::ConfigInvalid(_))));
    }

    #[test]
    fn accepts_existing_directory() {
        let dir = std::env::temp_dir();
        let plugin = LocalFsPlugin::new(
            "lfs",
            LocalFsConfig {
                path: dir.to_string_lossy().into_owned(),
                display_name: Some("Temp".into()),
            },
        )
        .expect("temp dir should be a valid local_fs path");
        assert_eq!(plugin.display_name(), "Temp");
        assert!(plugin.is_configured());
    }

    #[test]
    fn default_display_name_uses_basename() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin = LocalFsPlugin::new(
            "lfs",
            LocalFsConfig {
                path: tmp.path().to_string_lossy().into_owned(),
                display_name: None,
            },
        )
        .expect("tempdir is a valid local_fs path");
        let basename = tmp
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap()
            .to_owned();
        assert_eq!(plugin.display_name(), basename);
    }
}
