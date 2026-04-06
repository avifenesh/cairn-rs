//! Per-plugin heartbeat tracking for RFC 007 lifecycle management.
//!
//! Each plugin is expected to send periodic heartbeats. If no heartbeat
//! arrives within [`HEARTBEAT_TIMEOUT_MS`] (30 seconds), the plugin is
//! considered unhealthy and eligible for restart.

use std::collections::HashMap;
use std::sync::RwLock;

/// Default heartbeat timeout: 30 seconds.
pub const HEARTBEAT_TIMEOUT_MS: u64 = 30_000;

/// Per-plugin health snapshot.
#[derive(Clone, Debug)]
pub struct PluginHealthState {
    pub last_heartbeat_ms: u64,
    pub consecutive_missed: u32,
    pub last_error: Option<String>,
}

/// In-memory heartbeat tracker, keyed by plugin ID string.
///
/// Thread-safe — designed to be shared via `Arc<PluginHealthMonitor>`.
pub struct PluginHealthMonitor {
    timeout_ms: u64,
    state: RwLock<HashMap<String, PluginHealthState>>,
}

impl PluginHealthMonitor {
    pub fn new() -> Self {
        Self {
            timeout_ms: HEARTBEAT_TIMEOUT_MS,
            state: RwLock::new(HashMap::new()),
        }
    }

    /// Create with a custom timeout (useful for tests).
    pub fn with_timeout_ms(timeout_ms: u64) -> Self {
        Self {
            timeout_ms,
            state: RwLock::new(HashMap::new()),
        }
    }

    /// Record a heartbeat for a plugin. Resets the missed counter.
    pub fn record_heartbeat(&self, plugin_id: &str) {
        let mut map = self.state.write().unwrap();
        let entry = map.entry(plugin_id.to_owned()).or_insert_with(|| {
            PluginHealthState {
                last_heartbeat_ms: 0,
                consecutive_missed: 0,
                last_error: None,
            }
        });
        entry.last_heartbeat_ms = now_ms();
        entry.consecutive_missed = 0;
    }

    /// Record that a health check was missed (no heartbeat within window).
    pub fn record_missed(&self, plugin_id: &str) {
        let mut map = self.state.write().unwrap();
        if let Some(entry) = map.get_mut(plugin_id) {
            entry.consecutive_missed += 1;
        }
    }

    /// Record an error from a plugin (crash, protocol violation, etc.).
    pub fn record_error(&self, plugin_id: &str, error: String) {
        let mut map = self.state.write().unwrap();
        let entry = map.entry(plugin_id.to_owned()).or_insert_with(|| {
            PluginHealthState {
                last_heartbeat_ms: 0,
                consecutive_missed: 0,
                last_error: None,
            }
        });
        entry.last_error = Some(error);
    }

    /// Check whether a plugin is healthy (heartbeat received within timeout).
    ///
    /// An unknown plugin is considered unhealthy (not yet registered).
    pub fn is_healthy(&self, plugin_id: &str) -> bool {
        let map = self.state.read().unwrap();
        match map.get(plugin_id) {
            None => false,
            Some(state) => {
                let elapsed = now_ms().saturating_sub(state.last_heartbeat_ms);
                elapsed < self.timeout_ms
            }
        }
    }

    /// Return IDs of all plugins whose last heartbeat is older than the timeout.
    pub fn unhealthy_plugins(&self) -> Vec<String> {
        let map = self.state.read().unwrap();
        let now = now_ms();
        map.iter()
            .filter(|(_, state)| now.saturating_sub(state.last_heartbeat_ms) >= self.timeout_ms)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Snapshot health state for a specific plugin.
    pub fn get(&self, plugin_id: &str) -> Option<PluginHealthState> {
        self.state.read().unwrap().get(plugin_id).cloned()
    }

    /// Remove tracking state for a plugin (e.g. after uninstall).
    pub fn remove(&self, plugin_id: &str) {
        self.state.write().unwrap().remove(plugin_id);
    }

    /// Number of tracked plugins.
    pub fn tracked_count(&self) -> usize {
        self.state.read().unwrap().len()
    }
}

impl Default for PluginHealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_plugin_is_unhealthy_until_first_heartbeat() {
        let monitor = PluginHealthMonitor::new();
        assert!(!monitor.is_healthy("unknown-plugin"));
    }

    #[test]
    fn heartbeat_makes_plugin_healthy() {
        let monitor = PluginHealthMonitor::new();
        monitor.record_heartbeat("p1");
        assert!(monitor.is_healthy("p1"));
    }

    #[test]
    fn stale_heartbeat_marks_unhealthy() {
        // Use a 1ms timeout so it expires immediately.
        let monitor = PluginHealthMonitor::with_timeout_ms(1);
        monitor.record_heartbeat("p1");
        // Sleep just enough for the timeout to elapse.
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(!monitor.is_healthy("p1"));
    }

    #[test]
    fn unhealthy_plugins_returns_stale_ids() {
        let monitor = PluginHealthMonitor::with_timeout_ms(1);
        monitor.record_heartbeat("p1");
        monitor.record_heartbeat("p2");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let unhealthy = monitor.unhealthy_plugins();
        assert!(unhealthy.contains(&"p1".to_owned()));
        assert!(unhealthy.contains(&"p2".to_owned()));
    }

    #[test]
    fn fresh_heartbeat_not_in_unhealthy_list() {
        let monitor = PluginHealthMonitor::with_timeout_ms(60_000);
        monitor.record_heartbeat("p1");
        assert!(monitor.unhealthy_plugins().is_empty());
    }

    #[test]
    fn record_error_stores_message() {
        let monitor = PluginHealthMonitor::new();
        monitor.record_heartbeat("p1");
        monitor.record_error("p1", "segfault".into());
        let state = monitor.get("p1").unwrap();
        assert_eq!(state.last_error.as_deref(), Some("segfault"));
    }

    #[test]
    fn record_missed_increments_counter() {
        let monitor = PluginHealthMonitor::new();
        monitor.record_heartbeat("p1");
        monitor.record_missed("p1");
        monitor.record_missed("p1");
        assert_eq!(monitor.get("p1").unwrap().consecutive_missed, 2);
    }

    #[test]
    fn heartbeat_resets_missed_counter() {
        let monitor = PluginHealthMonitor::new();
        monitor.record_heartbeat("p1");
        monitor.record_missed("p1");
        monitor.record_missed("p1");
        monitor.record_heartbeat("p1");
        assert_eq!(monitor.get("p1").unwrap().consecutive_missed, 0);
    }

    #[test]
    fn remove_clears_state() {
        let monitor = PluginHealthMonitor::new();
        monitor.record_heartbeat("p1");
        assert_eq!(monitor.tracked_count(), 1);
        monitor.remove("p1");
        assert_eq!(monitor.tracked_count(), 0);
        assert!(monitor.get("p1").is_none());
    }
}
