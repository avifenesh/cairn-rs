//! Shared visibility and access context types for Phase 2 plugin/sandbox/trigger work.
//!
//! These types are declared in RFCs 015, 016, 017, and 022 and live in `cairn-domain`
//! so that downstream crates (`cairn-tools`, `cairn-workspace`, `cairn-runtime`) can
//! import exactly the projection they need without pulling in plugin internals.

use crate::ids::RunId;
use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ── Per-Run Tool Visibility (RFC 015 §"Per-Run Tool Visibility") ─────────

/// Context carried into prompt building and tool-search filtering so that a
/// run only sees tools from plugins enabled for its project.
///
/// Constructed by the runtime when a run starts; passed to
/// `BuiltinToolRegistry::prompt_tools_for` and the deferred-tier `tool_search`
/// filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityContext {
    /// The project this run belongs to.
    pub project: ProjectKey,
    /// The run that is requesting tool visibility (None during project-level queries).
    pub run_id: Option<RunId>,
    /// Plugin IDs enabled for this project (from `PluginEnablement` projections).
    pub enabled_plugins: HashSet<String>,
    /// Per-plugin tool allowlist.
    /// Key = plugin_id.  Value = `Some(tool_names)` if the project restricts
    /// which tools are visible, `None` if all of the plugin's tools are allowed.
    pub allowlisted_tools: HashMap<String, Option<HashSet<String>>>,
}

// ── Repo Access (RFC 016 §"Access Layer") ────────────────────────────────

/// Minimal context for repo-access checks in `cairn-workspace`.
///
/// `cairn-workspace` imports only this type — never `VisibilityContext` — so
/// plugin/tool concerns stay out of the workspace crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoAccessContext {
    pub project: ProjectKey,
}

/// Thin projection: callers that already hold a `VisibilityContext` can
/// cheaply obtain a `RepoAccessContext` without importing workspace internals.
impl From<&VisibilityContext> for RepoAccessContext {
    fn from(vc: &VisibilityContext) -> Self {
        Self {
            project: vc.project.clone(),
        }
    }
}

// ── Signal Capture Override (RFC 015 §"Per-Project Enable State") ─────────

/// Per-project override of a plugin's declared knowledge-capture behaviour.
///
/// `None` on a field means "inherit the `SignalSource` capability default"
/// (graph projection defaults to `true`, memory ingest defaults to `false`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalCaptureOverride {
    /// Override automatic graph projection of received signals.
    pub graph_project: Option<bool>,
    /// Override memory ingestion of signal payloads.
    pub memory_ingest: Option<bool>,
}

// ── Plugin Category (RFC 015 §"Canonical Model") ─────────────────────────

/// Marketplace filter category for plugin descriptors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCategory {
    IssueTracker,
    ChatOps,
    Calendar,
    Files,
    CustomerSupport,
    Observability,
    DataSource,
    CommunicationChannel,
    /// Reserved for forward-compat; not surfaced as a marketplace filter in v1.
    EvalScorer,
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenancy::ProjectKey;

    #[test]
    fn visibility_context_round_trips_through_serde() {
        let mut enabled = HashSet::new();
        enabled.insert("github".to_string());

        let mut allowed = HashMap::new();
        let mut tools = HashSet::new();
        tools.insert("github.get_issue".to_string());
        allowed.insert("github".to_string(), Some(tools));

        let ctx = VisibilityContext {
            project: ProjectKey::new("t1", "w1", "p1"),
            run_id: Some(RunId::new("run-1")),
            enabled_plugins: enabled,
            allowlisted_tools: allowed,
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let back: VisibilityContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, back);
    }

    #[test]
    fn repo_access_context_from_visibility_context() {
        let ctx = VisibilityContext {
            project: ProjectKey::new("t1", "w1", "p1"),
            run_id: Some(RunId::new("run-1")),
            enabled_plugins: HashSet::new(),
            allowlisted_tools: HashMap::new(),
        };

        let access: RepoAccessContext = RepoAccessContext::from(&ctx);
        assert_eq!(access.project, ctx.project);
    }

    #[test]
    fn signal_capture_override_defaults_to_none() {
        let sco = SignalCaptureOverride::default();
        assert_eq!(sco.graph_project, None);
        assert_eq!(sco.memory_ingest, None);
    }

    #[test]
    fn plugin_category_serde_round_trip() {
        let cases = vec![
            PluginCategory::IssueTracker,
            PluginCategory::ChatOps,
            PluginCategory::Calendar,
            PluginCategory::Files,
            PluginCategory::CustomerSupport,
            PluginCategory::Observability,
            PluginCategory::DataSource,
            PluginCategory::CommunicationChannel,
            PluginCategory::EvalScorer,
            PluginCategory::Other("custom_plugin".to_string()),
        ];

        for cat in &cases {
            let json = serde_json::to_string(cat).unwrap();
            let back: PluginCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, cat);
        }
    }
}
