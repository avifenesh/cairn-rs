//! Default settings lifecycle tests (RFC 002).
//!
//! Validates the default settings pipeline: set, get, clear, scope isolation,
//! and overwrite (last-write-wins) semantics.
//!
//! DefaultSetting is keyed by (scope, scope_id, key) — a composite key
//! that ensures the same key name can exist independently per scope level.
//!
//! Scope hierarchy: System > Tenant > Workspace > Project
//! Each scope level has its own isolated namespace.
//!
//! Read model:
//!   get(scope, scope_id, key)   — point lookup
//!   list_by_scope(scope, scope_id) — all settings for a scope

use cairn_domain::{
    DefaultSettingCleared, DefaultSettingSet, EventEnvelope, EventId, EventSource,
    RuntimeEvent,
};
use cairn_domain::tenancy::Scope;
use cairn_store::{
    projections::DefaultsReadModel,
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    // DefaultSettingSet/Cleared have no project — use raw envelope.
    use cairn_domain::OwnershipKey;
    EventEnvelope {
        event_id:       EventId::new(id),
        source:         EventSource::Runtime,
        ownership:      OwnershipKey::System,
        causation_id:   None,
        correlation_id: None,
        payload,
    }
}

fn set(evt_id: &str, scope: Scope, scope_id: &str, key: &str, value: serde_json::Value)
    -> EventEnvelope<RuntimeEvent>
{
    evt(evt_id, RuntimeEvent::DefaultSettingSet(DefaultSettingSet {
        scope,
        scope_id: scope_id.to_owned(),
        key:      key.to_owned(),
        value,
    }))
}

fn clear(evt_id: &str, scope: Scope, scope_id: &str, key: &str)
    -> EventEnvelope<RuntimeEvent>
{
    evt(evt_id, RuntimeEvent::DefaultSettingCleared(DefaultSettingCleared {
        scope,
        scope_id: scope_id.to_owned(),
        key:      key.to_owned(),
    }))
}

// ── 1. DefaultSettingSet stores the record ────────────────────────────────────

#[tokio::test]
async fn setting_set_is_stored() {
    let store = InMemoryStore::new();

    store.append(&[set(
        "e1", Scope::Tenant, "t1", "max_sessions",
        serde_json::json!(10),
    )]).await.unwrap();

    let setting = DefaultsReadModel::get(&store, Scope::Tenant, "t1", "max_sessions")
        .await.unwrap()
        .expect("setting must exist after DefaultSettingSet");

    assert_eq!(setting.key, "max_sessions");
    assert_eq!(setting.value, serde_json::json!(10));
    assert_eq!(setting.scope, Scope::Tenant);
}

// ── 2. Setting values persist across all JSON types ───────────────────────────

#[tokio::test]
async fn setting_values_persist_all_json_types() {
    let store = InMemoryStore::new();

    let cases = [
        ("string_setting", serde_json::json!("hello")),
        ("int_setting",    serde_json::json!(42)),
        ("bool_setting",   serde_json::json!(true)),
        ("object_setting", serde_json::json!({ "a": 1, "b": "x" })),
        ("array_setting",  serde_json::json!([1, 2, 3])),
        ("null_setting",   serde_json::json!(null)),
    ];

    for (i, (key, value)) in cases.iter().enumerate() {
        store.append(&[set(&format!("e{i}"), Scope::Tenant, "t_types", key, value.clone())])
            .await.unwrap();
    }

    for (key, expected) in &cases {
        let setting = DefaultsReadModel::get(&store, Scope::Tenant, "t_types", key)
            .await.unwrap()
            .unwrap_or_else(|| panic!("setting {key} must exist"));
        assert_eq!(setting.value, *expected, "{key}: value must round-trip");
    }
}

// ── 3. DefaultSettingCleared removes the setting ─────────────────────────────

#[tokio::test]
async fn setting_cleared_removes_record() {
    let store = InMemoryStore::new();

    store.append(&[set("e1", Scope::Tenant, "t_clear", "timeout_ms", serde_json::json!(5000))])
        .await.unwrap();

    // Verify present.
    let before = DefaultsReadModel::get(&store, Scope::Tenant, "t_clear", "timeout_ms")
        .await.unwrap();
    assert!(before.is_some(), "setting must exist before clear");

    store.append(&[clear("e2", Scope::Tenant, "t_clear", "timeout_ms")])
        .await.unwrap();

    let after = DefaultsReadModel::get(&store, Scope::Tenant, "t_clear", "timeout_ms")
        .await.unwrap();
    assert!(after.is_none(), "setting must be absent after DefaultSettingCleared");
}

// ── 4. Clearing non-existent setting is a no-op ───────────────────────────────

#[tokio::test]
async fn clearing_nonexistent_setting_is_noop() {
    let store = InMemoryStore::new();

    // Clear a setting that was never set — must not panic.
    store.append(&[clear("e1", Scope::Tenant, "t_noop", "ghost_key")])
        .await.unwrap();

    let result = DefaultsReadModel::get(&store, Scope::Tenant, "t_noop", "ghost_key")
        .await.unwrap();
    assert!(result.is_none());
}

// ── 5. Overwrite: set same key twice — latest value wins ──────────────────────

#[tokio::test]
async fn overwrite_same_key_latest_wins() {
    let store = InMemoryStore::new();

    store.append(&[
        set("e1", Scope::Tenant, "t_over", "limit", serde_json::json!(100)),
        set("e2", Scope::Tenant, "t_over", "limit", serde_json::json!(500)),
    ]).await.unwrap();

    let setting = DefaultsReadModel::get(&store, Scope::Tenant, "t_over", "limit")
        .await.unwrap().unwrap();
    assert_eq!(setting.value, serde_json::json!(500),
        "second set must overwrite the first — latest value wins");
}

#[tokio::test]
async fn overwrite_three_times_last_value_wins() {
    let store = InMemoryStore::new();

    for (i, val) in [10u64, 20, 30].iter().enumerate() {
        store.append(&[set(
            &format!("e{i}"), Scope::Workspace, "ws_over", "rate",
            serde_json::json!(val),
        )]).await.unwrap();
    }

    let setting = DefaultsReadModel::get(&store, Scope::Workspace, "ws_over", "rate")
        .await.unwrap().unwrap();
    assert_eq!(setting.value, serde_json::json!(30), "final value must be 30");
}

// ── 6. Scope isolation: Tenant scope ─────────────────────────────────────────

#[tokio::test]
async fn tenant_scope_is_isolated_by_scope_id() {
    let store = InMemoryStore::new();

    store.append(&[
        set("e1", Scope::Tenant, "tenant_a", "feature_x", serde_json::json!(true)),
        set("e2", Scope::Tenant, "tenant_b", "feature_x", serde_json::json!(false)),
    ]).await.unwrap();

    let ta = DefaultsReadModel::get(&store, Scope::Tenant, "tenant_a", "feature_x")
        .await.unwrap().unwrap();
    assert_eq!(ta.value, serde_json::json!(true));

    let tb = DefaultsReadModel::get(&store, Scope::Tenant, "tenant_b", "feature_x")
        .await.unwrap().unwrap();
    assert_eq!(tb.value, serde_json::json!(false));
}

// ── 7. Scope isolation: Workspace scope ──────────────────────────────────────

#[tokio::test]
async fn workspace_scope_isolated_from_tenant_scope() {
    let store = InMemoryStore::new();

    // Same key name, different scopes — each is independent.
    store.append(&[
        set("e1", Scope::Tenant,    "t_ws_iso", "timeout", serde_json::json!(30)),
        set("e2", Scope::Workspace, "ws_iso",   "timeout", serde_json::json!(60)),
        set("e3", Scope::Project,   "proj_iso", "timeout", serde_json::json!(90)),
    ]).await.unwrap();

    let tenant_setting = DefaultsReadModel::get(&store, Scope::Tenant, "t_ws_iso", "timeout")
        .await.unwrap().unwrap();
    assert_eq!(tenant_setting.value, serde_json::json!(30));

    let ws_setting = DefaultsReadModel::get(&store, Scope::Workspace, "ws_iso", "timeout")
        .await.unwrap().unwrap();
    assert_eq!(ws_setting.value, serde_json::json!(60));

    let proj_setting = DefaultsReadModel::get(&store, Scope::Project, "proj_iso", "timeout")
        .await.unwrap().unwrap();
    assert_eq!(proj_setting.value, serde_json::json!(90));

    // Querying with wrong scope returns None.
    let none = DefaultsReadModel::get(&store, Scope::Tenant, "ws_iso", "timeout")
        .await.unwrap();
    assert!(none.is_none(), "workspace setting must not appear under tenant scope");
}

// ── 8. System scope is its own namespace ──────────────────────────────────────

#[tokio::test]
async fn system_scope_is_independent_namespace() {
    let store = InMemoryStore::new();

    store.append(&[
        set("e1", Scope::System, "_", "global_feature", serde_json::json!("v2")),
        set("e2", Scope::Tenant, "_", "global_feature", serde_json::json!("v1")),
    ]).await.unwrap();

    let system = DefaultsReadModel::get(&store, Scope::System, "_", "global_feature")
        .await.unwrap().unwrap();
    assert_eq!(system.value, serde_json::json!("v2"));

    let tenant = DefaultsReadModel::get(&store, Scope::Tenant, "_", "global_feature")
        .await.unwrap().unwrap();
    assert_eq!(tenant.value, serde_json::json!("v1"));

    assert_ne!(system.value, tenant.value,
        "System and Tenant scopes must be independent namespaces");
}

// ── 9. get() returns None for unknown key ─────────────────────────────────────

#[tokio::test]
async fn get_returns_none_for_unknown_key() {
    let store = InMemoryStore::new();
    let result = DefaultsReadModel::get(&store, Scope::Tenant, "t_unknown", "missing")
        .await.unwrap();
    assert!(result.is_none());
}

// ── 10. list_by_scope returns all settings for a scope ────────────────────────

#[tokio::test]
async fn list_by_scope_returns_all_scope_settings() {
    let store = InMemoryStore::new();

    store.append(&[
        set("e1", Scope::Tenant, "t_list", "key_a", serde_json::json!(1)),
        set("e2", Scope::Tenant, "t_list", "key_b", serde_json::json!(2)),
        set("e3", Scope::Tenant, "t_list", "key_c", serde_json::json!(3)),
        // Different scope_id — must not appear.
        set("e4", Scope::Tenant, "t_other", "key_a", serde_json::json!(99)),
    ]).await.unwrap();

    let settings = DefaultsReadModel::list_by_scope(&store, Scope::Tenant, "t_list")
        .await.unwrap();

    assert_eq!(settings.len(), 3, "t_list has 3 settings");
    assert!(settings.iter().all(|s| s.scope == Scope::Tenant));

    let keys: std::collections::HashSet<_> = settings.iter().map(|s| s.key.as_str()).collect();
    assert!(keys.contains("key_a"));
    assert!(keys.contains("key_b"));
    assert!(keys.contains("key_c"));
    // t_other's key_a must not appear.
    let values: Vec<_> = settings.iter().filter(|s| s.key == "key_a").collect();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].value, serde_json::json!(1), "only t_list's key_a");
}

// ── 11. list_by_scope returns empty for unknown scope ─────────────────────────

#[tokio::test]
async fn list_by_scope_returns_empty_for_unknown_scope_id() {
    let store = InMemoryStore::new();
    let result = DefaultsReadModel::list_by_scope(&store, Scope::Tenant, "nobody")
        .await.unwrap();
    assert!(result.is_empty());
}

// ── 12. Set → Clear → Set cycle ───────────────────────────────────────────────

#[tokio::test]
async fn set_clear_set_cycle() {
    let store = InMemoryStore::new();

    // Set.
    store.append(&[set("e1", Scope::Tenant, "t_cycle", "toggle", serde_json::json!("on"))])
        .await.unwrap();
    let v1 = DefaultsReadModel::get(&store, Scope::Tenant, "t_cycle", "toggle")
        .await.unwrap().unwrap();
    assert_eq!(v1.value, serde_json::json!("on"));

    // Clear.
    store.append(&[clear("e2", Scope::Tenant, "t_cycle", "toggle")])
        .await.unwrap();
    let cleared = DefaultsReadModel::get(&store, Scope::Tenant, "t_cycle", "toggle")
        .await.unwrap();
    assert!(cleared.is_none());

    // Re-set with new value.
    store.append(&[set("e3", Scope::Tenant, "t_cycle", "toggle", serde_json::json!("off"))])
        .await.unwrap();
    let v2 = DefaultsReadModel::get(&store, Scope::Tenant, "t_cycle", "toggle")
        .await.unwrap().unwrap();
    assert_eq!(v2.value, serde_json::json!("off"),
        "setting after clear must reflect the new value");
}
