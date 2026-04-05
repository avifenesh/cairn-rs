//! RFC 008 operator profile lifecycle end-to-end integration test.
//!
//! Validates the full operator profile pipeline:
//!   (1) create an operator profile with display name and email
//!   (2) retrieve and verify all fields round-trip correctly
//!   (3) update the profile (change display name and email)
//!   (4) list profiles by tenant
//!   (5) verify tenant isolation — profiles from another tenant invisible
//!   (6) create requires an existing tenant (enforced at service boundary)
//!   (7) set_preferences with safe keys accepted; runtime-affecting keys rejected

use std::sync::Arc;

use std::time::Duration;
use cairn_domain::{TenantId, WorkspaceRole};
use cairn_runtime::{OperatorProfileService, TenantService};
use cairn_runtime::services::{OperatorProfileServiceImpl, TenantServiceImpl};
use cairn_store::InMemoryStore;

async fn setup(tenant_name: &str) -> (Arc<InMemoryStore>, OperatorProfileServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    TenantServiceImpl::new(store.clone())
        .create(TenantId::new(tenant_name), format!("{tenant_name} Corp"))
        .await
        .unwrap();
    let profiles = OperatorProfileServiceImpl::new(store.clone());
    (store, profiles)
}

// ── (1)+(2) Create and retrieve — all fields correct ─────────────────────

#[tokio::test]
async fn create_and_retrieve_operator_profile() {
    let (_, profiles) = setup("t_create").await;

    let created = profiles
        .create(
            TenantId::new("t_create"),
            "Alice Operator".to_owned(),
            "alice@example.com".to_owned(),
            WorkspaceRole::Admin,
        )
        .await
        .unwrap();

    assert!(!created.operator_id.as_str().is_empty(), "operator_id must be generated");
    assert_eq!(created.tenant_id, TenantId::new("t_create"));
    assert_eq!(created.display_name, "Alice Operator");
    assert_eq!(created.email, "alice@example.com");
    assert_eq!(created.role, WorkspaceRole::Admin);

    // Round-trip via get().
    let fetched = profiles
        .get(&created.operator_id)
        .await
        .unwrap()
        .expect("profile must be retrievable after create");

    assert_eq!(fetched.operator_id, created.operator_id);
    assert_eq!(fetched.display_name, "Alice Operator");
    assert_eq!(fetched.email, "alice@example.com");
    assert_eq!(fetched.role, WorkspaceRole::Admin);
    assert_eq!(fetched.tenant_id, TenantId::new("t_create"));
}

// ── (3) Update profile — display name and email change ───────────────────

#[tokio::test]
async fn update_profile_changes_display_name_and_email() {
    let (_, profiles) = setup("t_update").await;

    let created = profiles
        .create(
            TenantId::new("t_update"),
            "Bob Original".to_owned(),
            "bob@old.example.com".to_owned(),
            WorkspaceRole::Member,
        )
        .await
        .unwrap();

    let before = profiles.get(&created.operator_id).await.unwrap().unwrap();
    assert_eq!(before.display_name, "Bob Original");
    assert_eq!(before.email, "bob@old.example.com");

    let updated = profiles
        .update(
            &created.operator_id,
            "Bob Updated".to_owned(),
            "bob@new.example.com".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(updated.display_name, "Bob Updated");
    assert_eq!(updated.email, "bob@new.example.com");
    // tenant and role unchanged.
    assert_eq!(updated.tenant_id, TenantId::new("t_update"));
    assert_eq!(updated.role, WorkspaceRole::Member);

    // Verify persistence.
    let after = profiles.get(&created.operator_id).await.unwrap().unwrap();
    assert_eq!(after.display_name, "Bob Updated");
    assert_eq!(after.email, "bob@new.example.com");
}

// ── (4) List profiles by tenant ───────────────────────────────────────────

#[tokio::test]
async fn list_profiles_returns_all_for_tenant() {
    let (_, profiles) = setup("t_list").await;

    let names = ["Carol", "Dave", "Eve"];
    let mut ids = Vec::new();

    for name in &names {
        // 2ms sleep: operator IDs are keyed on now_ms() — same-millisecond
        // creates produce the same ID and overwrite each other in the projection.
        tokio::time::sleep(Duration::from_millis(2)).await;
        let p = profiles
            .create(
                TenantId::new("t_list"),
                name.to_string(),
                format!("{}@example.com", name.to_lowercase()),
                WorkspaceRole::Member,
            )
            .await
            .unwrap();
        ids.push(p.operator_id);
    }

    let listed = profiles.list(&TenantId::new("t_list"), 10, 0).await.unwrap();
    assert_eq!(listed.len(), 3, "all 3 profiles must appear in list");
    assert!(listed.iter().all(|p| p.tenant_id == TenantId::new("t_list")));

    let display_names: Vec<&str> = listed.iter().map(|p| p.display_name.as_str()).collect();
    for name in &names {
        assert!(display_names.contains(name), "{name} must appear in list");
    }

    // Pagination: limit 2.
    let page = profiles.list(&TenantId::new("t_list"), 2, 0).await.unwrap();
    assert_eq!(page.len(), 2, "limit must be respected");

    // Offset 2: returns the remaining 1.
    let rest = profiles.list(&TenantId::new("t_list"), 10, 2).await.unwrap();
    assert_eq!(rest.len(), 1);
}

// ── (5) Tenant isolation ──────────────────────────────────────────────────

#[tokio::test]
async fn profiles_are_isolated_by_tenant() {
    let store = Arc::new(InMemoryStore::new());
    let tenants = TenantServiceImpl::new(store.clone());
    let profiles = OperatorProfileServiceImpl::new(store.clone());

    tenants.create(TenantId::new("tenant_x"), "Tenant X".to_owned()).await.unwrap();
    tenants.create(TenantId::new("tenant_y"), "Tenant Y".to_owned()).await.unwrap();

    profiles.create(TenantId::new("tenant_x"), "X-Alice".to_owned(), "x@x.com".to_owned(), WorkspaceRole::Admin).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    profiles.create(TenantId::new("tenant_x"), "X-Bob".to_owned(),   "xb@x.com".to_owned(), WorkspaceRole::Member).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    profiles.create(TenantId::new("tenant_y"), "Y-Carol".to_owned(), "y@y.com".to_owned(), WorkspaceRole::Viewer).await.unwrap();

    let x_profiles = profiles.list(&TenantId::new("tenant_x"), 10, 0).await.unwrap();
    let y_profiles = profiles.list(&TenantId::new("tenant_y"), 10, 0).await.unwrap();

    assert_eq!(x_profiles.len(), 2, "tenant_x must only see its own 2 profiles");
    assert_eq!(y_profiles.len(), 1, "tenant_y must only see its own 1 profile");

    assert!(x_profiles.iter().all(|p| p.tenant_id == TenantId::new("tenant_x")));
    assert!(y_profiles.iter().all(|p| p.tenant_id == TenantId::new("tenant_y")));

    // Names don't bleed across tenants.
    let x_names: Vec<&str> = x_profiles.iter().map(|p| p.display_name.as_str()).collect();
    assert!(!x_names.contains(&"Y-Carol"), "tenant_x must not see tenant_y profiles");
}

// ── (6) Create requires existing tenant ───────────────────────────────────

#[tokio::test]
async fn create_fails_if_tenant_does_not_exist() {
    let store = Arc::new(InMemoryStore::new());
    let profiles = OperatorProfileServiceImpl::new(store);

    let result = profiles
        .create(
            TenantId::new("no_such_tenant"),
            "Ghost".to_owned(),
            "ghost@example.com".to_owned(),
            WorkspaceRole::Viewer,
        )
        .await;

    assert!(result.is_err(), "create must fail when tenant does not exist");
    assert!(
        matches!(result.unwrap_err(), cairn_runtime::error::RuntimeError::NotFound { entity: "tenant", .. }),
        "error must be NotFound for tenant"
    );
}

// ── (7) set_preferences: safe accepted, runtime-affecting rejected ─────────

#[tokio::test]
async fn set_preferences_safe_accepted_runtime_affecting_rejected() {
    let (_, profiles) = setup("t_prefs").await;

    let profile = profiles
        .create(
            TenantId::new("t_prefs"),
            "Frank".to_owned(),
            "frank@example.com".to_owned(),
            WorkspaceRole::Member,
        )
        .await
        .unwrap();

    // Safe preferences — accepted.
    let ok = profiles
        .set_preferences(
            &profile.operator_id,
            serde_json::json!({"theme": "dark", "timezone": "UTC"}),
        )
        .await;
    assert!(ok.is_ok(), "safe preferences must be accepted");

    // Runtime-affecting preference — rejected.
    let err = profiles
        .set_preferences(
            &profile.operator_id,
            serde_json::json!({"theme": "light", "provider_routing": {"model": "gpt-4"}}),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, cairn_runtime::error::RuntimeError::Validation { .. }),
        "runtime-affecting key must return Validation error"
    );
}

// ── Update non-existent profile returns NotFound ──────────────────────────

#[tokio::test]
async fn update_nonexistent_profile_returns_not_found() {
    let (_, profiles) = setup("t_notfound").await;

    let result = profiles
        .update(
            &cairn_domain::OperatorId::new("op_does_not_exist"),
            "New Name".to_owned(),
            "new@example.com".to_owned(),
        )
        .await;

    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), cairn_runtime::error::RuntimeError::NotFound { entity: "operator_profile", .. })
    );
}
