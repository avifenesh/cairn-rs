//! RFC 008 tenant/workspace lifecycle end-to-end integration test.
//!
//! Validates the full organizational hierarchy:
//!   (1) create a tenant
//!   (2) create a workspace within the tenant
//!   (3) create a project within the workspace
//!   (4) add a member to the workspace
//!   (5) verify tenant/workspace/project hierarchy via read models
//!   (6) verify workspace membership (role, list, duplicate rejection)

use std::sync::Arc;

use cairn_domain::{
    OperatorId, ProjectId, TenantId, WorkspaceId, WorkspaceKey, WorkspaceRole,
};
use cairn_runtime::{
    ProjectService, TenantService, WorkspaceMembershipService, WorkspaceService,
};
use cairn_runtime::services::{
    ProjectServiceImpl, TenantServiceImpl, WorkspaceMembershipServiceImpl, WorkspaceServiceImpl,
};
use cairn_store::InMemoryStore;

fn services() -> (
    Arc<InMemoryStore>,
    TenantServiceImpl<InMemoryStore>,
    WorkspaceServiceImpl<InMemoryStore>,
    ProjectServiceImpl<InMemoryStore>,
    WorkspaceMembershipServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    (
        store.clone(),
        TenantServiceImpl::new(store.clone()),
        WorkspaceServiceImpl::new(store.clone()),
        ProjectServiceImpl::new(store.clone()),
        WorkspaceMembershipServiceImpl::new(store),
    )
}

// ── (1) Create tenant ────────────────────────────────────────────────────

#[tokio::test]
async fn create_tenant_persists_and_is_retrievable() {
    let (_, tenants, _, _, _) = services();

    let tenant = tenants
        .create(TenantId::new("tenant_acme"), "Acme Corp".to_owned())
        .await
        .unwrap();

    assert_eq!(tenant.tenant_id, TenantId::new("tenant_acme"));
    assert_eq!(tenant.name, "Acme Corp");
    assert!(tenant.created_at > 0);

    let fetched = tenants
        .get(&TenantId::new("tenant_acme"))
        .await
        .unwrap()
        .expect("tenant must be retrievable after create");

    assert_eq!(fetched.tenant_id, tenant.tenant_id);
    assert_eq!(fetched.name, "Acme Corp");
}

#[tokio::test]
async fn duplicate_tenant_create_returns_conflict() {
    let (_, tenants, _, _, _) = services();

    tenants
        .create(TenantId::new("tenant_dup"), "Duplicate".to_owned())
        .await
        .unwrap();

    let result = tenants
        .create(TenantId::new("tenant_dup"), "Duplicate Again".to_owned())
        .await;

    assert!(result.is_err(), "creating a tenant with the same ID must fail");
}

// ── (2) Create workspace within tenant ───────────────────────────────────

#[tokio::test]
async fn create_workspace_scoped_to_tenant() {
    let (_, tenants, workspaces, _, _) = services();

    tenants
        .create(TenantId::new("tenant_ws"), "Workspace Tenant".to_owned())
        .await
        .unwrap();

    let ws = workspaces
        .create(
            TenantId::new("tenant_ws"),
            WorkspaceId::new("ws_main"),
            "Main Workspace".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(ws.workspace_id, WorkspaceId::new("ws_main"));
    assert_eq!(ws.tenant_id, TenantId::new("tenant_ws"));
    assert_eq!(ws.name, "Main Workspace");
    assert!(ws.created_at > 0);

    let fetched = workspaces
        .get(&WorkspaceId::new("ws_main"))
        .await
        .unwrap()
        .expect("workspace must be retrievable after create");

    assert_eq!(fetched, ws);
}

// ── (3) Create project within workspace ──────────────────────────────────

#[tokio::test]
async fn create_project_scoped_to_workspace() {
    let (_, tenants, workspaces, projects, _) = services();

    tenants
        .create(TenantId::new("tenant_proj"), "Project Tenant".to_owned())
        .await
        .unwrap();
    workspaces
        .create(
            TenantId::new("tenant_proj"),
            WorkspaceId::new("ws_proj"),
            "Project Workspace".to_owned(),
        )
        .await
        .unwrap();

    let project_key = cairn_domain::ProjectKey::new("tenant_proj", "ws_proj", "proj_alpha");
    let project = projects
        .create(project_key.clone(), "Alpha Project".to_owned())
        .await
        .unwrap();

    assert_eq!(project.project_id, ProjectId::new("proj_alpha"));
    assert_eq!(project.workspace_id, WorkspaceId::new("ws_proj"));
    assert_eq!(project.tenant_id, TenantId::new("tenant_proj"));
    assert_eq!(project.name, "Alpha Project");

    let fetched = projects
        .get(&project_key)
        .await
        .unwrap()
        .expect("project must be retrievable after create");

    assert_eq!(fetched, project);
}

// ── (4) + (6) Add member to workspace ────────────────────────────────────

#[tokio::test]
async fn add_member_to_workspace_with_role() {
    let (_, tenants, workspaces, _, memberships) = services();

    tenants
        .create(TenantId::new("tenant_mem"), "Member Tenant".to_owned())
        .await
        .unwrap();
    workspaces
        .create(
            TenantId::new("tenant_mem"),
            WorkspaceId::new("ws_mem"),
            "Member Workspace".to_owned(),
        )
        .await
        .unwrap();

    let ws_key = WorkspaceKey::new("tenant_mem", "ws_mem");

    let membership = memberships
        .add_member(ws_key.clone(), "operator_alice".to_owned(), WorkspaceRole::Admin)
        .await
        .unwrap();

    assert_eq!(membership.workspace_id, WorkspaceId::new("ws_mem"));
    assert_eq!(membership.operator_id, OperatorId::new("operator_alice"));
    assert_eq!(membership.role, WorkspaceRole::Admin);
}

#[tokio::test]
async fn list_members_returns_all_added_members() {
    let (_, tenants, workspaces, _, memberships) = services();

    tenants
        .create(TenantId::new("tenant_list_mem"), "List Tenant".to_owned())
        .await
        .unwrap();
    workspaces
        .create(
            TenantId::new("tenant_list_mem"),
            WorkspaceId::new("ws_list_mem"),
            "List Workspace".to_owned(),
        )
        .await
        .unwrap();

    let ws_key = WorkspaceKey::new("tenant_list_mem", "ws_list_mem");

    memberships
        .add_member(ws_key.clone(), "op_alice".to_owned(), WorkspaceRole::Owner)
        .await
        .unwrap();
    memberships
        .add_member(ws_key.clone(), "op_bob".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();
    memberships
        .add_member(ws_key.clone(), "op_carol".to_owned(), WorkspaceRole::Viewer)
        .await
        .unwrap();

    let members = memberships.list_members(&ws_key).await.unwrap();

    assert_eq!(members.len(), 3, "all 3 members must be listed");

    let alice = members.iter().find(|m| m.operator_id == OperatorId::new("op_alice")).unwrap();
    assert_eq!(alice.role, WorkspaceRole::Owner);

    let bob = members.iter().find(|m| m.operator_id == OperatorId::new("op_bob")).unwrap();
    assert_eq!(bob.role, WorkspaceRole::Member);

    let carol = members.iter().find(|m| m.operator_id == OperatorId::new("op_carol")).unwrap();
    assert_eq!(carol.role, WorkspaceRole::Viewer);
}

#[tokio::test]
async fn duplicate_member_add_returns_conflict() {
    let (_, tenants, workspaces, _, memberships) = services();

    tenants
        .create(TenantId::new("tenant_dup_mem"), "Dup Mem Tenant".to_owned())
        .await
        .unwrap();
    workspaces
        .create(
            TenantId::new("tenant_dup_mem"),
            WorkspaceId::new("ws_dup_mem"),
            "Dup Workspace".to_owned(),
        )
        .await
        .unwrap();

    let ws_key = WorkspaceKey::new("tenant_dup_mem", "ws_dup_mem");

    memberships
        .add_member(ws_key.clone(), "op_eve".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();

    let result = memberships
        .add_member(ws_key.clone(), "op_eve".to_owned(), WorkspaceRole::Admin)
        .await;

    assert!(result.is_err(), "adding the same member twice must fail");
}

// ── (5) Hierarchy correctness via read models ─────────────────────────────

#[tokio::test]
async fn full_hierarchy_tenant_workspace_project() {
    let (_, tenants, workspaces, projects, _) = services();

    // Create two tenants.
    tenants
        .create(TenantId::new("tenant_hier_a"), "Tenant A".to_owned())
        .await
        .unwrap();
    tenants
        .create(TenantId::new("tenant_hier_b"), "Tenant B".to_owned())
        .await
        .unwrap();

    // Tenant A gets two workspaces.
    workspaces
        .create(TenantId::new("tenant_hier_a"), WorkspaceId::new("ws_a1"), "A-WS1".to_owned())
        .await
        .unwrap();
    workspaces
        .create(TenantId::new("tenant_hier_a"), WorkspaceId::new("ws_a2"), "A-WS2".to_owned())
        .await
        .unwrap();

    // Tenant B gets one workspace.
    workspaces
        .create(TenantId::new("tenant_hier_b"), WorkspaceId::new("ws_b1"), "B-WS1".to_owned())
        .await
        .unwrap();

    // Each workspace gets a project.
    projects
        .create(
            cairn_domain::ProjectKey::new("tenant_hier_a", "ws_a1", "proj_a1"),
            "A1 Project".to_owned(),
        )
        .await
        .unwrap();
    projects
        .create(
            cairn_domain::ProjectKey::new("tenant_hier_a", "ws_a2", "proj_a2"),
            "A2 Project".to_owned(),
        )
        .await
        .unwrap();
    projects
        .create(
            cairn_domain::ProjectKey::new("tenant_hier_b", "ws_b1", "proj_b1"),
            "B1 Project".to_owned(),
        )
        .await
        .unwrap();

    // Tenant A: list workspaces — must return exactly 2.
    let a_workspaces = workspaces
        .list_by_tenant(&TenantId::new("tenant_hier_a"), 10, 0)
        .await
        .unwrap();
    assert_eq!(a_workspaces.len(), 2, "tenant A must have exactly 2 workspaces");
    assert!(a_workspaces.iter().all(|w| w.tenant_id == TenantId::new("tenant_hier_a")));

    // Tenant B: list workspaces — must return exactly 1.
    let b_workspaces = workspaces
        .list_by_tenant(&TenantId::new("tenant_hier_b"), 10, 0)
        .await
        .unwrap();
    assert_eq!(b_workspaces.len(), 1, "tenant B must have exactly 1 workspace");

    // Workspace A1: list projects — must return exactly 1.
    let a1_projects = projects
        .list_by_workspace(&TenantId::new("tenant_hier_a"), &WorkspaceId::new("ws_a1"), 10, 0)
        .await
        .unwrap();
    assert_eq!(a1_projects.len(), 1, "workspace A1 must have exactly 1 project");
    assert_eq!(a1_projects[0].project_id, ProjectId::new("proj_a1"));

    // Cross-tenant isolation: workspace B1 project must NOT appear under tenant A.
    let a2_projects = projects
        .list_by_workspace(&TenantId::new("tenant_hier_a"), &WorkspaceId::new("ws_a2"), 10, 0)
        .await
        .unwrap();
    assert_eq!(a2_projects.len(), 1);
    assert_eq!(a2_projects[0].project_id, ProjectId::new("proj_a2"),
        "tenant A workspace 2 must only contain its own project");
}

// ── Membership removal ────────────────────────────────────────────────────

#[tokio::test]
async fn remove_member_from_workspace() {
    let (_, tenants, workspaces, _, memberships) = services();

    tenants
        .create(TenantId::new("tenant_rm"), "Remove Tenant".to_owned())
        .await
        .unwrap();
    workspaces
        .create(
            TenantId::new("tenant_rm"),
            WorkspaceId::new("ws_rm"),
            "Remove Workspace".to_owned(),
        )
        .await
        .unwrap();

    let ws_key = WorkspaceKey::new("tenant_rm", "ws_rm");

    memberships
        .add_member(ws_key.clone(), "op_remove_me".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();

    let before = memberships.list_members(&ws_key).await.unwrap();
    assert_eq!(before.len(), 1);

    memberships
        .remove_member(ws_key.clone(), "op_remove_me".to_owned())
        .await
        .unwrap();

    let after = memberships.list_members(&ws_key).await.unwrap();
    assert!(after.is_empty(), "member must be removed from workspace");
}
