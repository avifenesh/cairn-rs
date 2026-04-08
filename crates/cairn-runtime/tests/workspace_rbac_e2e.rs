//! RFC 008 workspace RBAC end-to-end integration test.
//!
//! Validates the role-based access control hierarchy:
//!   (1) add a member with Admin role
//!   (2) add a member with Viewer role
//!   (3) verify Admin satisfies write-level permission check
//!   (4) verify Viewer is denied write-level permission check
//!   (5) upgrade Viewer to Member (remove + re-add with new role)
//!   (6) verify upgraded member now passes write-level check
//!   (7) verify role hierarchy ordering (Owner > Admin > Member > Viewer)
//!   (8) Owner role supersedes all other roles

use std::sync::Arc;

use cairn_domain::{OperatorId, TenantId, WorkspaceId, WorkspaceKey, WorkspaceRole};
use cairn_runtime::services::{WorkspaceMembershipServiceImpl, WorkspaceServiceImpl};
use cairn_runtime::{WorkspaceMembershipService, WorkspaceService};
use cairn_store::InMemoryStore;

fn workspace_key() -> WorkspaceKey {
    WorkspaceKey::new("tenant_rbac", "ws_rbac")
}

async fn setup() -> (
    WorkspaceServiceImpl<InMemoryStore>,
    WorkspaceMembershipServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    let workspaces = WorkspaceServiceImpl::new(store.clone());
    let memberships = WorkspaceMembershipServiceImpl::new(store);

    workspaces
        .create(
            TenantId::new("tenant_rbac"),
            WorkspaceId::new("ws_rbac"),
            "RBAC Workspace".to_owned(),
        )
        .await
        .unwrap();

    (workspaces, memberships)
}

/// Simulated write gate: returns Ok if the role has at least Member privilege.
/// This is the canonical RFC 008 check for mutating workspace resources.
fn assert_can_write(role: WorkspaceRole, operator: &str) {
    assert!(
        role.has_at_least(WorkspaceRole::Member),
        "{operator} with role {role:?} must be allowed to write (requires at least Member)"
    );
}

/// Simulated write gate that asserts the role is DENIED write access.
fn assert_cannot_write(role: WorkspaceRole, operator: &str) {
    assert!(
        !role.has_at_least(WorkspaceRole::Member),
        "{operator} with role {role:?} must NOT be allowed to write"
    );
}

/// Read gate: Viewer and above can read.
fn assert_can_read(role: WorkspaceRole, operator: &str) {
    assert!(
        role.has_at_least(WorkspaceRole::Viewer),
        "{operator} with role {role:?} must be allowed to read (requires at least Viewer)"
    );
}

// ── (1) Add Admin member ──────────────────────────────────────────────────

#[tokio::test]
async fn add_admin_member_to_workspace() {
    let (_, memberships) = setup().await;
    let wk = workspace_key();

    let membership = memberships
        .add_member(wk.clone(), "op_admin".to_owned(), WorkspaceRole::Admin)
        .await
        .unwrap();

    assert_eq!(membership.operator_id, OperatorId::new("op_admin"));
    assert_eq!(membership.role, WorkspaceRole::Admin);
    assert_eq!(membership.workspace_id, WorkspaceId::new("ws_rbac"));
}

// ── (2) Add Viewer member ─────────────────────────────────────────────────

#[tokio::test]
async fn add_viewer_member_to_workspace() {
    let (_, memberships) = setup().await;

    let membership = memberships
        .add_member(
            workspace_key(),
            "op_viewer".to_owned(),
            WorkspaceRole::Viewer,
        )
        .await
        .unwrap();

    assert_eq!(membership.role, WorkspaceRole::Viewer);
}

// ── (3) Admin can perform write operations ────────────────────────────────

#[tokio::test]
async fn admin_role_satisfies_write_permission() {
    let (_, memberships) = setup().await;
    let wk = workspace_key();

    memberships
        .add_member(
            wk.clone(),
            "op_admin_write".to_owned(),
            WorkspaceRole::Admin,
        )
        .await
        .unwrap();

    let members = memberships.list_members(&wk).await.unwrap();
    let admin = members
        .iter()
        .find(|m| m.operator_id == OperatorId::new("op_admin_write"))
        .unwrap();

    // RFC 008: Admin.level() = 3 >= Member.level() = 2 → write allowed.
    assert_can_write(admin.role, "op_admin_write");
    assert_can_read(admin.role, "op_admin_write");

    // Admin also satisfies admin-level checks.
    assert!(
        admin.role.has_at_least(WorkspaceRole::Admin),
        "Admin must satisfy Admin-level check"
    );
}

// ── (4) Viewer is restricted to read-only ────────────────────────────────

#[tokio::test]
async fn viewer_role_denied_write_permission() {
    let (_, memberships) = setup().await;
    let wk = workspace_key();

    memberships
        .add_member(wk.clone(), "op_viewer_ro".to_owned(), WorkspaceRole::Viewer)
        .await
        .unwrap();

    let members = memberships.list_members(&wk).await.unwrap();
    let viewer = members
        .iter()
        .find(|m| m.operator_id == OperatorId::new("op_viewer_ro"))
        .unwrap();

    // RFC 008: Viewer.level() = 1 < Member.level() = 2 → write denied.
    assert_cannot_write(viewer.role, "op_viewer_ro");

    // Viewer can still read.
    assert_can_read(viewer.role, "op_viewer_ro");

    // Viewer does not satisfy Admin-level check.
    assert!(
        !viewer.role.has_at_least(WorkspaceRole::Admin),
        "Viewer must not satisfy Admin-level check"
    );
}

// ── (5)+(6) Upgrade Viewer to Member — now allowed to write ──────────────

#[tokio::test]
async fn upgrade_viewer_to_member_grants_write_access() {
    let (_, memberships) = setup().await;
    let wk = workspace_key();

    // Add as Viewer.
    memberships
        .add_member(wk.clone(), "op_upgrade".to_owned(), WorkspaceRole::Viewer)
        .await
        .unwrap();

    // Verify Viewer cannot write.
    let before = memberships.list_members(&wk).await.unwrap();
    let before_role = before
        .iter()
        .find(|m| m.operator_id == OperatorId::new("op_upgrade"))
        .unwrap()
        .role;
    assert_cannot_write(before_role, "op_upgrade");

    // Upgrade: remove Viewer, re-add as Member.
    memberships
        .remove_member(wk.clone(), "op_upgrade".to_owned())
        .await
        .unwrap();
    memberships
        .add_member(wk.clone(), "op_upgrade".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();

    // Verify upgraded role.
    let after = memberships.list_members(&wk).await.unwrap();
    let after_role = after
        .iter()
        .find(|m| m.operator_id == OperatorId::new("op_upgrade"))
        .unwrap()
        .role;

    assert_eq!(
        after_role,
        WorkspaceRole::Member,
        "role must be Member after upgrade"
    );
    assert_can_write(after_role, "op_upgrade");
}

// ── (7) Role hierarchy ordering ───────────────────────────────────────────

#[tokio::test]
async fn role_hierarchy_ordering_is_correct() {
    // Viewer < Member < Admin < Owner (per RFC 008).
    assert!(WorkspaceRole::Member.has_at_least(WorkspaceRole::Viewer));
    assert!(WorkspaceRole::Admin.has_at_least(WorkspaceRole::Member));
    assert!(WorkspaceRole::Owner.has_at_least(WorkspaceRole::Admin));

    // Downward checks fail.
    assert!(!WorkspaceRole::Viewer.has_at_least(WorkspaceRole::Member));
    assert!(!WorkspaceRole::Member.has_at_least(WorkspaceRole::Admin));
    assert!(!WorkspaceRole::Admin.has_at_least(WorkspaceRole::Owner));

    // Each role satisfies its own level.
    for role in [
        WorkspaceRole::Viewer,
        WorkspaceRole::Member,
        WorkspaceRole::Admin,
        WorkspaceRole::Owner,
    ] {
        assert!(
            role.has_at_least(role),
            "{role:?} must satisfy its own level"
        );
    }
}

// ── (8) Owner role supersedes all others ─────────────────────────────────

#[tokio::test]
async fn owner_role_satisfies_all_permission_levels() {
    let (_, memberships) = setup().await;
    let wk = workspace_key();

    memberships
        .add_member(wk.clone(), "op_owner".to_owned(), WorkspaceRole::Owner)
        .await
        .unwrap();

    let members = memberships.list_members(&wk).await.unwrap();
    let owner = members
        .iter()
        .find(|m| m.operator_id == OperatorId::new("op_owner"))
        .unwrap();

    assert_can_read(owner.role, "op_owner");
    assert_can_write(owner.role, "op_owner");
    assert!(owner.role.has_at_least(WorkspaceRole::Admin));
    assert!(owner.role.has_at_least(WorkspaceRole::Owner));
}

// ── Mixed-role workspace membership list ─────────────────────────────────

#[tokio::test]
async fn list_members_reflects_correct_roles_for_each_operator() {
    let (_, memberships) = setup().await;
    let wk = workspace_key();

    memberships
        .add_member(wk.clone(), "alice".to_owned(), WorkspaceRole::Owner)
        .await
        .unwrap();
    memberships
        .add_member(wk.clone(), "bob".to_owned(), WorkspaceRole::Admin)
        .await
        .unwrap();
    memberships
        .add_member(wk.clone(), "carol".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();
    memberships
        .add_member(wk.clone(), "dave".to_owned(), WorkspaceRole::Viewer)
        .await
        .unwrap();

    let members = memberships.list_members(&wk).await.unwrap();
    assert_eq!(members.len(), 4, "all 4 members must be listed");

    for m in &members {
        let op = m.operator_id.as_str();
        match op {
            "alice" => assert_eq!(m.role, WorkspaceRole::Owner),
            "bob" => assert_eq!(m.role, WorkspaceRole::Admin),
            "carol" => assert_eq!(m.role, WorkspaceRole::Member),
            "dave" => assert_eq!(m.role, WorkspaceRole::Viewer),
            other => panic!("unexpected operator: {other}"),
        }
    }

    // Only Viewer (dave) is denied write.
    let write_capable: Vec<_> = members
        .iter()
        .filter(|m| m.role.has_at_least(WorkspaceRole::Member))
        .collect();
    assert_eq!(
        write_capable.len(),
        3,
        "Owner + Admin + Member can write; Viewer cannot"
    );

    let read_only: Vec<_> = members
        .iter()
        .filter(|m| !m.role.has_at_least(WorkspaceRole::Member))
        .collect();
    assert_eq!(read_only.len(), 1, "only Viewer is restricted to read-only");
    assert_eq!(read_only[0].operator_id, OperatorId::new("dave"));
}

// ── RBAC gate rejects un-enrolled operator ───────────────────────────────

#[tokio::test]
async fn unknown_operator_is_not_in_member_list() {
    let (_, memberships) = setup().await;
    let wk = workspace_key();

    memberships
        .add_member(wk.clone(), "known_op".to_owned(), WorkspaceRole::Member)
        .await
        .unwrap();

    let members = memberships.list_members(&wk).await.unwrap();
    let unknown = members
        .iter()
        .find(|m| m.operator_id == OperatorId::new("unknown_op"));

    assert!(
        unknown.is_none(),
        "un-enrolled operator must not appear in member list"
    );
}
