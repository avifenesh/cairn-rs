//! Integration tests for ConfigStore — GAP-008 TOML config persistence.
//!
//! Tests cover: set/get/delete/list for InMemoryConfigStore and FileConfigStore,
//! persistence across FileConfigStore reopen, and prefix-filtered listing.

use std::path::PathBuf;
use cairn_runtime::config_store::{ConfigStore, FileConfigStore, InMemoryConfigStore};

fn tmp_path(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("cairn_cfg_inttest_{suffix}_{}.toml", std::process::id()))
}

// ── InMemoryConfigStore ───────────────────────────────────────────────────

#[test]
fn config_store_in_memory_set_get() {
    let store = InMemoryConfigStore::new();
    store.set("agent.model", "claude-sonnet-4-6".to_owned()).unwrap();
    assert_eq!(store.get("agent.model").unwrap(), "claude-sonnet-4-6");
}

#[test]
fn config_store_in_memory_get_missing_is_none() {
    let store = InMemoryConfigStore::new();
    assert!(store.get("nonexistent.key").is_none());
}

#[test]
fn config_store_in_memory_delete_returns_existed() {
    let store = InMemoryConfigStore::new();
    store.set("key", "value".to_owned()).unwrap();
    assert!(store.delete("key").unwrap(), "delete existing key must return true");
    assert!(store.get("key").is_none(), "key must be absent after delete");
    assert!(!store.delete("key").unwrap(), "delete missing key must return false");
}

#[test]
fn config_store_in_memory_list_prefix() {
    let store = InMemoryConfigStore::new();
    store.set("agent.model", "sonnet".to_owned()).unwrap();
    store.set("agent.provider", "anthropic".to_owned()).unwrap();
    store.set("server.port", "3000".to_owned()).unwrap();

    let agent = store.list_prefix("agent.");
    assert_eq!(agent.len(), 2);
    // Results must be sorted by key
    assert_eq!(agent[0].0, "agent.model");
    assert_eq!(agent[0].1, "sonnet");
    assert_eq!(agent[1].0, "agent.provider");

    // server prefix returns only server keys
    let server = store.list_prefix("server.");
    assert_eq!(server.len(), 1);
    assert_eq!(server[0].0, "server.port");

    // empty prefix = list_all
    let all = store.list_prefix("");
    assert_eq!(all.len(), 3);
}

#[test]
fn config_store_in_memory_overwrite() {
    let store = InMemoryConfigStore::new();
    store.set("key", "old".to_owned()).unwrap();
    store.set("key", "new".to_owned()).unwrap();
    assert_eq!(store.get("key").unwrap(), "new");
}

#[test]
fn config_store_in_memory_list_prefix_no_match() {
    let store = InMemoryConfigStore::new();
    store.set("agent.model", "haiku".to_owned()).unwrap();
    assert!(store.list_prefix("signal.").is_empty());
}

// ── FileConfigStore ───────────────────────────────────────────────────────

#[test]
fn config_store_file_set_get_delete() {
    let path = tmp_path("set_get_del");
    let store = FileConfigStore::open(&path).unwrap();

    store.set("agent.model", "claude-haiku-4-5".to_owned()).unwrap();
    store.set("server.port", "3000".to_owned()).unwrap();

    assert_eq!(store.get("agent.model").unwrap(), "claude-haiku-4-5");
    assert_eq!(store.get("server.port").unwrap(), "3000");
    assert!(store.get("missing").is_none());

    assert!(store.delete("agent.model").unwrap());
    assert!(store.get("agent.model").is_none());
    assert!(!store.delete("agent.model").unwrap());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn config_store_file_list_with_prefix() {
    let path = tmp_path("list_prefix");
    let store = FileConfigStore::open(&path).unwrap();

    store.set("signal.poll_interval", "300".to_owned()).unwrap();
    store.set("signal.gh_owner", "acme".to_owned()).unwrap();
    store.set("memory.context_budget", "8000".to_owned()).unwrap();

    let signal = store.list_prefix("signal.");
    assert_eq!(signal.len(), 2);
    assert!(signal.iter().all(|(k, _)| k.starts_with("signal.")));

    // Sorted by key
    assert_eq!(signal[0].0, "signal.gh_owner");
    assert_eq!(signal[1].0, "signal.poll_interval");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn config_store_file_persists_across_reopen() {
    let path = tmp_path("persist");

    // First instance: write
    {
        let store = FileConfigStore::open(&path).unwrap();
        store.set("agent.model", "claude-opus-4-6".to_owned()).unwrap();
        store.set("server.port", "8080".to_owned()).unwrap();
    }

    // Second instance from same file: must read persisted values
    let store2 = FileConfigStore::open(&path).unwrap();
    assert_eq!(store2.get("agent.model").unwrap(), "claude-opus-4-6");
    assert_eq!(store2.get("server.port").unwrap(), "8080");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn config_store_file_delete_persists_across_reopen() {
    let path = tmp_path("del_persist");

    {
        let store = FileConfigStore::open(&path).unwrap();
        store.set("temp.key", "val".to_owned()).unwrap();
        store.delete("temp.key").unwrap();
    }

    let store2 = FileConfigStore::open(&path).unwrap();
    assert!(store2.get("temp.key").is_none());

    let _ = std::fs::remove_file(&path);
}
