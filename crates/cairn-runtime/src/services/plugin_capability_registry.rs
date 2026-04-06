//! Per-plugin capability discovery and registry for RFC 007.
//!
//! When a plugin registers via `initialize`, the host enumerates its
//! declared capabilities (tools, hooks, event subscriptions, etc.) and
//! stores them here. Callers can then query which plugins provide a given
//! capability family or specific tool name.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use cairn_plugin_proto::capabilities::CapabilityFamily;
use cairn_plugin_proto::wire::ToolDescriptorWire;
use cairn_plugin_proto::manifest::CapabilityWire;

/// Discovered capabilities for one plugin.
#[derive(Clone, Debug)]
pub struct PluginCapabilities {
    pub plugin_id: String,
    /// Capability families this plugin declared.
    pub families: HashSet<CapabilityFamily>,
    /// Tool descriptors (from `tools.list` or manifest).
    pub tools: Vec<ToolDescriptorWire>,
    /// Event type names this plugin wants to receive.
    pub event_subscriptions: HashSet<String>,
    /// Signal source names this plugin can poll.
    pub signal_sources: Vec<String>,
    /// Channel names this plugin can deliver to.
    pub channels: Vec<String>,
}

impl PluginCapabilities {
    pub fn new(plugin_id: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            families: HashSet::new(),
            tools: Vec::new(),
            event_subscriptions: HashSet::new(),
            signal_sources: Vec::new(),
            channels: Vec::new(),
        }
    }
}

/// Central registry of capabilities across all registered plugins.
///
/// Thread-safe — designed to be shared via `Arc<CapabilityRegistry>`.
pub struct CapabilityRegistry {
    plugins: RwLock<HashMap<String, PluginCapabilities>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self {
            plugins: RwLock::new(HashMap::new()),
        }
    }

    /// Register capabilities from a plugin's manifest declarations.
    ///
    /// Parses `CapabilityWire` entries from the manifest and populates
    /// the registry. Tool descriptors are added separately via
    /// [`register_tools`] after a `tools.list` call.
    pub fn register_from_manifest(&self, plugin_id: &str, capabilities: &[CapabilityWire]) {
        let mut map = self.plugins.write().unwrap();
        let entry = map
            .entry(plugin_id.to_owned())
            .or_insert_with(|| PluginCapabilities::new(plugin_id));

        for cap in capabilities {
            if let Some(family) = parse_family(&cap.capability_type) {
                entry.families.insert(family);
            }
            if let Some(ref signals) = cap.signals {
                entry.signal_sources.extend(signals.iter().cloned());
            }
            if let Some(ref channels) = cap.channels {
                entry.channels.extend(channels.iter().cloned());
            }
        }
    }

    /// Register tool descriptors discovered via `tools.list`.
    pub fn register_tools(&self, plugin_id: &str, tools: Vec<ToolDescriptorWire>) {
        let mut map = self.plugins.write().unwrap();
        let entry = map
            .entry(plugin_id.to_owned())
            .or_insert_with(|| PluginCapabilities::new(plugin_id));
        entry.families.insert(CapabilityFamily::ToolProvider);
        entry.tools = tools;
    }

    /// Register event type subscriptions for a plugin.
    pub fn register_event_subscriptions(&self, plugin_id: &str, event_types: Vec<String>) {
        let mut map = self.plugins.write().unwrap();
        let entry = map
            .entry(plugin_id.to_owned())
            .or_insert_with(|| PluginCapabilities::new(plugin_id));
        entry.event_subscriptions.extend(event_types);
    }

    /// Remove all capabilities for a plugin (on uninstall/stop).
    pub fn unregister(&self, plugin_id: &str) {
        self.plugins.write().unwrap().remove(plugin_id);
    }

    /// Get capabilities for a specific plugin.
    pub fn get(&self, plugin_id: &str) -> Option<PluginCapabilities> {
        self.plugins.read().unwrap().get(plugin_id).cloned()
    }

    /// Find all plugins providing a given capability family.
    pub fn plugins_with_family(&self, family: CapabilityFamily) -> Vec<String> {
        self.plugins
            .read()
            .unwrap()
            .iter()
            .filter(|(_, caps)| caps.families.contains(&family))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Find which plugin provides a tool by name.
    ///
    /// Returns the first match (tool names should be globally unique per RFC 007).
    pub fn find_tool_provider(&self, tool_name: &str) -> Option<String> {
        self.plugins
            .read()
            .unwrap()
            .iter()
            .find(|(_, caps)| caps.tools.iter().any(|t| t.name == tool_name))
            .map(|(id, _)| id.clone())
    }

    /// Find all plugins subscribed to a given event type name.
    pub fn plugins_subscribed_to(&self, event_type: &str) -> Vec<String> {
        self.plugins
            .read()
            .unwrap()
            .iter()
            .filter(|(_, caps)| caps.event_subscriptions.contains(event_type))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// List all registered plugin IDs.
    pub fn plugin_ids(&self) -> Vec<String> {
        self.plugins.read().unwrap().keys().cloned().collect()
    }

    /// Total number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.read().unwrap().is_empty()
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a capability type string into a `CapabilityFamily`.
fn parse_family(s: &str) -> Option<CapabilityFamily> {
    match s {
        "tool_provider" => Some(CapabilityFamily::ToolProvider),
        "signal_source" => Some(CapabilityFamily::SignalSource),
        "channel_provider" => Some(CapabilityFamily::ChannelProvider),
        "post_turn_hook" => Some(CapabilityFamily::PostTurnHook),
        "policy_hook" => Some(CapabilityFamily::PolicyHook),
        "eval_scorer" => Some(CapabilityFamily::EvalScorer),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolDescriptorWire {
        ToolDescriptorWire {
            name: name.to_owned(),
            description: format!("{name} tool"),
            input_schema: None,
            permissions: vec![],
        }
    }

    fn cap_wire(cap_type: &str) -> CapabilityWire {
        CapabilityWire {
            capability_type: cap_type.to_owned(),
            tools: None,
            signals: None,
            channels: None,
        }
    }

    #[test]
    fn register_from_manifest_stores_families() {
        let reg = CapabilityRegistry::new();
        reg.register_from_manifest(
            "com.example.git",
            &[cap_wire("tool_provider"), cap_wire("post_turn_hook")],
        );

        let caps = reg.get("com.example.git").unwrap();
        assert!(caps.families.contains(&CapabilityFamily::ToolProvider));
        assert!(caps.families.contains(&CapabilityFamily::PostTurnHook));
        assert!(!caps.families.contains(&CapabilityFamily::EvalScorer));
    }

    #[test]
    fn register_tools_populates_tool_list() {
        let reg = CapabilityRegistry::new();
        reg.register_tools("p1", vec![tool("git.status"), tool("git.diff")]);

        let caps = reg.get("p1").unwrap();
        assert_eq!(caps.tools.len(), 2);
        assert!(caps.families.contains(&CapabilityFamily::ToolProvider));
    }

    #[test]
    fn find_tool_provider_returns_correct_plugin() {
        let reg = CapabilityRegistry::new();
        reg.register_tools("git-plugin", vec![tool("git.status")]);
        reg.register_tools("slack-plugin", vec![tool("slack.send")]);

        assert_eq!(
            reg.find_tool_provider("git.status"),
            Some("git-plugin".to_owned())
        );
        assert_eq!(
            reg.find_tool_provider("slack.send"),
            Some("slack-plugin".to_owned())
        );
        assert!(reg.find_tool_provider("unknown.tool").is_none());
    }

    #[test]
    fn plugins_with_family_filters_correctly() {
        let reg = CapabilityRegistry::new();
        reg.register_from_manifest("p1", &[cap_wire("tool_provider")]);
        reg.register_from_manifest("p2", &[cap_wire("eval_scorer")]);
        reg.register_from_manifest("p3", &[cap_wire("tool_provider"), cap_wire("eval_scorer")]);

        let tool_providers = reg.plugins_with_family(CapabilityFamily::ToolProvider);
        assert_eq!(tool_providers.len(), 2);
        assert!(tool_providers.contains(&"p1".to_owned()));
        assert!(tool_providers.contains(&"p3".to_owned()));
    }

    #[test]
    fn event_subscriptions_work() {
        let reg = CapabilityRegistry::new();
        reg.register_event_subscriptions(
            "audit-plugin",
            vec!["session_created".into(), "run_created".into()],
        );
        reg.register_event_subscriptions("metrics-plugin", vec!["run_created".into()]);

        let subscribers = reg.plugins_subscribed_to("run_created");
        assert_eq!(subscribers.len(), 2);

        let subscribers = reg.plugins_subscribed_to("session_created");
        assert_eq!(subscribers.len(), 1);
        assert_eq!(subscribers[0], "audit-plugin");

        assert!(reg.plugins_subscribed_to("unknown_event").is_empty());
    }

    #[test]
    fn manifest_with_signals_and_channels() {
        let reg = CapabilityRegistry::new();
        let cap = CapabilityWire {
            capability_type: "signal_source".to_owned(),
            tools: None,
            signals: Some(vec!["github.webhook".into(), "pagerduty.alert".into()]),
            channels: None,
        };
        let chan = CapabilityWire {
            capability_type: "channel_provider".to_owned(),
            tools: None,
            signals: None,
            channels: Some(vec!["slack".into()]),
        };
        reg.register_from_manifest("integrations", &[cap, chan]);

        let caps = reg.get("integrations").unwrap();
        assert_eq!(caps.signal_sources, vec!["github.webhook", "pagerduty.alert"]);
        assert_eq!(caps.channels, vec!["slack"]);
        assert!(caps.families.contains(&CapabilityFamily::SignalSource));
        assert!(caps.families.contains(&CapabilityFamily::ChannelProvider));
    }

    #[test]
    fn unregister_removes_all_state() {
        let reg = CapabilityRegistry::new();
        reg.register_tools("p1", vec![tool("t1")]);
        assert_eq!(reg.len(), 1);
        reg.unregister("p1");
        assert_eq!(reg.len(), 0);
        assert!(reg.get("p1").is_none());
    }

    #[test]
    fn unknown_capability_type_ignored() {
        let reg = CapabilityRegistry::new();
        reg.register_from_manifest("p1", &[cap_wire("future_capability_v99")]);
        let caps = reg.get("p1").unwrap();
        assert!(caps.families.is_empty());
    }
}
