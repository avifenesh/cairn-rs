//! Approval policy lifecycle tests (RFC 005).
//!
//! Validates that approval policies are correctly stored, scoped to their tenant,
//! and carry all governance fields required for the human-in-the-loop workflow.
//!
//! ApprovalPolicyRecord fields:
//!   policy_id              — stable unique identifier
//!   tenant_id              — tenant that owns this policy
//!   name                   — human-readable label
//!   required_approvers     — minimum approvals before a release is approved
//!   allowed_approver_roles — which WorkspaceRoles may approve
//!   auto_approve_after_ms  — optional TTL: approve automatically if no action taken
//!   auto_reject_after_ms   — optional TTL: reject automatically if no action taken
//!   attached_release_ids   — prompt releases governed by this policy
//!                           (initialised empty by ApprovalPolicyCreated;
//!                            updated by InMemoryStore::attach_release_to_policy)
//!
//! list_by_tenant sorts by policy_id string ascending.

use cairn_domain::tenancy::WorkspaceRole;
use cairn_domain::{
    ApprovalPolicyCreated, EventEnvelope, EventId, EventSource, ProjectId, ProjectKey,
    PromptReleaseId, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_store::{projections::ApprovalPolicyReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new("w_pol"),
        project_id: ProjectId::new(format!("p_{tenant}")),
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

fn create_policy(
    evt_id: &str,
    policy_id: &str,
    tenant: &str,
    name: &str,
    required_approvers: u32,
    roles: Vec<WorkspaceRole>,
    auto_approve_after_ms: Option<u64>,
    auto_reject_after_ms: Option<u64>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ApprovalPolicyCreated(ApprovalPolicyCreated {
            project: project(tenant),
            policy_id: policy_id.to_owned(),
            tenant_id: TenantId::new(tenant),
            name: name.to_owned(),
            required_approvers,
            allowed_approver_roles: roles,
            auto_approve_after_ms,
            auto_reject_after_ms,
            created_at_ms: ts,
        }),
    )
}

// ── 1. ApprovalPolicyCreated stores the record ────────────────────────────────

#[tokio::test]
async fn approval_policy_created_stores_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_policy(
            "e1",
            "policy_strict",
            "tenant_a",
            "Strict Review",
            2,
            vec![WorkspaceRole::Admin, WorkspaceRole::Owner],
            None,
            Some(86_400_000), // auto-reject after 24 h
            ts,
        )])
        .await
        .unwrap();

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_strict")
        .await
        .unwrap()
        .expect("policy must exist after ApprovalPolicyCreated");

    assert_eq!(policy.policy_id, "policy_strict");
    assert_eq!(policy.tenant_id.as_str(), "tenant_a");
    assert_eq!(policy.name, "Strict Review");
    assert_eq!(
        policy.required_approvers, 2,
        "required_approvers=2 must persist"
    );
}

// ── 2. auto_approve_after_ms and auto_reject_after_ms persist ─────────────────

#[tokio::test]
async fn timing_fields_persist_correctly() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_policy(
            "e1",
            "policy_timed",
            "tenant_b",
            "Timed Policy",
            1,
            vec![WorkspaceRole::Admin],
            Some(3_600_000),  // auto-approve after 1 h
            Some(86_400_000), // auto-reject after 24 h
            ts,
        )])
        .await
        .unwrap();

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_timed")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        policy.auto_approve_after_ms,
        Some(3_600_000),
        "auto_approve_after_ms must persist"
    );
    assert_eq!(
        policy.auto_reject_after_ms,
        Some(86_400_000),
        "auto_reject_after_ms must persist"
    );
}

#[tokio::test]
async fn none_timing_fields_persist_as_none() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_policy(
            "e1",
            "policy_manual",
            "tenant_c",
            "Manual Only",
            3,
            vec![WorkspaceRole::Owner],
            None, // no auto-approve
            None, // no auto-reject
            ts,
        )])
        .await
        .unwrap();

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_manual")
        .await
        .unwrap()
        .unwrap();

    assert!(
        policy.auto_approve_after_ms.is_none(),
        "no auto-approve timeout → None"
    );
    assert!(
        policy.auto_reject_after_ms.is_none(),
        "no auto-reject timeout → None"
    );
    assert_eq!(policy.required_approvers, 3);
}

// ── 3. allowed_approver_roles persists all roles ──────────────────────────────

#[tokio::test]
async fn allowed_approver_roles_persist_correctly() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_policy(
            "e1",
            "policy_roles",
            "tenant_d",
            "Roles Policy",
            2,
            vec![WorkspaceRole::Admin, WorkspaceRole::Owner],
            None,
            None,
            ts,
        )])
        .await
        .unwrap();

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_roles")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(policy.allowed_approver_roles.len(), 2);
    assert!(policy
        .allowed_approver_roles
        .contains(&WorkspaceRole::Admin));
    assert!(policy
        .allowed_approver_roles
        .contains(&WorkspaceRole::Owner));
    assert!(
        !policy
            .allowed_approver_roles
            .contains(&WorkspaceRole::Member),
        "Member not in allowed roles"
    );
}

// ── 4. attached_release_ids starts empty, can be updated ─────────────────────

#[tokio::test]
async fn attached_release_ids_starts_empty() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_policy(
            "e1",
            "policy_attach",
            "tenant_e",
            "Attach Test",
            1,
            vec![WorkspaceRole::Admin],
            None,
            None,
            ts,
        )])
        .await
        .unwrap();

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_attach")
        .await
        .unwrap()
        .unwrap();
    assert!(
        policy.attached_release_ids.is_empty(),
        "ApprovalPolicyCreated initialises attached_release_ids as empty"
    );
}

#[tokio::test]
async fn attached_release_ids_can_be_updated() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_policy(
            "e1",
            "policy_upd",
            "tenant_f",
            "Update Test",
            2,
            vec![WorkspaceRole::Admin, WorkspaceRole::Owner],
            None,
            None,
            ts,
        )])
        .await
        .unwrap();

    // Attach two releases.
    let ok1 = store.attach_release_to_policy("policy_upd", PromptReleaseId::new("rel_v1"));
    let ok2 = store.attach_release_to_policy("policy_upd", PromptReleaseId::new("rel_v2"));
    assert!(
        ok1,
        "attach_release_to_policy returns true for registered policy"
    );
    assert!(ok2);

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_upd")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.attached_release_ids.len(), 2);
    assert!(policy
        .attached_release_ids
        .contains(&PromptReleaseId::new("rel_v1")));
    assert!(policy
        .attached_release_ids
        .contains(&PromptReleaseId::new("rel_v2")));
}

#[tokio::test]
async fn attach_release_is_idempotent() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_policy(
            "e1",
            "policy_idem",
            "tenant_g",
            "Idempotent",
            1,
            vec![WorkspaceRole::Admin],
            None,
            None,
            ts,
        )])
        .await
        .unwrap();

    store.attach_release_to_policy("policy_idem", PromptReleaseId::new("rel_idem"));
    store.attach_release_to_policy("policy_idem", PromptReleaseId::new("rel_idem")); // duplicate

    let policy = ApprovalPolicyReadModel::get_policy(&store, "policy_idem")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        policy.attached_release_ids.len(),
        1,
        "duplicate attach must be deduplicated"
    );
}

#[tokio::test]
async fn attach_release_to_unknown_policy_returns_false() {
    let store = InMemoryStore::new();
    let ok = store.attach_release_to_policy("ghost_policy", PromptReleaseId::new("rel_x"));
    assert!(!ok, "attach to non-existent policy returns false");
}

// ── 5. list_by_tenant returns only matching tenant's policies ─────────────────

#[tokio::test]
async fn list_by_tenant_returns_only_matching_tenant_policies() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            create_policy(
                "e1",
                "pol_t1_a",
                "t_iso_1",
                "Policy A",
                1,
                vec![WorkspaceRole::Admin],
                None,
                None,
                ts,
            ),
            create_policy(
                "e2",
                "pol_t1_b",
                "t_iso_1",
                "Policy B",
                2,
                vec![WorkspaceRole::Owner],
                None,
                None,
                ts + 1,
            ),
            create_policy(
                "e3",
                "pol_t2_a",
                "t_iso_2",
                "Policy C",
                1,
                vec![WorkspaceRole::Admin],
                None,
                None,
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    let t1_policies =
        ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("t_iso_1"), 10, 0)
            .await
            .unwrap();
    assert_eq!(t1_policies.len(), 2, "tenant_1 has 2 policies");
    assert!(t1_policies
        .iter()
        .all(|p| p.tenant_id.as_str() == "t_iso_1"));
    let t1_ids: Vec<_> = t1_policies.iter().map(|p| p.policy_id.as_str()).collect();
    assert!(t1_ids.contains(&"pol_t1_a"));
    assert!(t1_ids.contains(&"pol_t1_b"));
    assert!(
        !t1_ids.contains(&"pol_t2_a"),
        "t_iso_2 policy must not appear"
    );

    let t2_policies =
        ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("t_iso_2"), 10, 0)
            .await
            .unwrap();
    assert_eq!(t2_policies.len(), 1);
    assert_eq!(t2_policies[0].policy_id, "pol_t2_a");

    // Unknown tenant returns empty.
    let empty = ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("t_nobody"), 10, 0)
        .await
        .unwrap();
    assert!(empty.is_empty());
}

// ── 6. Cross-tenant isolation ─────────────────────────────────────────────────

#[tokio::test]
async fn cross_tenant_policies_are_isolated() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Both tenants have a policy with the same name — different IDs.
    store
        .append(&[
            create_policy(
                "e1",
                "alpha_pol",
                "alpha_tenant",
                "Security Review",
                2,
                vec![WorkspaceRole::Admin],
                None,
                Some(86_400_000),
                ts,
            ),
            create_policy(
                "e2",
                "beta_pol",
                "beta_tenant",
                "Security Review",
                1,
                vec![WorkspaceRole::Owner],
                Some(3_600_000),
                None,
                ts + 1,
            ),
        ])
        .await
        .unwrap();

    // Direct get works for either tenant by policy_id.
    let alpha = ApprovalPolicyReadModel::get_policy(&store, "alpha_pol")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alpha.tenant_id.as_str(), "alpha_tenant");
    assert_eq!(alpha.required_approvers, 2);
    assert!(alpha.auto_approve_after_ms.is_none());
    assert_eq!(alpha.auto_reject_after_ms, Some(86_400_000));

    let beta = ApprovalPolicyReadModel::get_policy(&store, "beta_pol")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(beta.tenant_id.as_str(), "beta_tenant");
    assert_eq!(beta.required_approvers, 1);
    assert_eq!(beta.auto_approve_after_ms, Some(3_600_000));
    assert!(beta.auto_reject_after_ms.is_none());

    // list_by_tenant is scoped.
    let alpha_list =
        ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("alpha_tenant"), 10, 0)
            .await
            .unwrap();
    assert_eq!(alpha_list.len(), 1);
    assert_eq!(alpha_list[0].policy_id, "alpha_pol");

    let beta_list =
        ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("beta_tenant"), 10, 0)
            .await
            .unwrap();
    assert_eq!(beta_list.len(), 1);
    assert_eq!(beta_list[0].policy_id, "beta_pol");
}

// ── 7. list_by_tenant sorted by policy_id ────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_sorted_by_policy_id() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Append in reverse lexicographic order.
    store
        .append(&[
            create_policy(
                "e1",
                "zzz_pol",
                "t_sort",
                "ZZZ",
                1,
                vec![WorkspaceRole::Admin],
                None,
                None,
                ts,
            ),
            create_policy(
                "e2",
                "aaa_pol",
                "t_sort",
                "AAA",
                1,
                vec![WorkspaceRole::Admin],
                None,
                None,
                ts + 1,
            ),
            create_policy(
                "e3",
                "mmm_pol",
                "t_sort",
                "MMM",
                1,
                vec![WorkspaceRole::Admin],
                None,
                None,
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    let policies = ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("t_sort"), 10, 0)
        .await
        .unwrap();

    assert_eq!(policies.len(), 3);
    assert_eq!(policies[0].policy_id, "aaa_pol", "sorted lexicographically");
    assert_eq!(policies[1].policy_id, "mmm_pol");
    assert_eq!(policies[2].policy_id, "zzz_pol");
}

// ── 8. list_by_tenant pagination ─────────────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_respects_limit_and_offset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u32..4 {
        store
            .append(&[create_policy(
                &format!("e{i}"),
                &format!("pol_pg_{i:02}"),
                "t_page",
                &format!("Policy {i}"),
                1,
                vec![WorkspaceRole::Admin],
                None,
                None,
                ts + i as u64,
            )])
            .await
            .unwrap();
    }

    let page1 = ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("t_page"), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].policy_id, "pol_pg_00");
    assert_eq!(page1[1].policy_id, "pol_pg_01");

    let page2 = ApprovalPolicyReadModel::list_by_tenant(&store, &TenantId::new("t_page"), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].policy_id, "pol_pg_02");
    assert_eq!(page2[1].policy_id, "pol_pg_03");
}

// ── 9. get_policy returns None for unknown ID ─────────────────────────────────

#[tokio::test]
async fn get_policy_returns_none_for_unknown_id() {
    let store = InMemoryStore::new();
    let result = ApprovalPolicyReadModel::get_policy(&store, "nonexistent")
        .await
        .unwrap();
    assert!(result.is_none());
}
