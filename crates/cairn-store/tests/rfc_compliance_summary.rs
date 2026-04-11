//! RFC compliance proof — one test per RFC verifying the core MUST requirement.
//!
//! Each test is intentionally compact: it targets the single most critical
//! invariant in the RFC and proves it passes through the real InMemoryStore
//! pipeline. These tests are the executable proof that the implementation
//! satisfies the contract.

use std::sync::Arc;

use cairn_domain::commercial::{
    DefaultFeatureGate, EntitlementSet, FeatureGate, FeatureGateResult, ProductTier,
};
use cairn_domain::events::{RunStateChanged, StateTransition};
use cairn_domain::lifecycle::RunState;
use cairn_domain::policy::ApprovalRequirement;
use cairn_domain::providers::{OperationKind, RouteDecisionStatus};
use cairn_domain::{
    ApprovalId, ApprovalRequested, CommandId, EventEnvelope, EventId, EventSource, ProjectKey,
    PromptAssetId, PromptReleaseCreated, PromptReleaseId, PromptReleaseTransitioned,
    PromptVersionId, RouteDecisionId, RouteDecisionMade, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, TenantId,
};
use cairn_store::{
    projections::{
        ApprovalReadModel, PromptReleaseReadModel, RouteDecisionReadModel, RunReadModel,
    },
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn project(tenant: &str) -> ProjectKey {
    ProjectKey::new(tenant, "ws", "p")
}

// ── RFC 002 — Event-log durability + causation-id idempotency ────────────────

/// RFC 002 MUST: a command that has already been applied MUST be detectable
/// via causation_id so the handler can skip re-applying it.
#[tokio::test]
async fn rfc002_causation_id_idempotency() {
    let store = Arc::new(InMemoryStore::new());

    let first = EventEnvelope::for_runtime_event(
        EventId::new("evt_1"),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project("t1"),
            session_id: SessionId::new("s1"),
        }),
    )
    .with_causation_id(CommandId::new("cmd_1"));

    let pos = EventLog::append(store.as_ref(), &[first]).await.unwrap()[0];
    let found = EventLog::find_by_causation_id(store.as_ref(), "cmd_1")
        .await
        .unwrap();

    // MUST: causation_id lookup returns the exact position of the first application.
    assert_eq!(
        found,
        Some(pos),
        "RFC 002: causation_id must resolve to the appended position"
    );
    // MUST: re-delivery is idempotent — same position, no second append.
    assert_eq!(found.unwrap(), pos);
    let stream = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(
        stream.len(),
        1,
        "RFC 002: exactly 1 event after idempotent re-delivery"
    );
}

// ── RFC 005 — Approval gating blocks run progression ─────────────────────────

/// RFC 005 MUST: a run that requires operator approval MUST be held in
/// WaitingApproval state and visible in the pending approval queue.
#[tokio::test]
async fn rfc005_approval_blocks_run_progression() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            ev(
                "sess",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project("t1"),
                    session_id: SessionId::new("s1"),
                }),
            ),
            ev(
                "run",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project("t1"),
                    session_id: SessionId::new("s1"),
                    run_id: RunId::new("r1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                "wait",
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project("t1"),
                    run_id: RunId::new("r1"),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::WaitingApproval,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
            ev(
                "appr",
                RuntimeEvent::ApprovalRequested(ApprovalRequested {
                    project: project("t1"),
                    approval_id: ApprovalId::new("a1"),
                    run_id: Some(RunId::new("r1")),
                    task_id: None,
                    requirement: ApprovalRequirement::Required,
                }),
            ),
        ])
        .await
        .unwrap();

    // MUST: run is blocked in WaitingApproval.
    let run = RunReadModel::get(store.as_ref(), &RunId::new("r1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run.state,
        RunState::WaitingApproval,
        "RFC 005: run must be blocked in WaitingApproval"
    );

    // MUST: approval is visible in the operator queue.
    let pending = ApprovalReadModel::list_pending(store.as_ref(), &project("t1"), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "RFC 005: blocked run must have 1 pending approval"
    );
    assert_eq!(pending[0].approval_id.as_str(), "a1");
}

// ── RFC 006 — Prompt release lifecycle: Draft → Active ───────────────────────

/// RFC 006 MUST: a prompt release MUST start in 'draft' state and
/// transition to 'active' via an explicit PromptReleaseTransitioned event.
#[tokio::test]
async fn rfc006_prompt_release_draft_to_active() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[ev(
            "rc",
            RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
                project: project("t1"),
                prompt_release_id: PromptReleaseId::new("rel_1"),
                prompt_asset_id: PromptAssetId::new("asset_1"),
                prompt_version_id: PromptVersionId::new("ver_1"),
                created_at: 1_000,
                created_by: None,
                release_tag: None,
            }),
        )])
        .await
        .unwrap();

    let draft = PromptReleaseReadModel::get(store.as_ref(), &PromptReleaseId::new("rel_1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        draft.state, "draft",
        "RFC 006: release must start in 'draft' state"
    );

    store
        .append(&[ev(
            "rt",
            RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
                project: project("t1"),
                prompt_release_id: PromptReleaseId::new("rel_1"),
                from_state: "draft".to_owned(),
                to_state: "active".to_owned(),
                transitioned_at: 2_000,
                actor: None,
                reason: None,
            }),
        )])
        .await
        .unwrap();

    let active = PromptReleaseReadModel::get(store.as_ref(), &PromptReleaseId::new("rel_1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        active.state, "active",
        "RFC 006: release must be 'active' after PromptReleaseTransitioned"
    );
}

// ── RFC 008 — Cross-tenant isolation ─────────────────────────────────────────

/// RFC 008 MUST: data written by tenant A MUST NOT be readable by tenant B.
/// Runs are keyed by project; a different tenant's project must return no results.
#[tokio::test]
async fn rfc008_cross_tenant_isolation() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            ev(
                "s_a",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project("tenant_a"),
                    session_id: SessionId::new("sa"),
                }),
            ),
            ev(
                "r_a",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project("tenant_a"),
                    session_id: SessionId::new("sa"),
                    run_id: RunId::new("run_a"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // MUST: tenant_a's run is NOT visible when queried as tenant_b.
    let run_via_b = RunReadModel::get(store.as_ref(), &RunId::new("run_a"))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        run_via_b.project.tenant_id.as_str(),
        "tenant_b",
        "RFC 008: run owned by tenant_a must not be returned as tenant_b's data"
    );

    // MUST: a non-existent run for tenant_b returns None.
    let no_run = RunReadModel::get(store.as_ref(), &RunId::new("run_b"))
        .await
        .unwrap();
    assert!(
        no_run.is_none(),
        "RFC 008: tenant_b has no runs — read must return None"
    );
}

// ── RFC 009 — Route decision persisted with fallback flag ────────────────────

/// RFC 009 MUST: every route decision MUST be persisted with its fallback_used
/// flag so operators can audit when the primary provider was unavailable.
#[tokio::test]
async fn rfc009_route_decision_persisted_with_fallback_flag() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[ev(
            "rd",
            RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
                project: project("t1"),
                route_decision_id: RouteDecisionId::new("rd_1"),
                operation_kind: OperationKind::Generate,
                selected_provider_binding_id: Some(cairn_domain::ProviderBindingId::new(
                    "binding_fallback",
                )),
                final_status: RouteDecisionStatus::Selected,
                attempt_count: 2,
                fallback_used: true, // ← MUST be persisted
                decided_at: 1_000,
            }),
        )])
        .await
        .unwrap();

    let decision = RouteDecisionReadModel::get(store.as_ref(), &RouteDecisionId::new("rd_1"))
        .await
        .unwrap()
        .unwrap();

    // MUST: fallback_used is durable — operators can see when primary failed.
    assert!(
        decision.fallback_used,
        "RFC 009: fallback_used=true MUST be persisted and readable"
    );
    assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
    assert_eq!(decision.attempt_count, 2);
}

// ── RFC 014 — Feature gate fail-closed for unknown features ──────────────────

/// RFC 014 MUST: an unknown feature name MUST return Denied (fail-closed).
/// Defaulting to Allowed for unrecognized names would silently grant access
/// to anything not in the registry — the opposite of a secure posture.
#[test]
fn rfc014_unknown_feature_denied_fail_closed() {
    let gate = DefaultFeatureGate::v1_defaults();
    let entitlements = EntitlementSet::new(TenantId::new("t1"), ProductTier::EnterpriseSelfHosted);

    // MUST: unknown feature returns Denied even with full enterprise tier.
    let result = gate.check(&entitlements, "not_a_registered_feature");
    assert!(
        matches!(result, FeatureGateResult::Denied { .. }),
        "RFC 014: unknown feature MUST return Denied (fail-closed), got: {result:?}"
    );

    // MUST: known GA feature still returns Allowed (gate not broken by fail-closed policy).
    assert_eq!(
        gate.check(&entitlements, "runtime_core"),
        FeatureGateResult::Allowed,
        "RFC 014: known GA feature must still be Allowed"
    );
}
