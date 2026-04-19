//! Event subscription routing for RFC 007 plugins.
//!
//! Plugins subscribe to specific `RuntimeEvent` type names (e.g.
//! `"session_created"`, `"run_state_changed"`). When a matching event
//! is emitted, the router forwards it to all subscribed plugins as a
//! JSON-RPC `event.forward` notification.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use cairn_domain::events::EventEnvelope;
use cairn_domain::RuntimeEvent;

use super::plugin_capability_registry::CapabilityRegistry;
use super::plugin_host::PluginHost;

/// Notification method used when forwarding events to plugins.
pub const EVENT_FORWARD_METHOD: &str = "event.forward";

/// Routes runtime events to subscribed plugins.
///
/// Backed by the capability registry for subscription lookup and the
/// plugin host for delivery.
pub struct PluginEventRouter {
    /// Capability registry to look up event subscriptions.
    capabilities: Arc<CapabilityRegistry>,
    /// Additional explicit subscriptions (beyond capability registry).
    subscriptions: RwLock<HashMap<String, HashSet<String>>>,
}

impl PluginEventRouter {
    pub fn new(capabilities: Arc<CapabilityRegistry>) -> Self {
        Self {
            capabilities,
            subscriptions: RwLock::new(HashMap::new()),
        }
    }

    /// Add an explicit event subscription for a plugin.
    ///
    /// This is in addition to subscriptions registered via the capability
    /// registry. Useful for dynamic subscription changes.
    pub fn subscribe(&self, plugin_id: &str, event_type: &str) {
        let mut subs = self
            .subscriptions
            .write()
            .unwrap_or_else(|e| e.into_inner());
        subs.entry(event_type.to_owned())
            .or_default()
            .insert(plugin_id.to_owned());
    }

    /// Remove an explicit subscription.
    pub fn unsubscribe(&self, plugin_id: &str, event_type: &str) {
        let mut subs = self
            .subscriptions
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(set) = subs.get_mut(event_type) {
            set.remove(plugin_id);
            if set.is_empty() {
                subs.remove(event_type);
            }
        }
    }

    /// Remove all subscriptions for a plugin (on stop/uninstall).
    pub fn unsubscribe_all(&self, plugin_id: &str) {
        let mut subs = self
            .subscriptions
            .write()
            .unwrap_or_else(|e| e.into_inner());
        subs.retain(|_, set| {
            set.remove(plugin_id);
            !set.is_empty()
        });
    }

    /// Find all plugins subscribed to a given event type.
    ///
    /// Merges subscriptions from both the capability registry and
    /// explicit dynamic subscriptions.
    pub fn subscribers_for(&self, event_type: &str) -> Vec<String> {
        let mut result: HashSet<String> = HashSet::new();

        // From capability registry.
        for id in self.capabilities.plugins_subscribed_to(event_type) {
            result.insert(id);
        }

        // From explicit subscriptions.
        let subs = self.subscriptions.read().unwrap_or_else(|e| e.into_inner());
        if let Some(set) = subs.get(event_type) {
            for id in set {
                result.insert(id.clone());
            }
        }

        result.into_iter().collect()
    }

    /// Dispatch a runtime event to all subscribed plugins.
    ///
    /// Serializes the event envelope as JSON and sends it as an
    /// `event.forward` notification to each subscriber via the plugin host.
    /// Delivery failures are collected but don't stop other deliveries.
    pub fn dispatch(
        &self,
        event: &EventEnvelope<RuntimeEvent>,
        host: &PluginHost,
    ) -> Vec<DeliveryFailure> {
        let event_type = runtime_event_type_name(&event.payload);
        let subscribers = self.subscribers_for(&event_type);

        if subscribers.is_empty() {
            return Vec::new();
        }

        let payload = serde_json::json!({
            "eventType": event_type,
            "eventId": event.event_id.as_str(),
            "payload": serde_json::to_value(&event.payload).unwrap_or_default(),
        });

        let mut failures = Vec::new();
        for plugin_id in &subscribers {
            if let Err(e) = host.send_notification(plugin_id, EVENT_FORWARD_METHOD, payload.clone())
            {
                failures.push(DeliveryFailure {
                    plugin_id: plugin_id.clone(),
                    event_type: event_type.clone(),
                    error: e.to_string(),
                });
            }
        }

        failures
    }
}

/// Record of a failed event delivery attempt.
#[derive(Debug, Clone)]
pub struct DeliveryFailure {
    pub plugin_id: String,
    pub event_type: String,
    pub error: String,
}

/// Extract the snake_case event type name from a RuntimeEvent variant.
///
/// Uses serde's tag-based serialization to get the canonical name.
fn runtime_event_type_name(event: &RuntimeEvent) -> String {
    // RuntimeEvent is tagged with #[serde(tag = "event", rename_all = "snake_case")].
    // Serialize to JSON and extract the "event" field.
    if let Ok(val) = serde_json::to_value(event) {
        if let Some(name) = val.get("event").and_then(|v| v.as_str()) {
            return name.to_owned();
        }
    }
    // Fallback: use Debug formatting to extract variant name.
    let debug = format!("{event:?}");
    debug.split('(').next().unwrap_or("unknown").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::events::{EventSource, SessionCreated};
    use cairn_domain::{EventId, ProjectKey, SessionId};

    fn make_session_created_event() -> EventEnvelope<RuntimeEvent> {
        let project = ProjectKey::new("t1", "w1", "p1");
        EventEnvelope::for_runtime_event(
            EventId::new("evt_1"),
            EventSource::System,
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project.clone(),
                session_id: SessionId::new("s1"),
            }),
        )
    }

    #[test]
    fn runtime_event_type_name_extracts_correct_name() {
        let event = RuntimeEvent::SessionCreated(SessionCreated {
            project: ProjectKey::new("t", "w", "p"),
            session_id: SessionId::new("s"),
        });
        assert_eq!(runtime_event_type_name(&event), "session_created");
    }

    #[test]
    fn subscribe_and_lookup() {
        let caps = Arc::new(CapabilityRegistry::new());
        let router = PluginEventRouter::new(caps);

        router.subscribe("audit-plugin", "session_created");
        router.subscribe("metrics-plugin", "session_created");
        router.subscribe("metrics-plugin", "run_created");

        let subs = router.subscribers_for("session_created");
        assert_eq!(subs.len(), 2);

        let subs = router.subscribers_for("run_created");
        assert_eq!(subs.len(), 1);
        assert!(subs.contains(&"metrics-plugin".to_owned()));

        assert!(router.subscribers_for("unknown_event").is_empty());
    }

    #[test]
    fn unsubscribe_removes_single_subscription() {
        let caps = Arc::new(CapabilityRegistry::new());
        let router = PluginEventRouter::new(caps);

        router.subscribe("p1", "session_created");
        router.subscribe("p2", "session_created");
        router.unsubscribe("p1", "session_created");

        let subs = router.subscribers_for("session_created");
        assert_eq!(subs.len(), 1);
        assert!(subs.contains(&"p2".to_owned()));
    }

    #[test]
    fn unsubscribe_all_removes_plugin_from_all_events() {
        let caps = Arc::new(CapabilityRegistry::new());
        let router = PluginEventRouter::new(caps);

        router.subscribe("p1", "session_created");
        router.subscribe("p1", "run_created");
        router.subscribe("p2", "session_created");

        router.unsubscribe_all("p1");

        let subs = router.subscribers_for("session_created");
        assert_eq!(subs.len(), 1);
        assert!(subs.contains(&"p2".to_owned()));
        assert!(router.subscribers_for("run_created").is_empty());
    }

    #[test]
    fn merges_registry_and_explicit_subscriptions() {
        let caps = Arc::new(CapabilityRegistry::new());
        // Register via capability registry.
        caps.register_event_subscriptions("registry-plugin", vec!["session_created".into()]);

        let router = PluginEventRouter::new(caps);
        // Also add an explicit subscription.
        router.subscribe("explicit-plugin", "session_created");

        let subs = router.subscribers_for("session_created");
        assert_eq!(subs.len(), 2);
    }

    #[test]
    fn dispatch_with_no_subscribers_returns_empty() {
        let caps = Arc::new(CapabilityRegistry::new());
        let router = PluginEventRouter::new(caps);
        let host = PluginHost::default();

        let event = make_session_created_event();
        let failures = router.dispatch(&event, &host);
        assert!(failures.is_empty());
    }

    #[test]
    fn dispatch_to_missing_plugin_records_failure() {
        let caps = Arc::new(CapabilityRegistry::new());
        let router = PluginEventRouter::new(caps);
        router.subscribe("nonexistent-plugin", "session_created");

        let host = PluginHost::default();
        let event = make_session_created_event();
        let failures = router.dispatch(&event, &host);

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].plugin_id, "nonexistent-plugin");
        assert_eq!(failures[0].event_type, "session_created");
    }
}
