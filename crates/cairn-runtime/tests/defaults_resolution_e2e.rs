//! RFC 008 — defaults resolution system end-to-end integration tests.
//!
//! Tests the layered scope chain: Project → Workspace → Tenant → System
//!   1. Set a system-level default
//!   2. Set a tenant-level override
//!   3. Resolve at project level — must return the tenant override (most specific)
//!   4. Set a project-level override
//!   5. Resolve again — must return the project override (most specific)
//!   6. Delete the project override — must fall back to the tenant value
//!
//! Additional coverage:
//!   - System default is the ultimate fallback when no other scope matches
//!   - Workspace-level setting wins over tenant but loses to project
//!   - Missing key returns None at all scope levels
//!   - Multiple keys are resolved independently
//!   - Clearing a system default removes the ultimate fallback

use std::sync::Arc;

use cairn_domain::{ProjectKey, Scope};
use cairn_runtime::defaults::DefaultsService;
use cairn_runtime::services::DefaultsServiceImpl;
use cairn_store::InMemoryStore;

/// Project: tenant=`t_d`, workspace=`w_d`, project=`p_d`
fn project() -> ProjectKey {
    ProjectKey::new("t_d", "w_d", "p_d")
}

// ── Tests 1–6: full scope-chain walk ─────────────────────────────────────────

/// RFC 008 §3: the scope chain must resolve the most specific override and
/// fall back gracefully when overrides are removed.
#[tokio::test]
async fn scope_chain_system_tenant_project_with_fallback() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);
    let key = "max_tokens";

    // ── (1) Set a system-level default ────────────────────────────────────
    let system_setting = svc
        .set(
            Scope::System,
            "system".to_owned(),
            key.to_owned(),
            serde_json::json!(1024),
        )
        .await
        .unwrap();

    assert_eq!(system_setting.value, serde_json::json!(1024));
    assert_eq!(system_setting.scope, Scope::System);
    assert_eq!(system_setting.key, key);

    // System default alone → resolve returns 1024.
    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!(1024)),
        "system default must be the fallback"
    );

    // ── (2) Set a tenant-level override ───────────────────────────────────
    svc.set(
        Scope::Tenant,
        "t_d".to_owned(),
        key.to_owned(),
        serde_json::json!(2048),
    )
    .await
    .unwrap();

    // ── (3) Resolve at project level — should return tenant override ───────
    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!(2048)),
        "RFC 008: tenant override must win over system default"
    );

    // ── (4) Set a project-level override ──────────────────────────────────
    svc.set(
        Scope::Project,
        "p_d".to_owned(),
        key.to_owned(),
        serde_json::json!(4096),
    )
    .await
    .unwrap();

    // ── (5) Resolve again — should return project override ────────────────
    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!(4096)),
        "RFC 008: project override must win over tenant and system"
    );

    // ── (6) Delete project override — should fall back to tenant ──────────
    svc.clear(Scope::Project, "p_d".to_owned(), key.to_owned())
        .await
        .unwrap();

    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!(2048)),
        "RFC 008: after clearing project override, must fall back to tenant value"
    );
}

// ── System is the ultimate fallback ──────────────────────────────────────────

/// RFC 008 §3: when no project/workspace/tenant override exists, the system
/// default must be returned.
#[tokio::test]
async fn system_default_is_ultimate_fallback() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);
    let key = "retry_limit";

    // Only a system default exists — every project should see it.
    svc.set(
        Scope::System,
        "system".to_owned(),
        key.to_owned(),
        serde_json::json!(3),
    )
    .await
    .unwrap();

    // Different projects in different tenants all see the system default.
    for proj in [
        ProjectKey::new("tenant_a", "ws_a", "proj_a"),
        ProjectKey::new("tenant_b", "ws_b", "proj_b"),
    ] {
        let v = svc.resolve(&proj, key).await.unwrap();
        assert_eq!(
            v,
            Some(serde_json::json!(3)),
            "system fallback must be visible from any project; project: {proj:?}"
        );
    }
}

// ── Workspace-level setting wins over tenant but loses to project ─────────────

/// RFC 008 §3: workspace scope sits between tenant and project in the chain.
/// A workspace override must win over tenant but be superseded by project.
#[tokio::test]
async fn workspace_override_wins_over_tenant_loses_to_project() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);
    let key = "log_level";

    svc.set(
        Scope::Tenant,
        "t_d".to_owned(),
        key.to_owned(),
        serde_json::json!("warn"),
    )
    .await
    .unwrap();
    svc.set(
        Scope::Workspace,
        "w_d".to_owned(),
        key.to_owned(),
        serde_json::json!("info"),
    )
    .await
    .unwrap();

    // With only tenant + workspace, workspace wins.
    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!("info")),
        "workspace must win over tenant"
    );

    // Add project override — project must now win.
    svc.set(
        Scope::Project,
        "p_d".to_owned(),
        key.to_owned(),
        serde_json::json!("debug"),
    )
    .await
    .unwrap();
    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!("debug")),
        "project must win over workspace"
    );

    // Clear project → workspace re-emerges.
    svc.clear(Scope::Project, "p_d".to_owned(), key.to_owned())
        .await
        .unwrap();
    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!("info")),
        "after clearing project, workspace must re-emerge"
    );

    // Clear workspace → tenant re-emerges.
    svc.clear(Scope::Workspace, "w_d".to_owned(), key.to_owned())
        .await
        .unwrap();
    let v = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(
        v,
        Some(serde_json::json!("warn")),
        "after clearing workspace, tenant must re-emerge"
    );
}

// ── Missing key returns None ──────────────────────────────────────────────────

/// RFC 008 §3: resolving a key that has never been set must return None
/// at any scope level, not a default value or error.
#[tokio::test]
async fn missing_key_returns_none() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);

    // Nothing set for any scope.
    let v = svc.resolve(&project(), "nonexistent_key").await.unwrap();
    assert!(v.is_none(), "unset key must resolve to None");

    // Set a different key — still None for the queried key.
    svc.set(
        Scope::System,
        "system".to_owned(),
        "other_key".to_owned(),
        serde_json::json!(1),
    )
    .await
    .unwrap();
    let v = svc.resolve(&project(), "nonexistent_key").await.unwrap();
    assert!(v.is_none(), "only matching key names must be returned");
}

// ── Multiple keys resolved independently ─────────────────────────────────────

/// RFC 008 §3: different keys at different scopes must each resolve
/// independently without cross-contamination.
#[tokio::test]
async fn multiple_keys_resolved_independently() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);

    // Key A: set at system + overridden at project.
    svc.set(
        Scope::System,
        "system".to_owned(),
        "key_a".to_owned(),
        serde_json::json!("sys_a"),
    )
    .await
    .unwrap();
    svc.set(
        Scope::Project,
        "p_d".to_owned(),
        "key_a".to_owned(),
        serde_json::json!("proj_a"),
    )
    .await
    .unwrap();

    // Key B: set only at tenant.
    svc.set(
        Scope::Tenant,
        "t_d".to_owned(),
        "key_b".to_owned(),
        serde_json::json!("tenant_b"),
    )
    .await
    .unwrap();

    // Key C: set only at system.
    svc.set(
        Scope::System,
        "system".to_owned(),
        "key_c".to_owned(),
        serde_json::json!("sys_c"),
    )
    .await
    .unwrap();

    let a = svc.resolve(&project(), "key_a").await.unwrap();
    let b = svc.resolve(&project(), "key_b").await.unwrap();
    let c = svc.resolve(&project(), "key_c").await.unwrap();

    assert_eq!(
        a,
        Some(serde_json::json!("proj_a")),
        "key_a: project must win over system"
    );
    assert_eq!(
        b,
        Some(serde_json::json!("tenant_b")),
        "key_b: tenant value must be returned"
    );
    assert_eq!(
        c,
        Some(serde_json::json!("sys_c")),
        "key_c: system fallback must be returned"
    );
}

// ── Clearing system default removes ultimate fallback ─────────────────────────

/// RFC 008 §3: clearing the system-level default must leave resolve() returning
/// None when no other scope has the key.
#[tokio::test]
async fn clearing_system_default_leaves_no_fallback() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);
    let key = "ephemeral_flag";

    svc.set(
        Scope::System,
        "system".to_owned(),
        key.to_owned(),
        serde_json::json!(true),
    )
    .await
    .unwrap();

    let before = svc.resolve(&project(), key).await.unwrap();
    assert_eq!(before, Some(serde_json::json!(true)));

    svc.clear(Scope::System, "system".to_owned(), key.to_owned())
        .await
        .unwrap();

    let after = svc.resolve(&project(), key).await.unwrap();
    assert!(
        after.is_none(),
        "after clearing system default, resolve must return None"
    );
}

// ── set() returns the stored DefaultSetting ───────────────────────────────────

/// RFC 008 §3: set() must return the persisted DefaultSetting record with
/// the correct scope, key, and value.
#[tokio::test]
async fn set_returns_persisted_default_setting() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);

    let setting = svc
        .set(
            Scope::Tenant,
            "t_d".to_owned(),
            "provider_timeout_ms".to_owned(),
            serde_json::json!(5000),
        )
        .await
        .unwrap();

    assert_eq!(setting.key, "provider_timeout_ms");
    assert_eq!(setting.value, serde_json::json!(5000));
    assert_eq!(setting.scope, Scope::Tenant);
}

// ── Value types: strings, numbers, booleans, objects all supported ─────────────

/// RFC 008 §3: defaults may hold any JSON-serialisable value; the resolver
/// must return the exact value without coercion.
#[tokio::test]
async fn various_value_types_preserved() {
    let store = Arc::new(InMemoryStore::new());
    let svc = DefaultsServiceImpl::new(store);

    let cases = [
        ("str_key", serde_json::json!("hello")),
        ("int_key", serde_json::json!(42)),
        ("bool_key", serde_json::json!(false)),
        ("obj_key", serde_json::json!({"nested": true, "count": 3})),
        ("arr_key", serde_json::json!([1, 2, 3])),
    ];

    for (key, value) in &cases {
        svc.set(
            Scope::System,
            "system".to_owned(),
            key.to_string(),
            value.clone(),
        )
        .await
        .unwrap();
    }

    for (key, expected) in &cases {
        let resolved = svc.resolve(&project(), key).await.unwrap();
        assert_eq!(
            resolved.as_ref(),
            Some(expected),
            "value for '{key}' must round-trip without coercion"
        );
    }
}
