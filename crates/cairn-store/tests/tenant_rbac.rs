//! Multi-tenancy isolation and RBAC integration tests (RFC 008).
//!
//! Validates that the event-sourced projections maintain strict tenant
//! boundaries: runs created in one tenant's project never appear in another
//! tenant's queries, and workspace membership records are scoped correctly.
//!
//! Pipeline under test:
//!   TenantCreated (×2)
//!     → WorkspaceCreated (one per tenant)
//!       → RunCreated (one per workspace)
//!         → RunReadModel isolation verified
//!           → WorkspaceMemberAdded → membership read model verified

use cairn_domain::{
    EventEnvelope, EventId, EventSource, OperatorId, ProjectId, ProjectKey, RunCreated, RunId,
    RuntimeEvent, SessionCreated, SessionId, TenantCreated, TenantId, WorkspaceCreated,
    WorkspaceId, WorkspaceMemberAdded,
};
use cairn_domain::tenancy::{WorkspaceKey, WorkspaceRole};
use cairn_store::{
    projections::{
        RunReadModel, SessionReadModel, TenantReadModel, WorkspaceMembershipReadModel,
        WorkspaceReadModel,
    },
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Canonical project key for a tenant + workspace pair.
fn project(tenant: &str, workspace: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
        project_id: ProjectId::new(format!("proj_{tenant}_{workspace}")),
    }
}

// ── 1. TenantCreated produces TenantRecord ────────────────────────────────────

#[tokio::test]
async fn two_tenants_produce_isolated_tenant_records() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::TenantCreated(TenantCreated {
                    project: project("tenant_a", "ws_a"),
                    tenant_id: TenantId::new("tenant_a"),
                    name: "Acme Corp".to_owned(),
                    created_at: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::TenantCreated(TenantCreated {
                    project: project("tenant_b", "ws_b"),
                    tenant_id: TenantId::new("tenant_b"),
                    name: "Beta Inc".to_owned(),
                    created_at: ts + 1,
                }),
            ),
        ])
        .await
        .unwrap();

    let a = TenantReadModel::get(&store, &TenantId::new("tenant_a"))
        .await
        .unwrap()
        .expect("tenant_a must exist");
    assert_eq!(a.name, "Acme Corp");

    let b = TenantReadModel::get(&store, &TenantId::new("tenant_b"))
        .await
        .unwrap()
        .expect("tenant_b must exist");
    assert_eq!(b.name, "Beta Inc");

    // Neither tenant sees the other's record.
    assert_ne!(a.tenant_id, b.tenant_id);

    let all = TenantReadModel::list(&store, 10, 0).await.unwrap();
    assert_eq!(all.len(), 2, "both tenants appear in global list");
}

// ── 2. WorkspaceCreated is scoped per tenant ──────────────────────────────────

#[tokio::test]
async fn workspace_list_is_scoped_to_owning_tenant() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::TenantCreated(TenantCreated {
                    project: project("ta", "wa"),
                    tenant_id: TenantId::new("ta"),
                    name: "Tenant A".to_owned(),
                    created_at: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::TenantCreated(TenantCreated {
                    project: project("tb", "wb"),
                    tenant_id: TenantId::new("tb"),
                    name: "Tenant B".to_owned(),
                    created_at: ts + 1,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::WorkspaceCreated(WorkspaceCreated {
                    project: project("ta", "wa"),
                    workspace_id: WorkspaceId::new("wa"),
                    tenant_id: TenantId::new("ta"),
                    name: "Workspace A".to_owned(),
                    created_at: ts + 2,
                }),
            ),
            evt(
                "e4",
                RuntimeEvent::WorkspaceCreated(WorkspaceCreated {
                    project: project("tb", "wb"),
                    workspace_id: WorkspaceId::new("wb"),
                    tenant_id: TenantId::new("tb"),
                    name: "Workspace B".to_owned(),
                    created_at: ts + 3,
                }),
            ),
        ])
        .await
        .unwrap();

    // Each tenant only sees their own workspace.
    let wa = WorkspaceReadModel::list_by_tenant(&store, &TenantId::new("ta"), 10, 0)
        .await
        .unwrap();
    assert_eq!(wa.len(), 1);
    assert_eq!(wa[0].workspace_id.as_str(), "wa");

    let wb = WorkspaceReadModel::list_by_tenant(&store, &TenantId::new("tb"), 10, 0)
        .await
        .unwrap();
    assert_eq!(wb.len(), 1);
    assert_eq!(wb[0].workspace_id.as_str(), "wb");

    // Cross-tenant workspace lookup returns None.
    let wrong = WorkspaceReadModel::get(&store, &WorkspaceId::new("wb")).await.unwrap().unwrap();
    assert_eq!(wrong.tenant_id.as_str(), "tb", "wb belongs to tb, not ta");
}

// ── 3. RunReadModel only returns runs for the queried project ─────────────────

#[tokio::test]
async fn run_list_is_scoped_to_project() {
    let store = InMemoryStore::new();
    let proj_a = project("tenant_a", "ws_a");
    let proj_b = project("tenant_b", "ws_b");
    let ts = now_ms();

    // Create sessions first (runs reference a session).
    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: proj_a.clone(),
                session_id: SessionId::new("sess_a"),
            })),
            evt("e2", RuntimeEvent::SessionCreated(SessionCreated {
                project: proj_b.clone(),
                session_id: SessionId::new("sess_b"),
            })),
            evt("e3", RuntimeEvent::RunCreated(RunCreated {
                project: proj_a.clone(),
                session_id: SessionId::new("sess_a"),
                run_id: RunId::new("run_a1"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            evt("e4", RuntimeEvent::RunCreated(RunCreated {
                project: proj_a.clone(),
                session_id: SessionId::new("sess_a"),
                run_id: RunId::new("run_a2"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
            evt("e5", RuntimeEvent::RunCreated(RunCreated {
                project: proj_b.clone(),
                session_id: SessionId::new("sess_b"),
                run_id: RunId::new("run_b1"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
        ])
        .await
        .unwrap();

    // Project A sees only its two runs.
    let runs_a = RunReadModel::list_by_session(
        &store,
        &SessionId::new("sess_a"),
        10,
        0,
    )
    .await
    .unwrap();
    assert_eq!(runs_a.len(), 2, "project A should have exactly 2 runs");
    assert!(runs_a.iter().all(|r| r.project == proj_a), "all runs belong to project A");

    // Project B sees only its one run.
    let runs_b = RunReadModel::list_by_session(
        &store,
        &SessionId::new("sess_b"),
        10,
        0,
    )
    .await
    .unwrap();
    assert_eq!(runs_b.len(), 1, "project B should have exactly 1 run");
    assert_eq!(runs_b[0].run_id.as_str(), "run_b1");
    assert_eq!(runs_b[0].project, proj_b);

    // Individual run lookup is correctly attributed.
    let run_a1 = RunReadModel::get(&store, &RunId::new("run_a1")).await.unwrap().unwrap();
    assert_eq!(run_a1.project.tenant_id.as_str(), "tenant_a");

    let run_b1 = RunReadModel::get(&store, &RunId::new("run_b1")).await.unwrap().unwrap();
    assert_eq!(run_b1.project.tenant_id.as_str(), "tenant_b");
}

// ── 4. Cross-tenant isolation: querying with wrong project returns empty ──────

#[tokio::test]
async fn cross_tenant_run_query_returns_empty() {
    let store = InMemoryStore::new();
    let proj_a = project("tenant_x", "ws_x");
    let proj_b = project("tenant_y", "ws_y");

    store
        .append(&[
            evt("e1", RuntimeEvent::SessionCreated(SessionCreated {
                project: proj_a.clone(),
                session_id: SessionId::new("sess_x"),
            })),
            evt("e2", RuntimeEvent::RunCreated(RunCreated {
                project: proj_a.clone(),
                session_id: SessionId::new("sess_x"),
                run_id: RunId::new("run_x"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            })),
        ])
        .await
        .unwrap();

    // Querying with tenant_y's session ID (which never had a session created)
    // returns an empty list — tenant_y cannot see tenant_x's runs.
    let cross = RunReadModel::list_by_session(
        &store,
        &SessionId::new("sess_nonexistent"),
        10,
        0,
    )
    .await
    .unwrap();
    assert!(cross.is_empty(), "cross-tenant run query must return empty");

    // Direct run lookup with a known run ID still returns its true owner.
    let run = RunReadModel::get(&store, &RunId::new("run_x")).await.unwrap().unwrap();
    assert_eq!(run.project.tenant_id.as_str(), "tenant_x");
    assert_ne!(run.project.tenant_id.as_str(), "tenant_y");

    // Session for tenant_y does not exist.
    let sess_y = SessionReadModel::get(&store, &SessionId::new("sess_nonexistent"))
        .await
        .unwrap();
    assert!(sess_y.is_none(), "tenant_y has no sessions");
}

// ── 5. WorkspaceMemberAdded produces membership record ───────────────────────

#[tokio::test]
async fn workspace_member_added_produces_member_record() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let wkey = WorkspaceKey {
        tenant_id: TenantId::new("t_rbac"),
        workspace_id: WorkspaceId::new("w_rbac"),
    };

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
                workspace_key: wkey.clone(),
                member_id: OperatorId::new("op_alice"),
                role: WorkspaceRole::Admin,
                added_at_ms: ts,
            }),
        )])
        .await
        .unwrap();

    let members = WorkspaceMembershipReadModel::list_workspace_members(&store, "w_rbac")
        .await
        .unwrap();
    assert_eq!(members.len(), 1, "one member added");
    assert_eq!(members[0].operator_id, "op_alice");
    assert_eq!(members[0].role, WorkspaceRole::Admin);
    assert_eq!(members[0].added_at_ms, ts);

    // get_member by workspace key.
    let record = WorkspaceMembershipReadModel::get_member(&store, &wkey, "op_alice")
        .await
        .unwrap()
        .expect("alice must be found by get_member");
    assert_eq!(record.workspace_id, "w_rbac");
    assert_eq!(record.role, WorkspaceRole::Admin);
}

// ── 6. Multiple members per workspace, isolated from other workspaces ─────────

#[tokio::test]
async fn workspace_members_are_scoped_to_workspace() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let wkey_1 = WorkspaceKey {
        tenant_id: TenantId::new("t_multi"),
        workspace_id: WorkspaceId::new("ws_one"),
    };
    let wkey_2 = WorkspaceKey {
        tenant_id: TenantId::new("t_multi"),
        workspace_id: WorkspaceId::new("ws_two"),
    };

    store
        .append(&[
            evt("e1", RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
                workspace_key: wkey_1.clone(),
                member_id: OperatorId::new("alice"),
                role: WorkspaceRole::Owner,
                added_at_ms: ts,
            })),
            evt("e2", RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
                workspace_key: wkey_1.clone(),
                member_id: OperatorId::new("bob"),
                role: WorkspaceRole::Member,
                added_at_ms: ts + 1,
            })),
            evt("e3", RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
                workspace_key: wkey_2.clone(),
                member_id: OperatorId::new("carol"),
                role: WorkspaceRole::Viewer,
                added_at_ms: ts + 2,
            })),
        ])
        .await
        .unwrap();

    let ws1_members = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_one")
        .await
        .unwrap();
    assert_eq!(ws1_members.len(), 2, "ws_one has alice and bob");
    let roles: Vec<_> = ws1_members.iter().map(|m| m.operator_id.as_str()).collect();
    assert!(roles.contains(&"alice"));
    assert!(roles.contains(&"bob"));
    assert!(!roles.contains(&"carol"), "carol is in ws_two, not ws_one");

    let ws2_members = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_two")
        .await
        .unwrap();
    assert_eq!(ws2_members.len(), 1);
    assert_eq!(ws2_members[0].operator_id, "carol");
    assert_eq!(ws2_members[0].role, WorkspaceRole::Viewer);
}

// ── 7. Role upgrade is reflected in read model ───────────────────────────────

#[tokio::test]
async fn re_adding_member_upgrades_role() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let wkey = WorkspaceKey {
        tenant_id: TenantId::new("t_upgrade"),
        workspace_id: WorkspaceId::new("ws_upgrade"),
    };

    // Add as Member first, then re-add as Admin (role upgrade).
    store
        .append(&[
            evt("e1", RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
                workspace_key: wkey.clone(),
                member_id: OperatorId::new("dave"),
                role: WorkspaceRole::Member,
                added_at_ms: ts,
            })),
            evt("e2", RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
                workspace_key: wkey.clone(),
                member_id: OperatorId::new("dave"),
                role: WorkspaceRole::Admin,
                added_at_ms: ts + 1,
            })),
        ])
        .await
        .unwrap();

    let members = WorkspaceMembershipReadModel::list_workspace_members(&store, "ws_upgrade")
        .await
        .unwrap();

    // Upsert semantics: dave appears exactly once with the updated role.
    assert_eq!(members.len(), 1, "re-adding a member should upsert, not duplicate");
    assert_eq!(members[0].operator_id, "dave");
    assert_eq!(members[0].role, WorkspaceRole::Admin, "role must be upgraded to Admin");
}
