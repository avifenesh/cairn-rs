//! RFC 008 — Workspace role hierarchy enforcement tests.
//!
//! Validates the complete role-check stack end-to-end:
//!
//! - `WorkspaceRole::has_at_least` correctly gates every role boundary.
//! - `WorkspaceRole::level` produces a total strict ordering.
//! - `WorkspaceRole` serialises / deserialises losslessly.
//! - `Default` resolves to `Member`.
//! - Membership events (`WorkspaceMemberAdded`, `WorkspaceMemberRemoved`)
//!   round-trip through `InMemoryStore` and are exposed via the
//!   `WorkspaceMembershipReadModel` trait.
//! - Removing a member clears their record from the read-model.
//! - The RFC 008 gate (`has_at_least`) applied to read-model results enforces
//!   the correct access boundaries.

use cairn_domain::{
    EventEnvelope, EventId, EventSource, OperatorId, ProjectId, ProjectKey, TenantId,
    WorkspaceId, WorkspaceKey, WorkspaceRole, RuntimeEvent,
    events::{WorkspaceMemberAdded, WorkspaceMemberRemoved},
    tenancy::OwnershipKey,
};
use cairn_store::{
    projections::WorkspaceMembershipReadModel,
    EventLog, InMemoryStore,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

const TENANT_ID: &str  = "tenant_rfc008";
const WS_ID: &str      = "ws_rfc008";

fn workspace_key() -> WorkspaceKey {
    WorkspaceKey::new(TENANT_ID, WS_ID)
}

fn ownership() -> OwnershipKey {
    OwnershipKey::Workspace(workspace_key())
}

/// Append a `WorkspaceMemberAdded` event to the store.
async fn add_member(store: &InMemoryStore, event_id: &str, member: &str, role: WorkspaceRole) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
            workspace_key: workspace_key(),
            member_id: OperatorId::new(member),
            role,
            added_at_ms: 1_000,
        }),
    );
    store.append(&[env]).await.unwrap();
}

/// Append a `WorkspaceMemberRemoved` event to the store.
async fn remove_member(store: &InMemoryStore, event_id: &str, member: &str) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::WorkspaceMemberRemoved(WorkspaceMemberRemoved {
            workspace_key: workspace_key(),
            member_id: OperatorId::new(member),
            removed_at_ms: 2_000,
        }),
    );
    store.append(&[env]).await.unwrap();
}

/// Convenience: fetch a member's role from the read model (asserts present).
async fn role_of(store: &InMemoryStore, member: &str) -> WorkspaceRole {
    WorkspaceMembershipReadModel::get_member(store, &workspace_key(), member)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("member {member} not found in read model"))
        .role
}

// ── 1. Role hierarchy ordering via level() ────────────────────────────────────

#[test]
fn privilege_levels_are_strictly_ordered() {
    use WorkspaceRole::*;
    assert!(Viewer.level() < Member.level(), "Viewer < Member");
    assert!(Member.level() < Admin.level(),  "Member < Admin");
    assert!(Admin.level()  < Owner.level(),  "Admin  < Owner");
}

#[test]
fn privilege_levels_match_rfc_contract_values() {
    // RFC 008 specifies numeric levels; these must not drift silently.
    assert_eq!(WorkspaceRole::Viewer.level(), 1);
    assert_eq!(WorkspaceRole::Member.level(), 2);
    assert_eq!(WorkspaceRole::Admin.level(),  3);
    assert_eq!(WorkspaceRole::Owner.level(),  4);
}

// ── 2. has_at_least boundary conditions ──────────────────────────────────────

#[test]
fn has_at_least_viewer_is_true_for_all_roles() {
    use WorkspaceRole::*;
    for role in [Viewer, Member, Admin, Owner] {
        assert!(role.has_at_least(Viewer), "{role:?} should satisfy ≥ Viewer");
    }
}

#[test]
fn has_at_least_member_passes_for_member_admin_owner_only() {
    use WorkspaceRole::*;
    assert!(!Viewer.has_at_least(Member), "Viewer must NOT satisfy ≥ Member");
    assert!( Member.has_at_least(Member), "Member must satisfy ≥ Member");
    assert!( Admin.has_at_least(Member),  "Admin must satisfy ≥ Member");
    assert!( Owner.has_at_least(Member),  "Owner must satisfy ≥ Member");
}

#[test]
fn has_at_least_admin_returns_true_for_owner_and_admin_only() {
    use WorkspaceRole::*;
    assert!(!Viewer.has_at_least(Admin), "Viewer must NOT satisfy ≥ Admin");
    assert!(!Member.has_at_least(Admin), "Member must NOT satisfy ≥ Admin");
    assert!( Admin.has_at_least(Admin),  "Admin  must satisfy ≥ Admin");
    assert!( Owner.has_at_least(Admin),  "Owner  must satisfy ≥ Admin");
}

#[test]
fn has_at_least_owner_is_true_only_for_owner() {
    use WorkspaceRole::*;
    assert!(!Viewer.has_at_least(Owner), "Viewer must NOT satisfy ≥ Owner");
    assert!(!Member.has_at_least(Owner), "Member must NOT satisfy ≥ Owner");
    assert!(!Admin.has_at_least(Owner),  "Admin  must NOT satisfy ≥ Owner");
    assert!( Owner.has_at_least(Owner),  "Owner  must satisfy ≥ Owner");
}

#[test]
fn every_role_satisfies_itself() {
    use WorkspaceRole::*;
    for role in [Viewer, Member, Admin, Owner] {
        assert!(role.has_at_least(role), "{role:?} must satisfy itself");
    }
}

// ── 3. Default is Member ──────────────────────────────────────────────────────

#[test]
fn default_workspace_role_is_member() {
    assert_eq!(WorkspaceRole::default(), WorkspaceRole::Member);
}

// ── 4. Serde round-trip ───────────────────────────────────────────────────────

#[test]
fn workspace_role_serde_round_trips_all_variants() {
    use WorkspaceRole::*;
    for role in [Viewer, Member, Admin, Owner] {
        let json = serde_json::to_string(&role).unwrap();
        let back: WorkspaceRole = serde_json::from_str(&json).unwrap();
        assert_eq!(back, role, "serde round-trip failed for {role:?}");
    }
}

#[test]
fn workspace_role_serialises_to_snake_case_strings() {
    assert_eq!(serde_json::to_string(&WorkspaceRole::Viewer).unwrap(), r#""viewer""#);
    assert_eq!(serde_json::to_string(&WorkspaceRole::Member).unwrap(), r#""member""#);
    assert_eq!(serde_json::to_string(&WorkspaceRole::Admin).unwrap(),  r#""admin""#);
    assert_eq!(serde_json::to_string(&WorkspaceRole::Owner).unwrap(),  r#""owner""#);
}

#[test]
fn workspace_role_deserialises_from_snake_case_strings() {
    let o: WorkspaceRole = serde_json::from_str(r#""owner""#).unwrap();
    assert_eq!(o, WorkspaceRole::Owner);
    let v: WorkspaceRole = serde_json::from_str(r#""viewer""#).unwrap();
    assert_eq!(v, WorkspaceRole::Viewer);
    let a: WorkspaceRole = serde_json::from_str(r#""admin""#).unwrap();
    assert_eq!(a, WorkspaceRole::Admin);
    let m: WorkspaceRole = serde_json::from_str(r#""member""#).unwrap();
    assert_eq!(m, WorkspaceRole::Member);
}

// ── 5. Add 4 members with distinct roles ─────────────────────────────────────

#[tokio::test]
async fn four_members_with_owner_admin_member_viewer_all_stored() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_owner",  WorkspaceRole::Owner).await;
    add_member(&store, "e2", "op_admin",  WorkspaceRole::Admin).await;
    add_member(&store, "e3", "op_member", WorkspaceRole::Member).await;
    add_member(&store, "e4", "op_viewer", WorkspaceRole::Viewer).await;

    let members = WorkspaceMembershipReadModel::list_workspace_members(
        &store, WS_ID,
    )
    .await
    .unwrap();

    assert_eq!(members.len(), 4, "all 4 members must be in the read model");
    let roles: Vec<WorkspaceRole> = members.iter().map(|m| m.role).collect();
    assert!(roles.contains(&WorkspaceRole::Owner));
    assert!(roles.contains(&WorkspaceRole::Admin));
    assert!(roles.contains(&WorkspaceRole::Member));
    assert!(roles.contains(&WorkspaceRole::Viewer));
}

#[tokio::test]
async fn get_member_returns_correct_role_per_operator() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_owner",  WorkspaceRole::Owner).await;
    add_member(&store, "e2", "op_admin",  WorkspaceRole::Admin).await;
    add_member(&store, "e3", "op_member", WorkspaceRole::Member).await;
    add_member(&store, "e4", "op_viewer", WorkspaceRole::Viewer).await;

    assert_eq!(role_of(&store, "op_owner").await,  WorkspaceRole::Owner);
    assert_eq!(role_of(&store, "op_admin").await,  WorkspaceRole::Admin);
    assert_eq!(role_of(&store, "op_member").await, WorkspaceRole::Member);
    assert_eq!(role_of(&store, "op_viewer").await, WorkspaceRole::Viewer);
}

// ── 6. has_at_least applied via read-model ────────────────────────────────────

#[tokio::test]
async fn owner_and_admin_satisfy_admin_minimum_from_read_model() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_owner",  WorkspaceRole::Owner).await;
    add_member(&store, "e2", "op_admin",  WorkspaceRole::Admin).await;
    add_member(&store, "e3", "op_member", WorkspaceRole::Member).await;
    add_member(&store, "e4", "op_viewer", WorkspaceRole::Viewer).await;

    // Owner and Admin pass the Admin gate.
    assert!(role_of(&store, "op_owner").await.has_at_least(WorkspaceRole::Admin),
            "Owner ≥ Admin");
    assert!(role_of(&store, "op_admin").await.has_at_least(WorkspaceRole::Admin),
            "Admin ≥ Admin");

    // Member and Viewer do not.
    assert!(!role_of(&store, "op_member").await.has_at_least(WorkspaceRole::Admin),
            "Member < Admin");
    assert!(!role_of(&store, "op_viewer").await.has_at_least(WorkspaceRole::Admin),
            "Viewer < Admin");
}

#[tokio::test]
async fn only_owner_satisfies_owner_minimum_from_read_model() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_owner",  WorkspaceRole::Owner).await;
    add_member(&store, "e2", "op_admin",  WorkspaceRole::Admin).await;
    add_member(&store, "e3", "op_member", WorkspaceRole::Member).await;
    add_member(&store, "e4", "op_viewer", WorkspaceRole::Viewer).await;

    assert!( role_of(&store, "op_owner").await.has_at_least(WorkspaceRole::Owner));
    assert!(!role_of(&store, "op_admin").await.has_at_least(WorkspaceRole::Owner));
    assert!(!role_of(&store, "op_member").await.has_at_least(WorkspaceRole::Owner));
    assert!(!role_of(&store, "op_viewer").await.has_at_least(WorkspaceRole::Owner));
}

#[tokio::test]
async fn all_four_members_satisfy_viewer_minimum() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_owner",  WorkspaceRole::Owner).await;
    add_member(&store, "e2", "op_admin",  WorkspaceRole::Admin).await;
    add_member(&store, "e3", "op_member", WorkspaceRole::Member).await;
    add_member(&store, "e4", "op_viewer", WorkspaceRole::Viewer).await;

    let members = WorkspaceMembershipReadModel::list_workspace_members(&store, WS_ID)
        .await
        .unwrap();
    for m in &members {
        assert!(
            m.role.has_at_least(WorkspaceRole::Viewer),
            "{:?} must satisfy ≥ Viewer",
            m.role
        );
    }
}

// ── 7. Removing a member clears their access ─────────────────────────────────

#[tokio::test]
async fn removing_member_returns_none_from_read_model() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_owner",  WorkspaceRole::Owner).await;
    add_member(&store, "e2", "op_member", WorkspaceRole::Member).await;

    // Confirm the member exists before removal.
    let before = WorkspaceMembershipReadModel::get_member(&store, &workspace_key(), "op_member")
        .await.unwrap();
    assert!(before.is_some(), "op_member should exist before removal");

    remove_member(&store, "e3", "op_member").await;

    // Read-model must reflect the removal.
    let after = WorkspaceMembershipReadModel::get_member(&store, &workspace_key(), "op_member")
        .await.unwrap();
    assert!(after.is_none(), "op_member must be absent after removal");
}

#[tokio::test]
async fn removing_one_member_does_not_affect_others() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_owner",  WorkspaceRole::Owner).await;
    add_member(&store, "e2", "op_admin",  WorkspaceRole::Admin).await;
    add_member(&store, "e3", "op_member", WorkspaceRole::Member).await;
    add_member(&store, "e4", "op_viewer", WorkspaceRole::Viewer).await;

    remove_member(&store, "e5", "op_member").await;

    let remaining = WorkspaceMembershipReadModel::list_workspace_members(&store, WS_ID)
        .await.unwrap();
    assert_eq!(remaining.len(), 3, "3 members should remain");

    let ids: Vec<&str> = remaining.iter().map(|m| m.operator_id.as_str()).collect();
    assert!(!ids.contains(&"op_member"), "op_member must be gone");
    assert!(ids.contains(&"op_owner"),   "op_owner must remain");
    assert!(ids.contains(&"op_admin"),   "op_admin must remain");
    assert!(ids.contains(&"op_viewer"),  "op_viewer must remain");
}

#[tokio::test]
async fn removed_admin_fails_admin_gate() {
    let store = InMemoryStore::new();

    add_member(&store, "e1", "op_admin", WorkspaceRole::Admin).await;

    // Confirm access before removal.
    let role_before = role_of(&store, "op_admin").await;
    assert!(role_before.has_at_least(WorkspaceRole::Admin), "should pass gate before removal");

    remove_member(&store, "e2", "op_admin").await;

    // After removal: no record → RFC 008 gate must deny.
    let record = WorkspaceMembershipReadModel::get_member(&store, &workspace_key(), "op_admin")
        .await.unwrap();
    assert!(record.is_none(), "removed member must have no record");

    let passes = record.map(|m| m.role.has_at_least(WorkspaceRole::Admin)).unwrap_or(false);
    assert!(!passes, "removed member must not pass the admin gate");
}

// ── 8. Fresh workspace has no members ────────────────────────────────────────

#[tokio::test]
async fn workspace_with_no_events_has_empty_member_list() {
    let store = InMemoryStore::new();
    let members = WorkspaceMembershipReadModel::list_workspace_members(&store, WS_ID)
        .await.unwrap();
    assert!(members.is_empty());
}

#[tokio::test]
async fn get_member_returns_none_for_unknown_operator() {
    let store = InMemoryStore::new();
    let record = WorkspaceMembershipReadModel::get_member(
        &store, &workspace_key(), "no_such_op",
    )
    .await.unwrap();
    assert!(record.is_none());
}

// ── 9. Events are written to the event log ────────────────────────────────────

#[tokio::test]
async fn add_member_persists_workspace_member_added_event() {
    let store = InMemoryStore::new();
    add_member(&store, "e1", "op_evt", WorkspaceRole::Admin).await;

    let all = store.read_stream(None, 100).await.unwrap();
    let has_added = all.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::WorkspaceMemberAdded(ev)
            if ev.member_id.as_str() == "op_evt")
    });
    assert!(has_added, "WorkspaceMemberAdded event must be in the log");
}

#[tokio::test]
async fn remove_member_persists_workspace_member_removed_event() {
    let store = InMemoryStore::new();
    add_member(&store, "e1", "op_rem", WorkspaceRole::Viewer).await;
    remove_member(&store, "e2", "op_rem").await;

    let all = store.read_stream(None, 100).await.unwrap();
    let has_removed = all.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::WorkspaceMemberRemoved(ev)
            if ev.member_id.as_str() == "op_rem")
    });
    assert!(has_removed, "WorkspaceMemberRemoved event must be in the log");
}

#[tokio::test]
async fn event_log_positions_increase_monotonically_across_membership_ops() {
    let store = InMemoryStore::new();

    add_member(&store,    "e1", "op_a", WorkspaceRole::Owner).await;
    add_member(&store,    "e2", "op_b", WorkspaceRole::Member).await;
    remove_member(&store, "e3", "op_b").await;
    add_member(&store,    "e4", "op_c", WorkspaceRole::Admin).await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 4);
    for w in all.windows(2) {
        assert!(w[0].position < w[1].position, "positions must increase");
    }
}
