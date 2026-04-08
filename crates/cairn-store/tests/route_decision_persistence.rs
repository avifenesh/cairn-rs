//! Route decision persistence integration tests (RFC 009).
//!
//! Validates that provider routing decisions are durably stored and queryable
//! after being emitted as events. Route decisions are the operator audit record
//! for every LLM call — they must survive restart and be correctly scoped by
//! project.
//!
//! Projection contract:
//!   RouteDecisionMade → RouteDecisionRecord with:
//!     - route_decision_id, project_id, operation_kind
//!     - selected_provider_binding_id (Option)
//!     - attempt_count, fallback_used, final_status
//!
//!   list_by_project   → all decisions for a project, sorted by route_decision_id
//!
//! Critical invariants:
//!   - fallback_used flag persists exactly as emitted
//!   - NoViableRoute decisions are stored (selected_provider_binding_id = None)
//!   - list_by_project is scoped by project_id, not full ProjectKey

use cairn_domain::providers::{OperationKind, RouteDecisionStatus};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, ProviderBindingId, RouteDecisionId,
    RouteDecisionMade, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_store::{projections::RouteDecisionReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str, workspace: &str, proj: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
        project_id: ProjectId::new(proj),
    }
}

fn default_project() -> ProjectKey {
    project("t_route", "w_route", "p_route")
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

/// Build a RouteDecisionMade event with a selected binding (happy path).
fn decision_selected(
    evt_id: &str,
    decision_id: &str,
    binding_id: &str,
    attempt_count: u16,
    fallback_used: bool,
    op: OperationKind,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
            project: default_project(),
            route_decision_id: RouteDecisionId::new(decision_id),
            operation_kind: op,
            selected_provider_binding_id: Some(ProviderBindingId::new(binding_id)),
            final_status: RouteDecisionStatus::Selected,
            attempt_count,
            fallback_used,
            decided_at: now_ms(),
        }),
    )
}

/// Build a RouteDecisionMade event with no viable route (failure path).
fn decision_no_route(
    evt_id: &str,
    decision_id: &str,
    attempt_count: u16,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
            project: default_project(),
            route_decision_id: RouteDecisionId::new(decision_id),
            operation_kind: OperationKind::Generate,
            selected_provider_binding_id: None,
            final_status: RouteDecisionStatus::NoViableRoute,
            attempt_count,
            fallback_used: false,
            decided_at: now_ms(),
        }),
    )
}

// ── 1. Single RouteDecisionMade → RouteDecisionRecord persisted ───────────────

#[tokio::test]
async fn route_decision_made_is_persisted() {
    let store = InMemoryStore::new();
    let decision_id = RouteDecisionId::new("rd_001");

    store
        .append(&[decision_selected(
            "e1",
            "rd_001",
            "binding_openai",
            1,
            false,
            OperationKind::Generate,
        )])
        .await
        .unwrap();

    let record = RouteDecisionReadModel::get(&store, &decision_id)
        .await
        .unwrap()
        .expect("RouteDecisionRecord must exist after RouteDecisionMade");

    assert_eq!(record.route_decision_id, decision_id);
    assert_eq!(record.project_id.as_str(), "p_route");
    assert_eq!(record.operation_kind, OperationKind::Generate);
    assert_eq!(
        record.selected_provider_binding_id,
        Some(ProviderBindingId::new("binding_openai"))
    );
    assert_eq!(record.final_status, RouteDecisionStatus::Selected);
    assert_eq!(record.attempt_count, 1);
    assert!(!record.fallback_used, "fallback_used must persist as false");
}

// ── 2. All fields round-trip through the projection ───────────────────────────

#[tokio::test]
async fn route_decision_all_fields_round_trip() {
    let store = InMemoryStore::new();

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
                project: default_project(),
                route_decision_id: RouteDecisionId::new("rd_full"),
                operation_kind: OperationKind::Embed,
                selected_provider_binding_id: Some(ProviderBindingId::new("binding_anthropic")),
                final_status: RouteDecisionStatus::Selected,
                attempt_count: 3,
                fallback_used: true,
                decided_at: now_ms(),
            }),
        )])
        .await
        .unwrap();

    let r = RouteDecisionReadModel::get(&store, &RouteDecisionId::new("rd_full"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(r.operation_kind, OperationKind::Embed);
    assert_eq!(r.attempt_count, 3);
    assert!(r.fallback_used, "fallback_used=true must persist");
    assert_eq!(r.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        r.selected_provider_binding_id,
        Some(ProviderBindingId::new("binding_anthropic"))
    );
}

// ── 3. fallback_used=false and fallback_used=true both persist correctly ───────

#[tokio::test]
async fn fallback_used_flag_persists_correctly() {
    let store = InMemoryStore::new();

    // Decision 1: no fallback needed.
    store
        .append(&[decision_selected(
            "e1",
            "rd_no_fallback",
            "binding_primary",
            1,
            false,
            OperationKind::Generate,
        )])
        .await
        .unwrap();

    // Decision 2: fallback was needed (primary rejected, secondary selected).
    store
        .append(&[decision_selected(
            "e2",
            "rd_with_fallback",
            "binding_secondary",
            2,
            true,
            OperationKind::Generate,
        )])
        .await
        .unwrap();

    let no_fb = RouteDecisionReadModel::get(&store, &RouteDecisionId::new("rd_no_fallback"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !no_fb.fallback_used,
        "decision without fallback must have fallback_used=false"
    );
    assert_eq!(no_fb.attempt_count, 1);

    let with_fb = RouteDecisionReadModel::get(&store, &RouteDecisionId::new("rd_with_fallback"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        with_fb.fallback_used,
        "decision with fallback must have fallback_used=true"
    );
    assert_eq!(with_fb.attempt_count, 2);
    assert_eq!(
        with_fb.selected_provider_binding_id,
        Some(ProviderBindingId::new("binding_secondary"))
    );
}

// ── 4. Multiple decisions for same run tracked in order ───────────────────────

#[tokio::test]
async fn multiple_decisions_for_same_project_tracked_in_order() {
    let store = InMemoryStore::new();

    // Three decisions within one project, ordered by decision_id.
    // Use IDs that sort lexicographically: rd_001 < rd_002 < rd_003.
    store
        .append(&[
            decision_selected(
                "e1",
                "rd_seq_001",
                "binding_a",
                1,
                false,
                OperationKind::Generate,
            ),
            decision_selected(
                "e2",
                "rd_seq_002",
                "binding_b",
                2,
                true,
                OperationKind::Generate,
            ),
            decision_selected(
                "e3",
                "rd_seq_003",
                "binding_a",
                1,
                false,
                OperationKind::Embed,
            ),
        ])
        .await
        .unwrap();

    let decisions = RouteDecisionReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();

    assert_eq!(decisions.len(), 3, "all three decisions persisted");

    // Sorted by route_decision_id string — rd_seq_001 < rd_seq_002 < rd_seq_003.
    assert_eq!(decisions[0].route_decision_id.as_str(), "rd_seq_001");
    assert_eq!(decisions[1].route_decision_id.as_str(), "rd_seq_002");
    assert_eq!(decisions[2].route_decision_id.as_str(), "rd_seq_003");

    // Second decision had fallback.
    assert!(decisions[1].fallback_used);
    assert_eq!(decisions[1].attempt_count, 2);

    // Third decision is Embed, not Generate.
    assert_eq!(decisions[2].operation_kind, OperationKind::Embed);
}

// ── 5. NoViableRoute decision stored with no binding ─────────────────────────

#[tokio::test]
async fn no_viable_route_decision_stored_with_none_binding() {
    let store = InMemoryStore::new();

    store
        .append(&[decision_no_route("e1", "rd_no_route", 3)])
        .await
        .unwrap();

    let record = RouteDecisionReadModel::get(&store, &RouteDecisionId::new("rd_no_route"))
        .await
        .unwrap()
        .expect("NoViableRoute decision must be stored");

    assert_eq!(record.final_status, RouteDecisionStatus::NoViableRoute);
    assert!(
        record.selected_provider_binding_id.is_none(),
        "NoViableRoute has no selected binding"
    );
    assert_eq!(
        record.attempt_count, 3,
        "all 3 attempts tried before giving up"
    );
    assert!(!record.fallback_used);
}

// ── 6. All RouteDecisionStatus variants persist ───────────────────────────────

#[tokio::test]
async fn all_final_status_variants_persist_correctly() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let variants = [
        (
            "rd_selected",
            RouteDecisionStatus::Selected,
            Some("binding_x"),
        ),
        (
            "rd_failed_dispatch",
            RouteDecisionStatus::FailedAfterDispatch,
            Some("binding_y"),
        ),
        ("rd_no_route", RouteDecisionStatus::NoViableRoute, None),
        ("rd_cancelled", RouteDecisionStatus::Cancelled, None),
    ];

    for (id, status, binding) in &variants {
        store
            .append(&[evt(
                &format!("e_{id}"),
                RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
                    project: default_project(),
                    route_decision_id: RouteDecisionId::new(*id),
                    operation_kind: OperationKind::Generate,
                    selected_provider_binding_id: binding.map(ProviderBindingId::new),
                    final_status: *status,
                    attempt_count: 1,
                    fallback_used: false,
                    decided_at: ts,
                }),
            )])
            .await
            .unwrap();
    }

    for (id, expected_status, expected_binding) in &variants {
        let r = RouteDecisionReadModel::get(&store, &RouteDecisionId::new(*id))
            .await
            .unwrap()
            .expect(&format!("{id} must be persisted"));

        assert_eq!(
            r.final_status, *expected_status,
            "{id}: final_status mismatch"
        );
        assert_eq!(
            r.selected_provider_binding_id,
            expected_binding.map(ProviderBindingId::new),
            "{id}: binding mismatch"
        );
    }
}

// ── 7. list_by_project scoping — cross-project isolation ──────────────────────

#[tokio::test]
async fn list_by_project_scoped_to_project_id() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let proj_a = project("ta", "wa", "pa");
    let proj_b = project("tb", "wb", "pb");

    // Two decisions in project A, one in project B.
    for (evt_id, decision_id, proj) in [
        ("e1", "rd_pa_1", proj_a.clone()),
        ("e2", "rd_pa_2", proj_a.clone()),
        ("e3", "rd_pb_1", proj_b.clone()),
    ] {
        store
            .append(&[evt(
                evt_id,
                RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
                    project: proj,
                    route_decision_id: RouteDecisionId::new(decision_id),
                    operation_kind: OperationKind::Generate,
                    selected_provider_binding_id: Some(ProviderBindingId::new("binding_x")),
                    final_status: RouteDecisionStatus::Selected,
                    attempt_count: 1,
                    fallback_used: false,
                    decided_at: ts,
                }),
            )])
            .await
            .unwrap();
    }

    let decisions_a = RouteDecisionReadModel::list_by_project(&store, &proj_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(decisions_a.len(), 2, "project A has exactly 2 decisions");
    assert!(decisions_a
        .iter()
        .all(|d| d.project_id == proj_a.project_id));

    let decisions_b = RouteDecisionReadModel::list_by_project(&store, &proj_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(decisions_b.len(), 1, "project B has exactly 1 decision");
    assert_eq!(decisions_b[0].route_decision_id.as_str(), "rd_pb_1");

    // Project with no decisions returns empty.
    let proj_c = project("tc", "wc", "pc");
    let decisions_c = RouteDecisionReadModel::list_by_project(&store, &proj_c, 10, 0)
        .await
        .unwrap();
    assert!(decisions_c.is_empty());
}

// ── 8. list_by_project pagination ────────────────────────────────────────────

#[tokio::test]
async fn list_by_project_respects_limit_and_offset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 1u32..=5 {
        store
            .append(&[evt(
                &format!("e{i}"),
                RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
                    project: default_project(),
                    route_decision_id: RouteDecisionId::new(format!("rd_page_{i:02}")),
                    operation_kind: OperationKind::Generate,
                    selected_provider_binding_id: Some(ProviderBindingId::new("binding")),
                    final_status: RouteDecisionStatus::Selected,
                    attempt_count: 1,
                    fallback_used: false,
                    decided_at: ts,
                }),
            )])
            .await
            .unwrap();
    }

    let page1 = RouteDecisionReadModel::list_by_project(&store, &default_project(), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].route_decision_id.as_str(), "rd_page_01");
    assert_eq!(page1[1].route_decision_id.as_str(), "rd_page_02");

    let page2 = RouteDecisionReadModel::list_by_project(&store, &default_project(), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].route_decision_id.as_str(), "rd_page_03");
    assert_eq!(page2[1].route_decision_id.as_str(), "rd_page_04");

    let page3 = RouteDecisionReadModel::list_by_project(&store, &default_project(), 2, 4)
        .await
        .unwrap();
    assert_eq!(page3.len(), 1);
    assert_eq!(page3[0].route_decision_id.as_str(), "rd_page_05");
}

// ── 9. All OperationKind variants persist ─────────────────────────────────────

#[tokio::test]
async fn all_operation_kinds_persist_correctly() {
    let store = InMemoryStore::new();

    for (id, op) in [
        ("rd_gen", OperationKind::Generate),
        ("rd_embed", OperationKind::Embed),
        ("rd_rerank", OperationKind::Rerank),
    ] {
        store
            .append(&[decision_selected(&format!("e_{id}"), id, "b", 1, false, op)])
            .await
            .unwrap();
    }

    for (id, expected_op) in [
        ("rd_gen", OperationKind::Generate),
        ("rd_embed", OperationKind::Embed),
        ("rd_rerank", OperationKind::Rerank),
    ] {
        let r = RouteDecisionReadModel::get(&store, &RouteDecisionId::new(id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            r.operation_kind, expected_op,
            "{id}: operation_kind mismatch"
        );
    }
}

// ── 10. get() returns None for unknown decision ID ────────────────────────────

#[tokio::test]
async fn get_unknown_decision_returns_none() {
    let store = InMemoryStore::new();
    let result = RouteDecisionReadModel::get(&store, &RouteDecisionId::new("rd_ghost"))
        .await
        .unwrap();
    assert!(result.is_none(), "non-existent decision must return None");
}

// ── 11. High attempt_count (many providers tried) persists correctly ──────────

#[tokio::test]
async fn high_attempt_count_persists() {
    let store = InMemoryStore::new();

    // Simulate a routing decision that tried 5 providers before settling.
    store
        .append(&[evt(
            "e1",
            RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
                project: default_project(),
                route_decision_id: RouteDecisionId::new("rd_many_attempts"),
                operation_kind: OperationKind::Generate,
                selected_provider_binding_id: Some(ProviderBindingId::new("binding_last")),
                final_status: RouteDecisionStatus::Selected,
                attempt_count: 5,
                fallback_used: true,
                decided_at: now_ms(),
            }),
        )])
        .await
        .unwrap();

    let r = RouteDecisionReadModel::get(&store, &RouteDecisionId::new("rd_many_attempts"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(r.attempt_count, 5, "all 5 attempts recorded");
    assert!(
        r.fallback_used,
        "fallback was used when trying multiple providers"
    );
    assert_eq!(
        r.final_status,
        RouteDecisionStatus::Selected,
        "eventual success despite multiple attempts"
    );
}
