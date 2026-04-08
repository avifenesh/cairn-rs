//! Runtime configuration store (GAP-008).
//!
//! Flat key-value config persistence. Keys use dot-separated namespacing
//! (e.g. `agent.model`, `server.port`). Values are always strings; callers
//! parse to native types as needed.
//!
//! Two implementations:
//! - `InMemoryConfigStore` — HashMap-backed, not persistent across restarts.
//!   Used in tests and as a fallback when no config file is desired.
//! - `FileConfigStore` — TOML file at `~/.cairn/config.toml` (or any path).
//!   Reads are served from an in-memory cache; every write atomically flushes
//!   to disk so config survives restarts.
//!
//! # TOML layout
//! ```toml
//! [config]
//! "agent.model" = "claude-sonnet-4-6"
//! "server.port" = "3000"
//! "signal.poll_interval" = "300"
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ── Error ─────────────────────────────────────────────────────────────────

/// Error from `ConfigStore` operations.
#[derive(Debug)]
pub enum ConfigStoreError {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for ConfigStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigStoreError::Io(e) => write!(f, "config I/O error: {e}"),
            ConfigStoreError::Parse(msg) => write!(f, "config parse error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigStoreError {}

impl From<std::io::Error> for ConfigStoreError {
    fn from(e: std::io::Error) -> Self {
        ConfigStoreError::Io(e)
    }
}

// ── Trait ─────────────────────────────────────────────────────────────────

/// Flat key-value configuration store.
///
/// Keys use dot-separated namespacing (e.g. `agent.model`, `server.port`).
/// Values are always `String`; callers are responsible for type conversion.
pub trait ConfigStore: Send + Sync {
    /// Get a configuration value by key. Returns `None` if the key is not set.
    fn get(&self, key: &str) -> Option<String>;

    /// Set a configuration key to a string value. Persists on `FileConfigStore`.
    fn set(&self, key: &str, value: String) -> Result<(), ConfigStoreError>;

    /// Delete a configuration key.
    ///
    /// Returns `Ok(true)` if the key existed and was removed,
    /// `Ok(false)` if the key was not present.
    fn delete(&self, key: &str) -> Result<bool, ConfigStoreError>;

    /// List all `(key, value)` pairs whose key starts with `prefix`.
    ///
    /// Pass `""` as prefix to list all keys. Results are sorted by key.
    fn list_prefix(&self, prefix: &str) -> Vec<(String, String)>;

    /// Return all `(key, value)` pairs in the store, sorted by key.
    fn list_all(&self) -> Vec<(String, String)> {
        self.list_prefix("")
    }
}

// ── InMemoryConfigStore ───────────────────────────────────────────────────

/// In-memory config store backed by a `HashMap`.
///
/// Not persistent across process restarts. Useful in tests and for
/// deployments that derive all configuration from environment variables.
pub struct InMemoryConfigStore {
    entries: Mutex<HashMap<String, String>>,
}

impl InMemoryConfigStore {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Seed the store from a list of `(key, value)` pairs.
    pub fn with_entries(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        let store = Self::new();
        {
            let mut entries = store.entries.lock().unwrap();
            for (k, v) in pairs {
                entries.insert(k, v);
            }
        }
        store
    }
}

impl Default for InMemoryConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigStore for InMemoryConfigStore {
    fn get(&self, key: &str) -> Option<String> {
        self.entries.lock().unwrap().get(key).cloned()
    }

    fn set(&self, key: &str, value: String) -> Result<(), ConfigStoreError> {
        self.entries.lock().unwrap().insert(key.to_owned(), value);
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<bool, ConfigStoreError> {
        Ok(self.entries.lock().unwrap().remove(key).is_some())
    }

    fn list_prefix(&self, prefix: &str) -> Vec<(String, String)> {
        let entries = self.entries.lock().unwrap();
        let mut results: Vec<(String, String)> = entries
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}

// ── FileConfigStore ────────────────────────────────────────────────────────

/// TOML-backed config store persisted to disk.
///
/// All values are stored under a `[config]` TOML table as string fields.
/// Reads are served from an in-memory cache; every `set`/`delete` atomically
/// flushes the full table to disk.
///
/// # File location
/// Default: `~/.cairn/config.toml`.
/// Override with `FileConfigStore::open(path)`.
pub struct FileConfigStore {
    path: PathBuf,
    cache: Mutex<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Default)]
struct TomlConfigFile {
    #[serde(default)]
    config: HashMap<String, String>,
}

impl FileConfigStore {
    /// Open (or create) a config file at `path`.
    ///
    /// If the file does not exist it will be created on the first write.
    /// If the file exists but cannot be parsed, an error is returned.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ConfigStoreError> {
        let path = path.into();
        let cache = if path.exists() {
            let src = std::fs::read_to_string(&path)?;
            let tf: TomlConfigFile =
                toml::from_str(&src).map_err(|e| ConfigStoreError::Parse(e.to_string()))?;
            tf.config
        } else {
            HashMap::new()
        };
        Ok(Self {
            path,
            cache: Mutex::new(cache),
        })
    }

    /// Open the default config at `~/.cairn/config.toml`.
    ///
    /// Creates `~/.cairn/` if it does not exist.
    pub fn open_default() -> Result<Self, ConfigStoreError> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_owned());
        let dir = PathBuf::from(home).join(".cairn");
        std::fs::create_dir_all(&dir)?;
        Self::open(dir.join("config.toml"))
    }

    /// Return the file path this store is persisted to.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn flush(&self, cache: &HashMap<String, String>) -> Result<(), ConfigStoreError> {
        let tf = TomlConfigFile {
            config: cache.clone(),
        };
        let content = toml::to_string_pretty(&tf)
            .map_err(|e: toml::ser::Error| ConfigStoreError::Parse(e.to_string()))?;
        // Write atomically via a temp file so partial writes don't corrupt config.
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, &content)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

impl ConfigStore for FileConfigStore {
    fn get(&self, key: &str) -> Option<String> {
        self.cache.lock().unwrap().get(key).cloned()
    }

    fn set(&self, key: &str, value: String) -> Result<(), ConfigStoreError> {
        let mut cache = self.cache.lock().unwrap();
        cache.insert(key.to_owned(), value);
        self.flush(&cache)
    }

    fn delete(&self, key: &str) -> Result<bool, ConfigStoreError> {
        let mut cache = self.cache.lock().unwrap();
        let existed = cache.remove(key).is_some();
        if existed {
            self.flush(&cache)?;
        }
        Ok(existed)
    }

    fn list_prefix(&self, prefix: &str) -> Vec<(String, String)> {
        let cache = self.cache.lock().unwrap();
        let mut results: Vec<(String, String)> = cache
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── InMemoryConfigStore ───────────────────────────────────────────────

    #[test]
    fn in_memory_get_missing_returns_none() {
        let store = InMemoryConfigStore::new();
        assert!(store.get("agent.model").is_none());
    }

    #[test]
    fn in_memory_set_and_get() {
        let store = InMemoryConfigStore::new();
        store
            .set("agent.model", "claude-sonnet-4-6".to_owned())
            .unwrap();
        assert_eq!(store.get("agent.model").unwrap(), "claude-sonnet-4-6");
    }

    #[test]
    fn in_memory_overwrite() {
        let store = InMemoryConfigStore::new();
        store.set("key", "v1".to_owned()).unwrap();
        store.set("key", "v2".to_owned()).unwrap();
        assert_eq!(store.get("key").unwrap(), "v2");
    }

    #[test]
    fn in_memory_delete_existing_returns_true() {
        let store = InMemoryConfigStore::new();
        store.set("key", "val".to_owned()).unwrap();
        assert!(store.delete("key").unwrap());
        assert!(store.get("key").is_none());
    }

    #[test]
    fn in_memory_delete_missing_returns_false() {
        let store = InMemoryConfigStore::new();
        assert!(!store.delete("nonexistent").unwrap());
    }

    #[test]
    fn in_memory_list_prefix() {
        let store = InMemoryConfigStore::new();
        store.set("agent.model", "sonnet".to_owned()).unwrap();
        store.set("agent.provider", "anthropic".to_owned()).unwrap();
        store.set("server.port", "3000".to_owned()).unwrap();

        let agent_keys = store.list_prefix("agent.");
        assert_eq!(agent_keys.len(), 2);
        // Sorted by key
        assert_eq!(agent_keys[0].0, "agent.model");
        assert_eq!(agent_keys[1].0, "agent.provider");
    }

    #[test]
    fn in_memory_list_all_via_empty_prefix() {
        let store = InMemoryConfigStore::new();
        store.set("a", "1".to_owned()).unwrap();
        store.set("b", "2".to_owned()).unwrap();
        store.set("c", "3".to_owned()).unwrap();

        let all = store.list_all();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn in_memory_list_prefix_no_match_returns_empty() {
        let store = InMemoryConfigStore::new();
        store.set("agent.model", "sonnet".to_owned()).unwrap();
        let results = store.list_prefix("server.");
        assert!(results.is_empty());
    }

    #[test]
    fn in_memory_with_entries_seed() {
        let store = InMemoryConfigStore::with_entries([
            ("a.b".to_owned(), "1".to_owned()),
            ("a.c".to_owned(), "2".to_owned()),
        ]);
        assert_eq!(store.get("a.b").unwrap(), "1");
        assert_eq!(store.list_prefix("a.").len(), 2);
    }

    // ── FileConfigStore ───────────────────────────────────────────────────

    fn tmp_config_path() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("cairn_config_test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("config_{}_{}.toml", std::process::id(), n))
    }

    #[test]
    fn file_store_set_and_get() {
        let path = tmp_config_path();
        let store = FileConfigStore::open(&path).unwrap();
        store
            .set("agent.model", "claude-sonnet-4-6".to_owned())
            .unwrap();
        assert_eq!(store.get("agent.model").unwrap(), "claude-sonnet-4-6");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_persists_across_reopen() {
        let path = tmp_config_path();

        {
            let store = FileConfigStore::open(&path).unwrap();
            store.set("server.port", "3000".to_owned()).unwrap();
            store.set("agent.provider", "anthropic".to_owned()).unwrap();
        }

        // Open a fresh instance from the same file
        let store2 = FileConfigStore::open(&path).unwrap();
        assert_eq!(store2.get("server.port").unwrap(), "3000");
        assert_eq!(store2.get("agent.provider").unwrap(), "anthropic");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_delete_persists() {
        let path = tmp_config_path();
        let store = FileConfigStore::open(&path).unwrap();
        store.set("del_key", "val".to_owned()).unwrap();
        assert!(store.delete("del_key").unwrap());

        let store2 = FileConfigStore::open(&path).unwrap();
        assert!(store2.get("del_key").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_list_prefix() {
        let path = tmp_config_path();
        let store = FileConfigStore::open(&path).unwrap();
        store.set("signal.poll_interval", "300".to_owned()).unwrap();
        store.set("signal.gh_owner", "acme".to_owned()).unwrap();
        store.set("agent.model", "haiku".to_owned()).unwrap();

        let signal_keys = store.list_prefix("signal.");
        assert_eq!(signal_keys.len(), 2);
        assert!(signal_keys.iter().all(|(k, _)| k.starts_with("signal.")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_nonexistent_path_starts_empty() {
        let path = tmp_config_path(); // path doesn't exist yet
        assert!(!path.exists());
        let store = FileConfigStore::open(&path).unwrap();
        assert!(store.list_all().is_empty());
        // No file written until first set
        assert!(!path.exists());
        store.set("k", "v".to_owned()).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }
}
