//! Provider binding lifecycle tests (RFC 009).
//!
//! Validates the full provider binding pipeline: connection registration,
//! binding creation, state changes, and project-scoped queries.
//!
//! Architecture:
//!   ProviderConnection  — tenant-level, represents a configured LLM backend
//!   ProviderBinding     — project-level, links a connection + model to routing
//!
//! Note on "priority ranking":
//!   ProviderBindingRecord has no priority field (priority lives in RouteRule).
//!   Effective routing priority is expressed by the `active` flag: only
//!   `active=true` bindings are considered by the router. `list_active` is the
//!   priority-filtered view. Tests verify this via:
//!     - Multiple bindings → list_active returns only active ones
//!     - Inactive bindings excluded from routing candidates
//!     - list_by_project returns ALL bindings (active + inactive) for audit

use cairn_domain::providers::{
    OperationKind, ProviderBindingSettings, ProviderConnectionStatus, StructuredOutputMode,
};
use cairn_domain::tenancy::TenantKey;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, ProviderBindingCreated,
    ProviderBindingId, ProviderBindingStateChanged, ProviderConnectionId,
    ProviderConnectionRegistered, ProviderModelId, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::{ProviderBindingReadModel, ProviderConnectionReadModel},
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

fn default_project() -> ProjectKey {
    project("t_bind", "w_bind")
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

fn default_settings() -> ProviderBindingSettings {
    ProviderBindingSettings {
        temperature_milli: Some(700),
        max_output_tokens: Some(4096),
        timeout_ms: Some(30_000),
        structured_output_mode: StructuredOutputMode::Default,
        required_capabilities: vec![],
        disabled_capabilities: vec![],
        cost_type: Default::default(),
        daily_budget_micros: None,
    }
}

fn register_connection(
    evt_id: &str,
    conn_id: &str,
    tenant: &str,
    family: &str,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ProviderConnectionRegistered(ProviderConnectionRegistered {
            tenant: TenantKey {
                tenant_id: TenantId::new(tenant),
            },
            provider_connection_id: ProviderConnectionId::new(conn_id),
            provider_family: family.to_owned(),
            adapter_type: format!("{family}_adapter"),
            supported_models: vec![],
            status: ProviderConnectionStatus::Active,
            registered_at: ts,
        }),
    )
}

fn create_binding(
    evt_id: &str,
    binding_id: &str,
    conn_id: &str,
    model_id: &str,
    op: OperationKind,
    active: bool,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
            project: default_project(),
            provider_binding_id: ProviderBindingId::new(binding_id),
            provider_connection_id: ProviderConnectionId::new(conn_id),
            provider_model_id: ProviderModelId::new(model_id),
            operation_kind: op,
            settings: default_settings(),
            policy_id: None,
            active,
            created_at: ts,
            estimated_cost_micros: None,
        }),
    )
}

fn change_binding_state(
    evt_id: &str,
    binding_id: &str,
    active: bool,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::ProviderBindingStateChanged(ProviderBindingStateChanged {
            project: default_project(),
            provider_binding_id: ProviderBindingId::new(binding_id),
            active,
            changed_at: now_ms(),
        }),
    )
}

// ── 1. Create provider connection ────────────────────────────────────────────

#[tokio::test]
async fn provider_connection_registered_is_stored() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let conn_id = ProviderConnectionId::new("conn_openai");

    store
        .append(&[register_connection(
            "e1",
            "conn_openai",
            "t_bind",
            "openai",
            ts,
        )])
        .await
        .unwrap();

    let record = ProviderConnectionReadModel::get(&store, &conn_id)
        .await
        .unwrap()
        .expect("connection must exist after registration");

    assert_eq!(record.provider_connection_id, conn_id);
    assert_eq!(record.tenant_id.as_str(), "t_bind");
    assert_eq!(record.provider_family, "openai");
    assert_eq!(record.adapter_type, "openai_adapter");
    assert_eq!(record.status, ProviderConnectionStatus::Active);
    assert_eq!(record.created_at, ts);
}

// ── 2. Create binding linking connection to model ─────────────────────────────

#[tokio::test]
async fn provider_binding_created_links_connection_and_model() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let binding_id = ProviderBindingId::new("bind_001");

    store
        .append(&[
            register_connection("e1", "conn_a1", "t_bind", "openai", ts),
            create_binding(
                "e2",
                "bind_001",
                "conn_a1",
                "gpt-4o",
                OperationKind::Generate,
                true,
                ts + 1,
            ),
        ])
        .await
        .unwrap();

    let record = ProviderBindingReadModel::get(&store, &binding_id)
        .await
        .unwrap()
        .expect("binding must exist after creation");

    assert_eq!(record.provider_binding_id, binding_id);
    assert_eq!(record.provider_connection_id.as_str(), "conn_a1");
    assert_eq!(record.provider_model_id.as_str(), "gpt-4o");
    assert_eq!(record.operation_kind, OperationKind::Generate);
    assert!(record.active, "binding created with active=true");
    assert_eq!(record.project, default_project());
    assert_eq!(record.settings.temperature_milli, Some(700));
    assert_eq!(record.settings.max_output_tokens, Some(4096));
}

// ── 3. Verify ProviderBindingReadModel returns the binding ────────────────────

#[tokio::test]
async fn get_returns_none_for_unknown_binding() {
    let store = InMemoryStore::new();
    let result = ProviderBindingReadModel::get(&store, &ProviderBindingId::new("nonexistent"))
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn binding_carries_all_settings_fields() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let settings = ProviderBindingSettings {
        temperature_milli: Some(800),
        max_output_tokens: Some(8192),
        timeout_ms: Some(60_000),
        structured_output_mode: StructuredOutputMode::Preferred,
        required_capabilities: vec![],
        disabled_capabilities: vec![],
        cost_type: Default::default(),
        daily_budget_micros: Some(1_000_000),
    };

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                project: default_project(),
                provider_binding_id: ProviderBindingId::new("bind_full"),
                provider_connection_id: ProviderConnectionId::new("conn_full"),
                provider_model_id: ProviderModelId::new("claude-sonnet-4-6"),
                operation_kind: OperationKind::Generate,
                settings: settings.clone(),
                policy_id: None,
                active: true,
                created_at: ts,
                estimated_cost_micros: None,
            }),
        )])
        .await
        .unwrap();

    let r = ProviderBindingReadModel::get(&store, &ProviderBindingId::new("bind_full"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(r.settings.temperature_milli, Some(800));
    assert_eq!(r.settings.max_output_tokens, Some(8192));
    assert_eq!(r.settings.timeout_ms, Some(60_000));
    assert_eq!(
        r.settings.structured_output_mode,
        StructuredOutputMode::Preferred
    );
    assert_eq!(r.settings.daily_budget_micros, Some(1_000_000));
}

// ── 4. Change binding state to inactive ──────────────────────────────────────

#[tokio::test]
async fn binding_state_changed_to_inactive() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            register_connection("e1", "conn_deact", "t_bind", "anthropic", ts),
            create_binding(
                "e2",
                "bind_deact",
                "conn_deact",
                "claude-haiku-4-5",
                OperationKind::Generate,
                true,
                ts + 1,
            ),
        ])
        .await
        .unwrap();

    // Verify active before deactivation.
    let before = ProviderBindingReadModel::get(&store, &ProviderBindingId::new("bind_deact"))
        .await
        .unwrap()
        .unwrap();
    assert!(before.active, "binding must start as active");

    // Deactivate.
    store
        .append(&[change_binding_state("e3", "bind_deact", false)])
        .await
        .unwrap();

    let after = ProviderBindingReadModel::get(&store, &ProviderBindingId::new("bind_deact"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !after.active,
        "binding must be inactive after ProviderBindingStateChanged"
    );
}

// ── 5. Verify state updated — re-activate ────────────────────────────────────

#[tokio::test]
async fn binding_state_can_be_toggled_active_inactive_active() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[create_binding(
            "e1",
            "bind_toggle",
            "conn_tog",
            "gpt-4o-mini",
            OperationKind::Generate,
            true,
            ts,
        )])
        .await
        .unwrap();

    // active → inactive.
    store
        .append(&[change_binding_state("e2", "bind_toggle", false)])
        .await
        .unwrap();
    let r = ProviderBindingReadModel::get(&store, &ProviderBindingId::new("bind_toggle"))
        .await
        .unwrap()
        .unwrap();
    assert!(!r.active);

    // inactive → active.
    store
        .append(&[change_binding_state("e3", "bind_toggle", true)])
        .await
        .unwrap();
    let r = ProviderBindingReadModel::get(&store, &ProviderBindingId::new("bind_toggle"))
        .await
        .unwrap()
        .unwrap();
    assert!(r.active, "re-activated binding must show active=true");
}

// ── 6. list_by_project scoping ───────────────────────────────────────────────

#[tokio::test]
async fn list_by_project_returns_only_project_bindings() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let proj_a = project("ta", "wa");
    let proj_b = project("tb", "wb");

    // Two bindings for project A, one for project B.
    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: proj_a.clone(),
                    provider_binding_id: ProviderBindingId::new("bind_a1"),
                    provider_connection_id: ProviderConnectionId::new("conn_x"),
                    provider_model_id: ProviderModelId::new("gpt-4o"),
                    operation_kind: OperationKind::Generate,
                    settings: default_settings(),
                    policy_id: None,
                    active: true,
                    created_at: ts,
                    estimated_cost_micros: None,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: proj_a.clone(),
                    provider_binding_id: ProviderBindingId::new("bind_a2"),
                    provider_connection_id: ProviderConnectionId::new("conn_x"),
                    provider_model_id: ProviderModelId::new("gpt-4o-mini"),
                    operation_kind: OperationKind::Embed,
                    settings: default_settings(),
                    policy_id: None,
                    active: true,
                    created_at: ts + 1,
                    estimated_cost_micros: None,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: proj_b.clone(),
                    provider_binding_id: ProviderBindingId::new("bind_b1"),
                    provider_connection_id: ProviderConnectionId::new("conn_y"),
                    provider_model_id: ProviderModelId::new("claude-haiku-4-5"),
                    operation_kind: OperationKind::Generate,
                    settings: default_settings(),
                    policy_id: None,
                    active: true,
                    created_at: ts + 2,
                    estimated_cost_micros: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let bindings_a = ProviderBindingReadModel::list_by_project(&store, &proj_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(bindings_a.len(), 2, "project A has 2 bindings");
    assert!(bindings_a.iter().all(|b| b.project == proj_a));

    let ids_a: std::collections::HashSet<_> = bindings_a
        .iter()
        .map(|b| b.provider_binding_id.as_str())
        .collect();
    assert!(ids_a.contains(&"bind_a1"));
    assert!(ids_a.contains(&"bind_a2"));
    assert!(
        !ids_a.contains(&"bind_b1"),
        "project B binding must not appear in A"
    );

    let bindings_b = ProviderBindingReadModel::list_by_project(&store, &proj_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(bindings_b.len(), 1);
    assert_eq!(bindings_b[0].provider_binding_id.as_str(), "bind_b1");
}

// ── 7. list_active: multiple bindings ranked by active flag ──────────────────

#[tokio::test]
async fn list_active_returns_only_active_bindings_for_operation() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Three Generate bindings: 2 active, 1 inactive.
    // One Embed binding: active.
    store
        .append(&[
            create_binding(
                "e1",
                "gen_a",
                "conn_1",
                "gpt-4o",
                OperationKind::Generate,
                true,
                ts,
            ),
            create_binding(
                "e2",
                "gen_b",
                "conn_2",
                "gpt-4o-mini",
                OperationKind::Generate,
                true,
                ts + 1,
            ),
            create_binding(
                "e3",
                "gen_c",
                "conn_3",
                "claude-haiku",
                OperationKind::Generate,
                false,
                ts + 2,
            ),
            create_binding(
                "e4",
                "emb_a",
                "conn_1",
                "text-embedding",
                OperationKind::Embed,
                true,
                ts + 3,
            ),
        ])
        .await
        .unwrap();

    // list_active(Generate) returns only active generate bindings.
    let active_gen =
        ProviderBindingReadModel::list_active(&store, &default_project(), OperationKind::Generate)
            .await
            .unwrap();
    assert_eq!(active_gen.len(), 2, "2 active generate bindings");
    let gen_ids: Vec<_> = active_gen
        .iter()
        .map(|b| b.provider_binding_id.as_str())
        .collect();
    assert!(gen_ids.contains(&"gen_a"));
    assert!(gen_ids.contains(&"gen_b"));
    assert!(
        !gen_ids.contains(&"gen_c"),
        "inactive binding excluded from routing candidates"
    );
    assert!(
        !gen_ids.contains(&"emb_a"),
        "embed binding excluded from generate results"
    );

    // list_active(Embed) returns only active embed binding.
    let active_emb =
        ProviderBindingReadModel::list_active(&store, &default_project(), OperationKind::Embed)
            .await
            .unwrap();
    assert_eq!(active_emb.len(), 1);
    assert_eq!(active_emb[0].provider_binding_id.as_str(), "emb_a");

    // list_by_project returns ALL 4 (active + inactive, all operations).
    let all = ProviderBindingReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        all.len(),
        4,
        "list_by_project includes inactive bindings for audit"
    );
}

#[tokio::test]
async fn deactivating_binding_removes_it_from_routing_candidates() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            create_binding(
                "e1",
                "bind_pri_1",
                "conn_p1",
                "gpt-4o",
                OperationKind::Generate,
                true,
                ts,
            ),
            create_binding(
                "e2",
                "bind_pri_2",
                "conn_p2",
                "gpt-4o-mini",
                OperationKind::Generate,
                true,
                ts + 1,
            ),
            create_binding(
                "e3",
                "bind_pri_3",
                "conn_p3",
                "claude-haiku",
                OperationKind::Generate,
                true,
                ts + 2,
            ),
        ])
        .await
        .unwrap();

    // Initially all 3 active.
    let before =
        ProviderBindingReadModel::list_active(&store, &default_project(), OperationKind::Generate)
            .await
            .unwrap();
    assert_eq!(before.len(), 3, "all 3 bindings are routing candidates");

    // Deactivate the primary binding (simulate failover).
    store
        .append(&[change_binding_state("e4", "bind_pri_1", false)])
        .await
        .unwrap();

    let after =
        ProviderBindingReadModel::list_active(&store, &default_project(), OperationKind::Generate)
            .await
            .unwrap();
    assert_eq!(
        after.len(),
        2,
        "deactivated binding removed from routing candidates"
    );
    let remaining: Vec<_> = after
        .iter()
        .map(|b| b.provider_binding_id.as_str())
        .collect();
    assert!(
        !remaining.contains(&"bind_pri_1"),
        "deactivated binding excluded"
    );
    assert!(remaining.contains(&"bind_pri_2"));
    assert!(remaining.contains(&"bind_pri_3"));
}

// ── 8. Connection read model: list_by_tenant ─────────────────────────────────

#[tokio::test]
async fn connections_listed_by_tenant() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            register_connection("e1", "conn_t1_a", "tenant_one", "openai", ts),
            register_connection("e2", "conn_t1_b", "tenant_one", "anthropic", ts + 1),
            register_connection("e3", "conn_t2_a", "tenant_two", "openrouter", ts + 2),
        ])
        .await
        .unwrap();

    let t1_conns =
        ProviderConnectionReadModel::list_by_tenant(&store, &TenantId::new("tenant_one"), 10, 0)
            .await
            .unwrap();
    assert_eq!(t1_conns.len(), 2);
    assert!(t1_conns
        .iter()
        .all(|c| c.tenant_id.as_str() == "tenant_one"));

    let t2_conns =
        ProviderConnectionReadModel::list_by_tenant(&store, &TenantId::new("tenant_two"), 10, 0)
            .await
            .unwrap();
    assert_eq!(t2_conns.len(), 1);
    assert_eq!(t2_conns[0].provider_family, "openrouter");
}

// ── 9. OperationKind variants are isolated in list_active ─────────────────────

#[tokio::test]
async fn list_active_is_scoped_by_operation_kind() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for (i, op) in [
        (0, OperationKind::Generate),
        (1, OperationKind::Embed),
        (2, OperationKind::Rerank),
    ] {
        store
            .append(&[evt(
                &format!("e{i}"),
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: default_project(),
                    provider_binding_id: ProviderBindingId::new(format!("bind_op_{i}")),
                    provider_connection_id: ProviderConnectionId::new("conn_ops"),
                    provider_model_id: ProviderModelId::new("model_ops"),
                    operation_kind: op,
                    settings: default_settings(),
                    policy_id: None,
                    active: true,
                    created_at: ts + i as u64,
                    estimated_cost_micros: None,
                }),
            )])
            .await
            .unwrap();
    }

    // Each operation kind returns exactly 1 binding.
    for (i, op) in [
        (0, OperationKind::Generate),
        (1, OperationKind::Embed),
        (2, OperationKind::Rerank),
    ] {
        let results = ProviderBindingReadModel::list_active(&store, &default_project(), op)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "{op:?} must return exactly 1 binding");
        assert_eq!(results[0].operation_kind, op);
        assert_eq!(
            results[0].provider_binding_id.as_str(),
            format!("bind_op_{i}")
        );
    }
}
