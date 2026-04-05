//! Plugin registry for managing installed plugin manifests.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::plugins::PluginManifest;

/// Plugin registry trait for managing installed plugins.
pub trait PluginRegistry: Send + Sync {
    fn register(&self, manifest: PluginManifest) -> Result<(), RegistryError>;
    fn unregister(&self, plugin_id: &str) -> Result<(), RegistryError>;
    fn get(&self, plugin_id: &str) -> Option<PluginManifest>;
    fn list_all(&self) -> Vec<PluginManifest>;
}

/// Registry-specific errors.
#[derive(Debug)]
pub enum RegistryError {
    AlreadyRegistered(String),
    NotFound(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::AlreadyRegistered(id) => write!(f, "plugin already registered: {id}"),
            RegistryError::NotFound(id) => write!(f, "plugin not found: {id}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// In-memory plugin registry backed by a HashMap.
pub struct InMemoryPluginRegistry {
    plugins: Mutex<HashMap<String, PluginManifest>>,
}

impl InMemoryPluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry for InMemoryPluginRegistry {
    fn register(&self, manifest: PluginManifest) -> Result<(), RegistryError> {
        let mut plugins = self.plugins.lock().unwrap();
        if plugins.contains_key(&manifest.id) {
            return Err(RegistryError::AlreadyRegistered(manifest.id.clone()));
        }
        plugins.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    fn unregister(&self, plugin_id: &str) -> Result<(), RegistryError> {
        let mut plugins = self.plugins.lock().unwrap();
        if plugins.remove(plugin_id).is_none() {
            return Err(RegistryError::NotFound(plugin_id.to_owned()));
        }
        Ok(())
    }

    fn get(&self, plugin_id: &str) -> Option<PluginManifest> {
        let plugins = self.plugins.lock().unwrap();
        plugins.get(plugin_id).cloned()
    }

    fn list_all(&self) -> Vec<PluginManifest> {
        let plugins = self.plugins.lock().unwrap();
        plugins.values().cloned().collect()
    }
}

impl InMemoryPluginRegistry {
    /// Returns metrics for a plugin, or `None` if not found.
    pub fn metrics(&self, plugin_id: &str) -> Option<crate::PluginMetrics> {
        self.plugins
            .lock()
            .unwrap()
            .contains_key(plugin_id)
            .then(|| crate::PluginMetrics {
                plugin_id: plugin_id.to_owned(),
                ..Default::default()
            })
    }

    /// Lists log entries for a plugin (stub — in-memory has no persistent log store).
    pub fn list_logs(
        &self,
        plugin_id: &str,
        _limit: usize,
    ) -> Result<Vec<crate::PluginLogEntry>, RegistryError> {
        if !self.plugins.lock().unwrap().contains_key(plugin_id) {
            return Err(RegistryError::NotFound(plugin_id.to_owned()));
        }
        Ok(vec![])
    }

    /// Lists pending signals for a plugin (stub).
    pub fn list_pending_signals(
        &self,
        plugin_id: &str,
        _limit: usize,
    ) -> Result<Vec<cairn_domain::SignalRecord>, RegistryError> {
        if !self.plugins.lock().unwrap().contains_key(plugin_id) {
            return Err(RegistryError::NotFound(plugin_id.to_owned()));
        }
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{DeclaredPermissions, Permission};
    use cairn_domain::ExecutionClass;

    fn test_manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: format!("{id} Plugin"),
            version: "0.1.0".to_owned(),
            command: vec!["test-binary".to_owned()],
            capabilities: vec![],
            permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
            limits: None,
            execution_class: ExecutionClass::SupervisedProcess,
            description: None,
            homepage: None,
        }
    }

    #[test]
    fn register_and_get() {
        let registry = InMemoryPluginRegistry::new();
        registry.register(test_manifest("com.example.foo")).unwrap();

        let manifest = registry.get("com.example.foo").unwrap();
        assert_eq!(manifest.id, "com.example.foo");
    }

    #[test]
    fn duplicate_register_fails() {
        let registry = InMemoryPluginRegistry::new();
        registry.register(test_manifest("com.example.foo")).unwrap();

        let result = registry.register(test_manifest("com.example.foo"));
        assert!(result.is_err());
    }

    #[test]
    fn unregister_removes_plugin() {
        let registry = InMemoryPluginRegistry::new();
        registry.register(test_manifest("com.example.foo")).unwrap();
        registry.unregister("com.example.foo").unwrap();

        assert!(registry.get("com.example.foo").is_none());
    }

    #[test]
    fn unregister_missing_fails() {
        let registry = InMemoryPluginRegistry::new();
        let result = registry.unregister("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn list_all_returns_registered() {
        let registry = InMemoryPluginRegistry::new();
        registry.register(test_manifest("com.example.a")).unwrap();
        registry.register(test_manifest("com.example.b")).unwrap();

        let all = registry.list_all();
        assert_eq!(all.len(), 2);
    }
}
