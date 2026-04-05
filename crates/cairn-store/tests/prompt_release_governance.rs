//! Prompt release governance integration tests (RFC 006).
//!
//! Validates the full prompt release lifecycle: asset → version → release →
//! governance transitions → rollout → archival.
//!
//! Note on state names: the manager referred to "InReview" but the domain
//! uses `Proposed` for the governance-review gate. The full lifecycle is:
//!   Draft → Proposed (submit for review)
//!          → Approved (reviewer approves)
//!            → Active (operator activates)
//!              → Archived (end of life)
//!   Draft → Approved (Standard preset shortcut, skips review)
//!   Proposed → Rejected → Archived (rejection path)
//!
//! Governance presets (RFC 006):
//!   Standard  — allows Draft→Approved shortcut
//!   Regulated — requires Draft→Proposed→Approved (no shortcut)
//!
//! PromptRolloutStarted:
//!   Sets rollout_percent on the release AND forces state="active".
//!   Used for gradual traffic ramp-up (partial rollout).

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, PromptAssetCreated,
    PromptAssetId, PromptReleaseCreated, PromptReleaseId, PromptReleaseTransitioned,
    PromptRolloutStarted, PromptVersionCreated, PromptVersionId, RuntimeEvent, TenantId,
    WorkspaceId,
};
use cairn_domain::prompts::{
    can_transition_prompt_release, PromptGovernancePreset, PromptReleaseState,
};
use cairn_store::{
    projections::{PromptAssetReadModel, PromptReleaseReadModel, PromptVersionReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_gov"),
        workspace_id: WorkspaceId::new("w_gov"),
        project_id: ProjectId::new("p_gov"),
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

/// Create the full asset→version→release stack in one append.
async fn create_release_stack(
    store: &InMemoryStore,
    asset_id: &str,
    version_id: &str,
    release_id: &str,
    ts: u64,
) {
    store
        .append(&[
            evt(
                &format!("ea_{release_id}"),
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: project(),
                    prompt_asset_id: PromptAssetId::new(asset_id),
                    name: format!("{asset_id} prompt"),
                    kind: "system".to_owned(),
                    created_at: ts,
                    workspace_id: project().workspace_id.clone(),
                }),
            ),
            evt(
                &format!("ev_{release_id}"),
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project(),
                    prompt_version_id: PromptVersionId::new(version_id),
                    prompt_asset_id: PromptAssetId::new(asset_id),
                    content_hash: format!("sha256:{version_id}"),
                    created_at: ts + 1,
                    workspace_id: project().workspace_id.clone(),
                }),
            ),
            evt(
                &format!("er_{release_id}"),
                RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
                    project: project(),
                    prompt_release_id: PromptReleaseId::new(release_id),
                    prompt_asset_id: PromptAssetId::new(asset_id),
                    prompt_version_id: PromptVersionId::new(version_id),
                    created_at: ts + 2,
            release_tag: None,
            created_by: None,
                }),
            ),
        ])
        .await
        .unwrap();
}

fn transition(
    evt_id: &str,
    release_id: &str,
    from: &str,
    to: &str,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
            project: project(),
            prompt_release_id: PromptReleaseId::new(release_id),
            from_state: from.to_owned(),
            to_state: to.to_owned(),
            transitioned_at: ts,
            actor: None,
            reason: None,
        }),
    )
}

// ── 1. Create asset + version + release → release starts as Draft ─────────────

#[tokio::test]
async fn create_asset_version_release_starts_as_draft() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_gov", "ver_gov", "rel_gov", ts).await;

    // All three records exist.
    let asset = PromptAssetReadModel::get(&store, &PromptAssetId::new("asset_gov"))
        .await
        .unwrap()
        .expect("asset must exist");
    assert_eq!(asset.prompt_asset_id.as_str(), "asset_gov");

    let version = PromptVersionReadModel::get(&store, &PromptVersionId::new("ver_gov"))
        .await
        .unwrap()
        .expect("version must exist");
    assert_eq!(version.prompt_version_id.as_str(), "ver_gov");
    assert_eq!(version.version_number, 1);

    let release = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_gov"))
        .await
        .unwrap()
        .expect("release must exist");
    assert_eq!(release.state, "draft", "new releases start as draft");
    assert!(release.rollout_percent.is_none(), "no rollout at creation");
    assert_eq!(release.prompt_asset_id.as_str(), "asset_gov");
    assert_eq!(release.prompt_version_id.as_str(), "ver_gov");
}

// ── 2. Full governance lifecycle: Draft → Proposed → Approved → Active ────────

#[tokio::test]
async fn full_governance_lifecycle_draft_to_active() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_full", "ver_full", "rel_full", ts).await;

    // Step 1: submit for review (Draft → Proposed).
    store
        .append(&[transition("t1", "rel_full", "draft", "proposed", ts + 10)])
        .await
        .unwrap();
    let r = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_full"))
        .await.unwrap().unwrap();
    assert_eq!(r.state, "proposed", "after review submission: state=proposed");

    // Step 2: reviewer approves (Proposed → Approved).
    store
        .append(&[transition("t2", "rel_full", "proposed", "approved", ts + 20)])
        .await
        .unwrap();
    let r = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_full"))
        .await.unwrap().unwrap();
    assert_eq!(r.state, "approved");

    // Step 3: operator activates (Approved → Active).
    store
        .append(&[transition("t3", "rel_full", "approved", "active", ts + 30)])
        .await
        .unwrap();
    let r = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_full"))
        .await.unwrap().unwrap();
    assert_eq!(r.state, "active");
    assert_eq!(r.updated_at, ts + 30, "updated_at tracks the latest transition");
}

// ── 3. Full lifecycle: Active → Archived ─────────────────────────────────────

#[tokio::test]
async fn active_release_can_be_archived() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_arch", "ver_arch", "rel_arch", ts).await;
    store
        .append(&[
            transition("t1", "rel_arch", "draft",    "approved", ts + 10),
            transition("t2", "rel_arch", "approved", "active",   ts + 20),
            transition("t3", "rel_arch", "active",   "archived", ts + 30),
        ])
        .await
        .unwrap();

    let r = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_arch"))
        .await.unwrap().unwrap();
    assert_eq!(r.state, "archived");
    assert_eq!(r.updated_at, ts + 30);
}

// ── 4. Valid transitions per can_transition_prompt_release (domain contract) ───

#[test]
fn valid_transitions_standard_governance() {
    use PromptGovernancePreset::Standard;
    use PromptReleaseState::*;

    // Standard shortcuts.
    assert!(can_transition_prompt_release(Draft,    Approved, Standard), "Draft→Approved allowed in Standard");
    assert!(can_transition_prompt_release(Draft,    Proposed, Standard), "Draft→Proposed");
    assert!(can_transition_prompt_release(Proposed, Approved, Standard), "Proposed→Approved");
    assert!(can_transition_prompt_release(Proposed, Rejected, Standard), "Proposed→Rejected");
    assert!(can_transition_prompt_release(Approved, Active,   Standard), "Approved→Active");
    assert!(can_transition_prompt_release(Active,   Approved, Standard), "Active→Approved (rollback)");
    assert!(can_transition_prompt_release(Active,   Archived, Standard), "Active→Archived");
    assert!(can_transition_prompt_release(Rejected, Archived, Standard), "Rejected→Archived");
}

#[test]
fn invalid_transitions_are_blocked_by_domain() {
    use PromptGovernancePreset::{Regulated, Standard};
    use PromptReleaseState::*;

    // Can never jump straight from Draft to Active.
    assert!(!can_transition_prompt_release(Draft,    Active,  Standard), "Draft→Active is invalid");
    assert!(!can_transition_prompt_release(Draft,    Active,  Regulated), "Draft→Active is invalid");
    // Archived is terminal.
    assert!(!can_transition_prompt_release(Archived, Active,  Standard), "Archived→Active is invalid");
    assert!(!can_transition_prompt_release(Archived, Draft,   Standard), "Archived→Draft is invalid");
    // Regulated blocks the Draft→Approved shortcut.
    assert!(!can_transition_prompt_release(Draft, Approved, Regulated),
        "Draft→Approved shortcut forbidden under Regulated governance");
    // Completed release cannot loop back to Draft.
    assert!(!can_transition_prompt_release(Active, Draft, Standard), "Active→Draft is invalid");
}

// ── 5. Regulated governance requires the full review path ────────────────────

#[test]
fn regulated_governance_requires_review_step() {
    use PromptGovernancePreset::{Regulated, Standard};
    use PromptReleaseState::*;

    // Standard allows the shortcut.
    assert!(can_transition_prompt_release(Draft, Approved, Standard));
    // Regulated requires Draft → Proposed first.
    assert!(!can_transition_prompt_release(Draft, Approved, Regulated));
    assert!(can_transition_prompt_release(Draft,    Proposed, Regulated));
    assert!(can_transition_prompt_release(Proposed, Approved, Regulated));
    assert!(can_transition_prompt_release(Approved, Active,   Regulated));
}

// ── 6. Rejection path: Proposed → Rejected → Archived ────────────────────────

#[tokio::test]
async fn rejection_path_proposed_to_rejected_to_archived() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_rej", "ver_rej", "rel_rej", ts).await;
    store
        .append(&[
            transition("t1", "rel_rej", "draft",    "proposed", ts + 10),
            transition("t2", "rel_rej", "proposed", "rejected", ts + 20),
            transition("t3", "rel_rej", "rejected", "archived", ts + 30),
        ])
        .await
        .unwrap();

    let r = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_rej"))
        .await.unwrap().unwrap();
    assert_eq!(r.state, "archived",
        "rejected releases must ultimately be archived");
}

// ── 7. PromptRolloutStarted: sets rollout_percent + forces state=active ────────

#[tokio::test]
async fn rollout_started_sets_percent_and_activates_release() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_roll", "ver_roll", "rel_roll", ts).await;
    // Get to Approved first (prerequisite for activation).
    store
        .append(&[
            transition("t1", "rel_roll", "draft",    "approved", ts + 10),
        ])
        .await
        .unwrap();

    // Operator starts a gradual rollout at 25%.
    store
        .append(&[evt(
            "rollout",
            RuntimeEvent::PromptRolloutStarted(PromptRolloutStarted {
                project: project(),
                prompt_release_id: PromptReleaseId::new("rel_roll"),
                percent: 25,
                started_at: ts + 20,
                release_id: None,
            }),
        )])
        .await
        .unwrap();

    let r = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_roll"))
        .await.unwrap().unwrap();

    assert_eq!(r.state, "active",
        "PromptRolloutStarted must force state=active");
    assert_eq!(r.rollout_percent, Some(25),
        "rollout_percent must be set to 25");
    assert_eq!(r.updated_at, ts + 20);
}

#[tokio::test]
async fn rollout_can_ramp_to_100_percent() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_ramp", "ver_ramp", "rel_ramp", ts).await;
    store.append(&[transition("t1", "rel_ramp", "draft", "approved", ts + 10)])
        .await.unwrap();

    // Ramp: 10% → 50% → 100%.
    for (i, pct) in [(1u64, 10u8), (2, 50), (3, 100)] {
        store
            .append(&[evt(
                &format!("rollout_{i}"),
                RuntimeEvent::PromptRolloutStarted(PromptRolloutStarted {
                    project: project(),
                    prompt_release_id: PromptReleaseId::new("rel_ramp"),
                    percent: pct,
                    started_at: ts + 20 + i,
                    release_id: None,
                }),
            )])
            .await
            .unwrap();
    }

    let r = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_ramp"))
        .await.unwrap().unwrap();

    assert_eq!(r.rollout_percent, Some(100), "final rollout_percent is 100");
    assert_eq!(r.state, "active");
}

// ── 8. list_by_project returns releases ordered by created_at ─────────────────

#[tokio::test]
async fn list_by_project_returns_releases_in_created_at_order() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Create 3 releases with distinct timestamps.
    for (i, (asset, ver, rel)) in [
        ("asset_o3", "ver_o3", "rel_o3"),
        ("asset_o1", "ver_o1", "rel_o1"),
        ("asset_o2", "ver_o2", "rel_o2"),
    ]
    .iter()
    .enumerate()
    {
        create_release_stack(&store, asset, ver, rel, ts + i as u64 * 100).await;
    }

    let releases = PromptReleaseReadModel::list_by_project(&store, &project(), 10, 0)
        .await
        .unwrap();

    assert_eq!(releases.len(), 3);
    // Sorted by created_at ascending.
    assert_eq!(releases[0].prompt_release_id.as_str(), "rel_o3", "earliest first");
    assert_eq!(releases[1].prompt_release_id.as_str(), "rel_o1");
    assert_eq!(releases[2].prompt_release_id.as_str(), "rel_o2", "latest last");
}

// ── 9. Active release is queryable via active_for_selector ────────────────────

#[tokio::test]
async fn active_release_is_returned_by_active_for_selector() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_sel", "ver_sel", "rel_sel", ts).await;
    store
        .append(&[
            transition("t1", "rel_sel", "draft",    "approved", ts + 10),
            transition("t2", "rel_sel", "approved", "active",   ts + 20),
        ])
        .await
        .unwrap();

    let found = PromptReleaseReadModel::active_for_selector(
        &store,
        &project(),
        &PromptAssetId::new("asset_sel"),
        "user_abc",
    )
    .await
    .unwrap()
    .expect("active release must be found by selector");

    assert_eq!(found.prompt_release_id.as_str(), "rel_sel");
    assert_eq!(found.state, "active");
}

#[tokio::test]
async fn draft_release_not_returned_by_active_for_selector() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_nodraft", "ver_nd", "rel_nd", ts).await;
    // Leave in Draft — do NOT activate.

    let none = PromptReleaseReadModel::active_for_selector(
        &store,
        &project(),
        &PromptAssetId::new("asset_nodraft"),
        "user_abc",
    )
    .await
    .unwrap();

    assert!(none.is_none(), "draft release must not be returned as active");
}

// ── 10. Rollout-activated release found by active_for_selector ────────────────

#[tokio::test]
async fn rollout_activated_release_is_found_by_selector() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_rsel", "ver_rsel", "rel_rsel", ts).await;
    store.append(&[transition("t1", "rel_rsel", "draft", "approved", ts + 10)])
        .await.unwrap();

    store
        .append(&[evt(
            "rollout",
            RuntimeEvent::PromptRolloutStarted(PromptRolloutStarted {
                project: project(),
                prompt_release_id: PromptReleaseId::new("rel_rsel"),
                percent: 50,
                started_at: ts + 20,
                release_id: None,
            }),
        )])
        .await
        .unwrap();

    let found = PromptReleaseReadModel::active_for_selector(
        &store,
        &project(),
        &PromptAssetId::new("asset_rsel"),
        "user_xyz",
    )
    .await
    .unwrap()
    .expect("rollout-activated release must be found by selector");

    assert_eq!(found.state, "active");
    assert_eq!(found.rollout_percent, Some(50));
}

// ── 11. list_by_project pagination ───────────────────────────────────────────

#[tokio::test]
async fn list_by_project_respects_limit_and_offset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u64..4 {
        create_release_stack(
            &store,
            &format!("a_pg_{i}"),
            &format!("v_pg_{i}"),
            &format!("r_pg_{i:02}"),
            ts + i * 10,
        )
        .await;
    }

    let page1 = PromptReleaseReadModel::list_by_project(&store, &project(), 2, 0)
        .await.unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].prompt_release_id.as_str(), "r_pg_00");
    assert_eq!(page1[1].prompt_release_id.as_str(), "r_pg_01");

    let page2 = PromptReleaseReadModel::list_by_project(&store, &project(), 2, 2)
        .await.unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].prompt_release_id.as_str(), "r_pg_02");
    assert_eq!(page2[1].prompt_release_id.as_str(), "r_pg_03");
}

// ── 12. Rollback: Active → Approved resets to Approved ───────────────────────

#[tokio::test]
async fn active_release_can_roll_back_to_approved() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    create_release_stack(&store, "asset_rb", "ver_rb", "rel_rb", ts).await;
    store
        .append(&[
            transition("t1", "rel_rb", "draft",    "approved", ts + 10),
            transition("t2", "rel_rb", "approved", "active",   ts + 20),
        ])
        .await
        .unwrap();

    // Verify active.
    let active = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_rb"))
        .await.unwrap().unwrap();
    assert_eq!(active.state, "active");

    // Operator rolls back.
    store
        .append(&[transition("t3", "rel_rb", "active", "approved", ts + 30)])
        .await
        .unwrap();

    let rolled_back = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_rb"))
        .await.unwrap().unwrap();
    assert_eq!(rolled_back.state, "approved",
        "after rollback: state=approved (not active)");

    // No longer returned by active_for_selector.
    let none = PromptReleaseReadModel::active_for_selector(
        &store,
        &project(),
        &PromptAssetId::new("asset_rb"),
        "user_abc",
    )
    .await
    .unwrap();
    assert!(none.is_none(), "rolled-back release must not appear as active");
}
