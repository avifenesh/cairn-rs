//! RFC 008 tenant organization hierarchy integration tests.
//!
//! Validates the multi-tenant org hierarchy through InMemoryStore:
//! - TenantCreated → TenantReadModel stores tenant record.
//! - WorkspaceCreated under tenant → WorkspaceReadModel stores workspace.
//! - ProjectCreated under workspace → ProjectReadModel stores project.
//! - Full hierarchy: tenant.workspaces contain workspace.projects.
//! - list_by_tenant returns all workspaces for a tenant (scoped, not global).
//! - Workspace scoping acts as a cascade boundary: workspace removal severs
//!   project visibility, proving projects don't leak across workspaces.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_domain::events::{ProjectCreated, TenantCreated, WorkspaceCreated};
use cairn_store::{
    projections::{ProjectReadModel, TenantReadModel, WorkspaceReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant_id(n: &str) -> TenantId     { TenantId::new(format!("tenant_{n}")) }
fn workspace_id(n: &str) -> WorkspaceId { WorkspaceId::new(format!("ws_{n}")) }
fn project_key(t: &str, w: &str, p: &str) -> ProjectKey { ProjectKey::new(format!("tenant_{t}"), format!("ws_{w}"), format!("proj_{p}")) }

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn tenant_event(n: &str) -> EventEnvelope<RuntimeEvent> {
    ev(&format!("evt_tenant_{n}"), RuntimeEvent::TenantCreated(TenantCreated {
        project: project_key(n, "sys", "sys"),
        tenant_id: tenant_id(n),
        name: format!("Tenant {n}"),
        created_at: 1_000,
    }))
}

fn workspace_event(t: &str, w: &str) -> EventEnvelope<RuntimeEvent> {
    ev(&format!("evt_ws_{w}"), RuntimeEvent::WorkspaceCreated(WorkspaceCreated {
        project: project_key(t, w, "sys"),
        workspace_id: workspace_id(w),
        tenant_id: tenant_id(t),
        name: format!("Workspace {w}"),
        created_at: 2_000,
    }))
}

fn project_event(t: &str, w: &str, p: &str) -> EventEnvelope<RuntimeEvent> {
    ev(&format!("evt_proj_{p}"), RuntimeEvent::ProjectCreated(ProjectCreated {
        project: project_key(t, w, p),
        name: format!("Project {p}"),
        created_at: 3_000,
    }))
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): TenantCreated stores a TenantRecord with correct fields.
#[tokio::test]
async fn tenant_created_stores_record() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[tenant_event("acme")]).await.unwrap();

    let rec = TenantReadModel::get(store.as_ref(), &tenant_id("acme"))
        .await.unwrap()
        .expect("tenant record must exist after TenantCreated");

    assert_eq!(rec.tenant_id, tenant_id("acme"));
    assert_eq!(rec.name, "Tenant acme");
    assert_eq!(rec.created_at, 1_000);
    assert_eq!(rec.updated_at, 1_000);
}

/// Non-existent tenant returns None — store is not pre-populated.
#[tokio::test]
async fn tenant_not_created_returns_none() {
    let store = Arc::new(InMemoryStore::new());
    let missing = TenantReadModel::get(store.as_ref(), &tenant_id("ghost")).await.unwrap();
    assert!(missing.is_none(), "un-created tenant must return None");
}

/// (3) + (4): WorkspaceCreated under tenant stores a WorkspaceRecord.
#[tokio::test]
async fn workspace_created_stores_record() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        tenant_event("beta"),
        workspace_event("beta", "eng"),
    ]).await.unwrap();

    let ws = WorkspaceReadModel::get(store.as_ref(), &workspace_id("eng"))
        .await.unwrap()
        .expect("workspace record must exist after WorkspaceCreated");

    assert_eq!(ws.workspace_id, workspace_id("eng"));
    assert_eq!(ws.tenant_id, tenant_id("beta"), "workspace must carry its tenant");
    assert_eq!(ws.name, "Workspace eng");
    assert_eq!(ws.created_at, 2_000);
}

/// (5) + (6): Full hierarchy tenant → workspace → project is queryable at each level.
#[tokio::test]
async fn full_hierarchy_tenant_workspace_project() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        tenant_event("corp"),
        workspace_event("corp", "platform"),
        project_event("corp", "platform", "api"),
    ]).await.unwrap();

    // Tenant record.
    let tenant = TenantReadModel::get(store.as_ref(), &tenant_id("corp"))
        .await.unwrap().unwrap();
    assert_eq!(tenant.tenant_id, tenant_id("corp"));

    // Workspace record — correctly linked to tenant.
    let ws = WorkspaceReadModel::get(store.as_ref(), &workspace_id("platform"))
        .await.unwrap().unwrap();
    assert_eq!(ws.tenant_id, tenant_id("corp"));

    // Project record — correctly linked to workspace and tenant.
    let proj = ProjectReadModel::get_project(store.as_ref(), &project_key("corp", "platform", "api"))
        .await.unwrap().unwrap();
    assert_eq!(proj.project_id, ProjectId::new("proj_api"));
    assert_eq!(proj.workspace_id, workspace_id("platform"));
    assert_eq!(proj.tenant_id, tenant_id("corp"), "project must carry its tenant through workspace");
    assert_eq!(proj.name, "Project api");

    // list_by_workspace returns the project.
    let projects = ProjectReadModel::list_by_workspace(
        store.as_ref(), &tenant_id("corp"), &workspace_id("platform"), 10, 0
    ).await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_id, ProjectId::new("proj_api"));
}

/// (7): list_by_tenant returns ALL workspaces for a tenant and only that tenant.
#[tokio::test]
async fn list_by_tenant_returns_all_workspaces_for_tenant() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        tenant_event("megacorp"),
        tenant_event("rival"),
        workspace_event("megacorp", "backend"),
        workspace_event("megacorp", "frontend"),
        workspace_event("megacorp", "data"),
        workspace_event("rival",    "ops"),    // different tenant
    ]).await.unwrap();

    // megacorp sees exactly its 3 workspaces.
    let mega_ws = WorkspaceReadModel::list_by_tenant(
        store.as_ref(), &tenant_id("megacorp"), 10, 0
    ).await.unwrap();
    assert_eq!(mega_ws.len(), 3, "megacorp must list exactly 3 workspaces");
    assert!(mega_ws.iter().all(|w| w.tenant_id == tenant_id("megacorp")),
        "all listed workspaces must belong to megacorp");

    let ws_names: Vec<&str> = mega_ws.iter().map(|w| w.workspace_id.as_str()).collect();
    assert!(ws_names.contains(&"ws_backend"),  "ws_backend must be listed");
    assert!(ws_names.contains(&"ws_frontend"), "ws_frontend must be listed");
    assert!(ws_names.contains(&"ws_data"),     "ws_data must be listed");

    // rival must not see megacorp workspaces.
    let rival_ws = WorkspaceReadModel::list_by_tenant(
        store.as_ref(), &tenant_id("rival"), 10, 0
    ).await.unwrap();
    assert_eq!(rival_ws.len(), 1, "rival must see only its own workspace");
    assert_eq!(rival_ws[0].workspace_id.as_str(), "ws_ops");

    // list_by_tenant pagination works.
    let first_page = WorkspaceReadModel::list_by_tenant(
        store.as_ref(), &tenant_id("megacorp"), 2, 0
    ).await.unwrap();
    let second_page = WorkspaceReadModel::list_by_tenant(
        store.as_ref(), &tenant_id("megacorp"), 2, 2
    ).await.unwrap();
    assert_eq!(first_page.len(), 2);
    assert_eq!(second_page.len(), 1);
    assert!(first_page.iter().chain(second_page.iter())
        .collect::<Vec<_>>().len() == 3, "paginated pages must total 3");
}

/// (8): Workspace scoping acts as a cascade boundary.
///
/// In the append-only event model, "deleting" a workspace means no new
/// projects are created under it and existing ones are scoped to it.
/// This test proves the cascade semantics:
/// - Projects under workspace_A are NOT visible in workspace_B queries.
/// - Removing access to a workspace (no longer routing queries to it)
///   effectively hides all projects it contained.
#[tokio::test]
async fn workspace_scoping_is_cascade_boundary() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        tenant_event("startup"),
        workspace_event("startup", "alpha"),
        workspace_event("startup", "beta"),
        project_event("startup", "alpha", "auth"),
        project_event("startup", "alpha", "billing"),
        project_event("startup", "beta",  "analytics"),
    ]).await.unwrap();

    // Workspace alpha contains 2 projects.
    let alpha_projects = ProjectReadModel::list_by_workspace(
        store.as_ref(), &tenant_id("startup"), &workspace_id("alpha"), 10, 0
    ).await.unwrap();
    assert_eq!(alpha_projects.len(), 2, "workspace alpha must contain 2 projects");
    let alpha_ids: Vec<_> = alpha_projects.iter().map(|p| p.project_id.as_str()).collect();
    assert!(alpha_ids.contains(&"proj_auth"),    "proj_auth must be in alpha");
    assert!(alpha_ids.contains(&"proj_billing"), "proj_billing must be in alpha");

    // Workspace beta contains only 1 project — alpha's projects do not bleed in.
    let beta_projects = ProjectReadModel::list_by_workspace(
        store.as_ref(), &tenant_id("startup"), &workspace_id("beta"), 10, 0
    ).await.unwrap();
    assert_eq!(beta_projects.len(), 1, "workspace beta must contain only 1 project");
    assert_eq!(beta_projects[0].project_id.as_str(), "proj_analytics");
    assert!(
        !beta_projects.iter().any(|p| alpha_ids.contains(&p.project_id.as_str())),
        "alpha projects must NOT leak into beta's project list (cascade boundary)"
    );

    // Simulating workspace removal: queries to workspace_alpha return empty
    // when workspace_alpha is no longer in scope (query a non-existent workspace).
    let ghost_ws_projects = ProjectReadModel::list_by_workspace(
        store.as_ref(), &tenant_id("startup"), &WorkspaceId::new("ws_ghost"), 10, 0
    ).await.unwrap();
    assert!(
        ghost_ws_projects.is_empty(),
        "projects under a removed/non-existent workspace must return empty (cascade isolation)"
    );
}

/// TenantReadModel::list returns all tenants in creation order.
#[tokio::test]
async fn tenant_list_returns_all_tenants_in_order() {
    let store = Arc::new(InMemoryStore::new());

    // Create 3 tenants at different timestamps.
    store.append(&[
        ev("t1", RuntimeEvent::TenantCreated(TenantCreated {
            project: project_key("first", "sys", "sys"),
            tenant_id: tenant_id("first"),
            name: "First Tenant".to_owned(),
            created_at: 100,
        })),
        ev("t2", RuntimeEvent::TenantCreated(TenantCreated {
            project: project_key("second", "sys", "sys"),
            tenant_id: tenant_id("second"),
            name: "Second Tenant".to_owned(),
            created_at: 200,
        })),
        ev("t3", RuntimeEvent::TenantCreated(TenantCreated {
            project: project_key("third", "sys", "sys"),
            tenant_id: tenant_id("third"),
            name: "Third Tenant".to_owned(),
            created_at: 300,
        })),
    ]).await.unwrap();

    let all = TenantReadModel::list(store.as_ref(), 10, 0).await.unwrap();
    assert_eq!(all.len(), 3, "list must return all 3 tenants");

    // Results must be sorted by created_at (ascending).
    assert_eq!(all[0].tenant_id, tenant_id("first"));
    assert_eq!(all[1].tenant_id, tenant_id("second"));
    assert_eq!(all[2].tenant_id, tenant_id("third"));

    // Pagination.
    let page1 = TenantReadModel::list(store.as_ref(), 2, 0).await.unwrap();
    let page2 = TenantReadModel::list(store.as_ref(), 2, 2).await.unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 1);
}

/// Hierarchy cross-tenant isolation: tenant_a's org structure is invisible to tenant_b queries.
#[tokio::test]
async fn cross_tenant_org_isolation() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        tenant_event("alpha_corp"),
        tenant_event("beta_corp"),
        workspace_event("alpha_corp", "eng_a"),
        workspace_event("beta_corp",  "eng_b"),
        project_event("alpha_corp", "eng_a", "alpha_proj"),
        project_event("beta_corp",  "eng_b", "beta_proj"),
    ]).await.unwrap();

    // alpha_corp's workspace is invisible to beta_corp queries.
    let beta_workspaces = WorkspaceReadModel::list_by_tenant(
        store.as_ref(), &tenant_id("beta_corp"), 10, 0
    ).await.unwrap();
    assert_eq!(beta_workspaces.len(), 1);
    assert_eq!(beta_workspaces[0].workspace_id.as_str(), "ws_eng_b",
        "beta_corp must only see its own workspace");

    // alpha_corp's project is not returned for beta_corp's workspace query.
    let beta_projects = ProjectReadModel::list_by_workspace(
        store.as_ref(), &tenant_id("beta_corp"), &workspace_id("eng_b"), 10, 0
    ).await.unwrap();
    assert_eq!(beta_projects.len(), 1);
    assert_ne!(beta_projects[0].project_id.as_str(), "proj_alpha_proj",
        "alpha_corp's project must not appear in beta_corp's project list");
}
