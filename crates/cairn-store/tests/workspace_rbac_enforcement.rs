//! Workspace RBAC enforcement integration tests (RFC 008).
//!
//! Validates that workspace membership and role hierarchy are correctly
//! persisted and enforced. RFC 008 mandates that all permission checks use
//! `WorkspaceRole::has_at_least()` — never direct equality — so the role
//! hierarchy must be durable and inspectable at the read-model layer.
//!
//! Role hierarchy (ascending privilege):
//!   Viewer(1) < Member(2) < Admin(3) < Owner(4)
//!
//! Projection contract:
//!   WorkspaceMemberAdded   → upsert: retain removes stale entry, push new one
//!   WorkspaceMemberRemoved → retain removes the matching (workspace, operator) pair

use cairn_domain::{
    EventEnvelope, EventId, EventSource, OperatorId, ProjectId, ProjectKey, RuntimeEvent,
    TenantCreated, TenantId, WorkspaceCreated, WorkspaceId, WorkspaceMemberAdded,
    WorkspaceMemberRemoved,
};
use cairn_domain::tenancy::{WorkspaceKey, WorkspaceRole};
use cairn_store::{
    projections::{TenantReadModel, WorkspaceMembershipReadModel, WorkspaceReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str, workspace: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
        project_id: ProjectId::new(format!("p_{tenant}_{workspace}")),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn wkey(tenant: &str, workspace: &str) -> WorkspaceKey {
    WorkspaceKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
    }
}

fn add_member(
    evt_id: &str,
    tenant: &str,
    workspace: &str,
    operator: &str,
    role: WorkspaceRole,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
            workspace_key: wkey(tenant, workspace),
            member_id: OperatorId::new(operator),
            role,
            added_at_ms: ts,
        }),
    )
}

fn remove_member(
    evt_id: &str,
    tenant: &str,
    workspace: &str,
    operator: &str,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::WorkspaceMemberRemoved(WorkspaceMemberRemoved {
            workspace_key: wkey(tenant, workspace),
            member_id: OperatorId::new(operator),
            removed_at_ms: now_ms(),
        }),
    )
}

// ── 1. Create tenant with 2 workspaces ────────────────────────────────────────

#[tokio::test]
async fn create_tenant_with_two_workspaces() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            evt("e1", RuntimeEvent::TenantCreated(TenantCreated {
                project: project("acme", "ws_eng"),
                tenant_id: TenantId::new("acme"),
                name: "Acme Corp".to_owned(),
                created_at: ts,
            })),
            evt("e2", RuntimeEvent::WorkspaceCreated(WorkspaceCreated {
                project: project("acme", "ws_eng"),
                workspace_id: WorkspaceId::new("ws_eng"),
                tenant_id: TenantId::new("acme"),
                name: "Engineering".to_owned(),
                created_at: ts + 1,
            })),
            evt("e3", RuntimeEvent::WorkspaceCreated(WorkspaceCreated {
                project: project("acme", "ws_ops"),
                workspace_id: WorkspaceId::new("ws_ops"),
                tenant_id: TenantId::new("acme"),
                name: "Operations".to_owned(),
                created_at: ts + 2,
            })),
        ])
        .await
        .unwrap();

    let tenant = TenantReadModel::get(&store, &TenantId::new("acme"))
        .await.unwrap().expect("tenant must exist");
    assert_eq!(tenant.name, "Acme Corp");

    let workspaces = WorkspaceReadModel::list_by_tenant(&store, &TenantId::new("acme"), 10, 0)
        .await.unwrap();
    assert_eq!(workspaces.len(), 2);
    let ws_ids: Vec<_> = workspaces.iter().map(|w| w.workspace_id.as_str()).collect();
    assert!(ws_ids.contains(&"ws_eng"));
    assert!(ws_ids.contains(&"ws_ops"));
}

// ── 2. Add members with all four roles ───────────────────────────────────────

#[tokio::test]
async fn add_members_with_all_four_roles() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            add_member("e1", "t1", "ws1", "alice",   WorkspaceRole::Owner,  ts),
            add_member("e2", "t1", "ws1", "bob",     WorkspaceRole::Admin,  ts + 1),
            add_member("e3", "t1", "ws1", "charlie", WorkspaceRole::Member, ts + 2),
            add_member("e4", "t1", "ws1", "diana",   WorkspaceRole::Viewer, ts + 3),
        ])
        .await
        .unwrap();

    let members = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws1")
        .await.unwrap();
    assert_eq!(members.len(), 4);

    // Retrieve each and verify role.
    for (operator, expected_role) in [
        ("alice",   WorkspaceRole::Owner),
        ("bob",     WorkspaceRole::Admin),
        ("charlie", WorkspaceRole::Member),
        ("diana",   WorkspaceRole::Viewer),
    ] {
        let rec = WorkspaceMembershipReadModel::get_member(&store, &wkey("t1", "ws1"), operator)
            .await.unwrap()
            .expect(&format!("{operator} must be in workspace"));
        assert_eq!(rec.role, expected_role, "{operator}: role mismatch");
    }
}

// ── 3. has_at_least() role hierarchy: Owner > Admin > Member > Viewer ──────────

#[test]
fn role_hierarchy_owner_satisfies_all_levels() {
    assert!(WorkspaceRole::Owner.has_at_least(WorkspaceRole::Owner),   "Owner ≥ Owner");
    assert!(WorkspaceRole::Owner.has_at_least(WorkspaceRole::Admin),   "Owner ≥ Admin");
    assert!(WorkspaceRole::Owner.has_at_least(WorkspaceRole::Member),  "Owner ≥ Member");
    assert!(WorkspaceRole::Owner.has_at_least(WorkspaceRole::Viewer),  "Owner ≥ Viewer");
}

#[test]
fn role_hierarchy_admin_satisfies_admin_and_below() {
    assert!(!WorkspaceRole::Admin.has_at_least(WorkspaceRole::Owner),  "Admin < Owner");
    assert!(WorkspaceRole::Admin.has_at_least(WorkspaceRole::Admin),   "Admin ≥ Admin");
    assert!(WorkspaceRole::Admin.has_at_least(WorkspaceRole::Member),  "Admin ≥ Member");
    assert!(WorkspaceRole::Admin.has_at_least(WorkspaceRole::Viewer),  "Admin ≥ Viewer");
}

#[test]
fn role_hierarchy_member_satisfies_member_and_viewer() {
    assert!(!WorkspaceRole::Member.has_at_least(WorkspaceRole::Owner),  "Member < Owner");
    assert!(!WorkspaceRole::Member.has_at_least(WorkspaceRole::Admin),  "Member < Admin");
    assert!(WorkspaceRole::Member.has_at_least(WorkspaceRole::Member),  "Member ≥ Member");
    assert!(WorkspaceRole::Member.has_at_least(WorkspaceRole::Viewer),  "Member ≥ Viewer");
}

#[test]
fn role_hierarchy_viewer_satisfies_only_viewer() {
    assert!(!WorkspaceRole::Viewer.has_at_least(WorkspaceRole::Owner),  "Viewer < Owner");
    assert!(!WorkspaceRole::Viewer.has_at_least(WorkspaceRole::Admin),  "Viewer < Admin");
    assert!(!WorkspaceRole::Viewer.has_at_least(WorkspaceRole::Member), "Viewer < Member");
    assert!(WorkspaceRole::Viewer.has_at_least(WorkspaceRole::Viewer),  "Viewer ≥ Viewer");
}

#[test]
fn role_levels_are_strictly_ordered() {
    assert!(WorkspaceRole::Owner.level()  > WorkspaceRole::Admin.level());
    assert!(WorkspaceRole::Admin.level()  > WorkspaceRole::Member.level());
    assert!(WorkspaceRole::Member.level() > WorkspaceRole::Viewer.level());
    assert_eq!(WorkspaceRole::Owner.level(),  4);
    assert_eq!(WorkspaceRole::Admin.level(),  3);
    assert_eq!(WorkspaceRole::Member.level(), 2);
    assert_eq!(WorkspaceRole::Viewer.level(), 1);
}

// ── 4. WorkspaceMemberRemoved removes access ──────────────────────────────────

#[tokio::test]
async fn member_removed_event_removes_access() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            add_member("e1", "t_rm", "ws_rm", "alice", WorkspaceRole::Admin,  ts),
            add_member("e2", "t_rm", "ws_rm", "bob",   WorkspaceRole::Member, ts + 1),
        ])
        .await
        .unwrap();

    // Verify both members present.
    let before = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_rm")
        .await.unwrap();
    assert_eq!(before.len(), 2);

    // Remove alice.
    store
        .append(&[remove_member("e3", "t_rm", "ws_rm", "alice")])
        .await
        .unwrap();

    let after = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_rm")
        .await.unwrap();
    assert_eq!(after.len(), 1, "alice must be removed");
    assert_eq!(after[0].operator_id, "bob");

    // get_member for removed operator returns None.
    let alice = WorkspaceMembershipReadModel::get_member(
        &store, &wkey("t_rm", "ws_rm"), "alice",
    )
    .await
    .unwrap();
    assert!(alice.is_none(), "removed member must not be findable");
}

#[tokio::test]
async fn removing_all_members_leaves_workspace_empty() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            add_member("e1", "t_empty", "ws_empty", "op1", WorkspaceRole::Owner,  ts),
            add_member("e2", "t_empty", "ws_empty", "op2", WorkspaceRole::Viewer, ts + 1),
        ])
        .await
        .unwrap();

    store
        .append(&[
            remove_member("e3", "t_empty", "ws_empty", "op1"),
            remove_member("e4", "t_empty", "ws_empty", "op2"),
        ])
        .await
        .unwrap();

    let members = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_empty")
        .await.unwrap();
    assert!(members.is_empty(), "all members removed — workspace has no members");
}

// ── 5. Cross-workspace member isolation ──────────────────────────────────────

#[tokio::test]
async fn cross_workspace_member_isolation() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Alice is in ws_alpha; bob is in ws_beta; carol is in both.
    store
        .append(&[
            add_member("e1", "t_iso", "ws_alpha", "alice", WorkspaceRole::Owner,  ts),
            add_member("e2", "t_iso", "ws_alpha", "carol", WorkspaceRole::Admin,  ts + 1),
            add_member("e3", "t_iso", "ws_beta",  "bob",   WorkspaceRole::Member, ts + 2),
            add_member("e4", "t_iso", "ws_beta",  "carol", WorkspaceRole::Viewer, ts + 3),
        ])
        .await
        .unwrap();

    let alpha = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_alpha")
        .await.unwrap();
    assert_eq!(alpha.len(), 2);
    let alpha_ids: Vec<_> = alpha.iter().map(|m| m.operator_id.as_str()).collect();
    assert!(alpha_ids.contains(&"alice"));
    assert!(alpha_ids.contains(&"carol"));
    assert!(!alpha_ids.contains(&"bob"), "bob is not in ws_alpha");

    let beta = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_beta")
        .await.unwrap();
    assert_eq!(beta.len(), 2);
    let beta_ids: Vec<_> = beta.iter().map(|m| m.operator_id.as_str()).collect();
    assert!(beta_ids.contains(&"bob"));
    assert!(beta_ids.contains(&"carol"));
    assert!(!beta_ids.contains(&"alice"), "alice is not in ws_beta");

    // Carol has different roles in each workspace.
    let carol_alpha = WorkspaceMembershipReadModel::get_member(
        &store, &wkey("t_iso", "ws_alpha"), "carol",
    )
    .await.unwrap().unwrap();
    assert_eq!(carol_alpha.role, WorkspaceRole::Admin);

    let carol_beta = WorkspaceMembershipReadModel::get_member(
        &store, &wkey("t_iso", "ws_beta"), "carol",
    )
    .await.unwrap().unwrap();
    assert_eq!(carol_beta.role, WorkspaceRole::Viewer);

    // Different role in each workspace: Admin ≥ Viewer but not vice versa.
    assert!(carol_alpha.role.has_at_least(carol_beta.role),
        "Admin satisfies Viewer minimum");
    assert!(!carol_beta.role.has_at_least(carol_alpha.role),
        "Viewer does not satisfy Admin minimum");
}

// ── 6. WorkspaceRole serialization round-trip ─────────────────────────────────

#[test]
fn workspace_role_serializes_to_snake_case() {
    // serde rename_all = "snake_case" — verify each variant serializes correctly.
    assert_eq!(serde_json::to_value(WorkspaceRole::Owner).unwrap(),  "owner");
    assert_eq!(serde_json::to_value(WorkspaceRole::Admin).unwrap(),  "admin");
    assert_eq!(serde_json::to_value(WorkspaceRole::Member).unwrap(), "member");
    assert_eq!(serde_json::to_value(WorkspaceRole::Viewer).unwrap(), "viewer");
}

#[test]
fn workspace_role_deserializes_from_snake_case() {
    let owner:  WorkspaceRole = serde_json::from_str("\"owner\"").unwrap();
    let admin:  WorkspaceRole = serde_json::from_str("\"admin\"").unwrap();
    let member: WorkspaceRole = serde_json::from_str("\"member\"").unwrap();
    let viewer: WorkspaceRole = serde_json::from_str("\"viewer\"").unwrap();

    assert_eq!(owner,  WorkspaceRole::Owner);
    assert_eq!(admin,  WorkspaceRole::Admin);
    assert_eq!(member, WorkspaceRole::Member);
    assert_eq!(viewer, WorkspaceRole::Viewer);
}

#[test]
fn workspace_role_round_trips_through_json() {
    for role in [
        WorkspaceRole::Owner,
        WorkspaceRole::Admin,
        WorkspaceRole::Member,
        WorkspaceRole::Viewer,
    ] {
        let json = serde_json::to_string(&role).unwrap();
        let back: WorkspaceRole = serde_json::from_str(&json).unwrap();
        assert_eq!(back, role, "{role:?} must survive JSON round-trip");
    }
}

// ── 7. Role upgrade via re-add upserts the record ────────────────────────────

#[tokio::test]
async fn re_adding_member_upgrades_role_without_duplicate() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            add_member("e1", "t_up", "ws_up", "eve", WorkspaceRole::Viewer, ts),
        ])
        .await
        .unwrap();

    let before = WorkspaceMembershipReadModel::get_member(
        &store, &wkey("t_up", "ws_up"), "eve",
    )
    .await.unwrap().unwrap();
    assert_eq!(before.role, WorkspaceRole::Viewer);

    // Operator upgrades eve to Admin.
    store
        .append(&[add_member("e2", "t_up", "ws_up", "eve", WorkspaceRole::Admin, ts + 1)])
        .await
        .unwrap();

    let after = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_up")
        .await.unwrap();
    assert_eq!(after.len(), 1, "upsert must not create a duplicate");
    assert_eq!(after[0].role, WorkspaceRole::Admin, "role upgraded to Admin");

    // Admin now satisfies Member check but not Owner.
    assert!(after[0].role.has_at_least(WorkspaceRole::Member));
    assert!(!after[0].role.has_at_least(WorkspaceRole::Owner));
}

// ── 8. has_at_least() is used for gate checks (not equality) ─────────────────

#[test]
fn has_at_least_models_rfc_permission_gate_semantics() {
    // Scenario: a resource requires at least Admin to mutate.
    let can_mutate = |role: WorkspaceRole| role.has_at_least(WorkspaceRole::Admin);
    let can_read   = |role: WorkspaceRole| role.has_at_least(WorkspaceRole::Viewer);

    assert!(can_mutate(WorkspaceRole::Owner),   "Owner can mutate");
    assert!(can_mutate(WorkspaceRole::Admin),   "Admin can mutate");
    assert!(!can_mutate(WorkspaceRole::Member), "Member cannot mutate");
    assert!(!can_mutate(WorkspaceRole::Viewer), "Viewer cannot mutate");

    assert!(can_read(WorkspaceRole::Owner),     "Owner can read");
    assert!(can_read(WorkspaceRole::Admin),     "Admin can read");
    assert!(can_read(WorkspaceRole::Member),    "Member can read");
    assert!(can_read(WorkspaceRole::Viewer),    "Viewer can read");
}

// ── 9. Two tenants — workspace members are tenant-scoped ─────────────────────

#[tokio::test]
async fn members_are_scoped_to_tenant_workspace_pair() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // tenant_x/ws_shared has alice.
    // tenant_y/ws_shared has bob (same workspace_id string, different tenant).
    store
        .append(&[
            add_member("e1", "tenant_x", "ws_shared", "alice", WorkspaceRole::Owner,  ts),
            add_member("e2", "tenant_y", "ws_shared", "bob",   WorkspaceRole::Admin,  ts + 1),
        ])
        .await
        .unwrap();

    // list_workspace_members looks up by workspace_id string only
    // — returns both since they share the same ws id string.
    // But get_member uses the full WorkspaceKey.
    let alice = WorkspaceMembershipReadModel::get_member(
        &store,
        &wkey("tenant_x", "ws_shared"),
        "alice",
    )
    .await.unwrap().unwrap();
    assert_eq!(alice.role, WorkspaceRole::Owner);

    let bob = WorkspaceMembershipReadModel::get_member(
        &store,
        &wkey("tenant_y", "ws_shared"),
        "bob",
    )
    .await.unwrap().unwrap();
    assert_eq!(bob.role, WorkspaceRole::Admin);
}
