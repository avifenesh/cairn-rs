//! RFC 006 — Prompt release rollout lifecycle tests.
//!
//! Validates the full rollout pipeline through the event log and synchronous
//! projection:
//!
//! - `PromptAssetCreated` + `PromptVersionCreated` + `PromptReleaseCreated`
//!   create the three-tier prompt hierarchy; release starts in `"draft"`.
//! - `PromptReleaseTransitioned` advances state to `"active"`.
//! - `PromptRolloutStarted` sets `rollout_percent` and activates the release.
//! - `rollout_percent = 100` means full deployment.
//! - Rollout on a non-active (draft) release is stored but has no special
//!   guard — the test documents observed behaviour.
//! - `list_by_project` filtered to `"active"` state returns only live releases.

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, PromptAssetId,
    PromptReleaseId, PromptVersionId, RuntimeEvent, TenantId, WorkspaceId,
    events::{
        PromptAssetCreated, PromptReleaseCreated, PromptReleaseTransitioned,
        PromptRolloutStarted, PromptVersionCreated,
    },
    tenancy::OwnershipKey,
};
use cairn_store::{
    projections::{PromptAssetReadModel, PromptReleaseReadModel, PromptVersionReadModel},
    EventLog, InMemoryStore,
};

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("tenant_rfc006"),
        workspace_id: WorkspaceId::new("ws_rfc006"),
        project_id: ProjectId::new("proj_rfc006"),
    }
}

fn ownership() -> OwnershipKey {
    OwnershipKey::Project(project())
}

// ── Low-level event helpers ───────────────────────────────────────────────────

async fn append_asset(store: &InMemoryStore, event_id: &str, asset_id: &str) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
            project: project(),
            prompt_asset_id: PromptAssetId::new(asset_id),
            name: format!("Asset {asset_id}"),
            kind: "system".to_owned(),
            created_at: 1_000,
            workspace_id: project().workspace_id,
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn append_version(
    store: &InMemoryStore,
    event_id: &str,
    version_id: &str,
    asset_id: &str,
    content_hash: &str,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
            project: project(),
            prompt_version_id: PromptVersionId::new(version_id),
            prompt_asset_id: PromptAssetId::new(asset_id),
            content_hash: content_hash.to_owned(),
            created_at: 1_000,
            workspace_id: project().workspace_id,
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn append_release(
    store: &InMemoryStore,
    event_id: &str,
    release_id: &str,
    asset_id: &str,
    version_id: &str,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
            project: project(),
            prompt_release_id: PromptReleaseId::new(release_id),
            prompt_asset_id: PromptAssetId::new(asset_id),
            prompt_version_id: PromptVersionId::new(version_id),
            created_at: 1_000,
            release_tag: None,
            created_by: None,
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn transition_release(
    store: &InMemoryStore,
    event_id: &str,
    release_id: &str,
    from: &str,
    to: &str,
    at: u64,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
            project: project(),
            prompt_release_id: PromptReleaseId::new(release_id),
            from_state: from.to_owned(),
            to_state: to.to_owned(),
            transitioned_at: at,
            actor: None,
            reason: None,
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn start_rollout(
    store: &InMemoryStore,
    event_id: &str,
    release_id: &str,
    percent: u8,
    at: u64,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::PromptRolloutStarted(PromptRolloutStarted {
            project: project(),
            prompt_release_id: PromptReleaseId::new(release_id),
            percent,
            started_at: at,
            release_id: None,
        }),
    );
    store.append(&[env]).await.unwrap();
}

/// Full happy-path setup: asset → version → release → active.
async fn setup_active_release(
    store: &InMemoryStore,
    prefix: &str,
) -> (PromptAssetId, PromptVersionId, PromptReleaseId) {
    let asset_id   = PromptAssetId::new(format!("{prefix}_asset"));
    let version_id = PromptVersionId::new(format!("{prefix}_version"));
    let release_id = PromptReleaseId::new(format!("{prefix}_release"));

    append_asset(store, &format!("{prefix}_e1"), asset_id.as_str()).await;
    append_version(store, &format!("{prefix}_e2"), version_id.as_str(), asset_id.as_str(), "hash_abc").await;
    append_release(store, &format!("{prefix}_e3"), release_id.as_str(), asset_id.as_str(), version_id.as_str()).await;
    transition_release(store, &format!("{prefix}_e4"), release_id.as_str(), "draft", "active", 2_000).await;

    (asset_id, version_id, release_id)
}

// ── 1. Asset + version + release creation ─────────────────────────────────────

#[tokio::test]
async fn release_created_starts_in_draft_state() {
    let store = InMemoryStore::new();
    append_asset(&store, "e1", "asset_1").await;
    append_version(&store, "e2", "ver_1", "asset_1", "h1").await;
    append_release(&store, "e3", "rel_1", "asset_1", "ver_1").await;

    let rel = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_1"))
        .await.unwrap().expect("release must exist after PromptReleaseCreated");

    assert_eq!(rel.state, "draft", "newly created release must be in draft");
    assert!(rel.rollout_percent.is_none(), "rollout_percent must be None until rollout starts");
}

#[tokio::test]
async fn asset_and_version_are_stored_by_their_events() {
    let store = InMemoryStore::new();
    append_asset(&store, "e1", "asset_av").await;
    append_version(&store, "e2", "ver_av", "asset_av", "hash_av").await;

    let asset = PromptAssetReadModel::get(&store, &PromptAssetId::new("asset_av"))
        .await.unwrap().expect("asset must be stored");
    assert_eq!(asset.prompt_asset_id.as_str(), "asset_av");

    let ver = PromptVersionReadModel::get(&store, &PromptVersionId::new("ver_av"))
        .await.unwrap().expect("version must be stored");
    assert_eq!(ver.content_hash, "hash_av");
}

#[tokio::test]
async fn release_links_correct_asset_and_version() {
    let store = InMemoryStore::new();
    let (asset_id, version_id, release_id) = setup_active_release(&store, "link").await;

    let rel = PromptReleaseReadModel::get(&store, &release_id)
        .await.unwrap().unwrap();

    assert_eq!(rel.prompt_asset_id, asset_id);
    assert_eq!(rel.prompt_version_id, version_id);
    assert_eq!(rel.project, project());
}

// ── 2. Transition to Active ───────────────────────────────────────────────────

#[tokio::test]
async fn transition_to_active_updates_state() {
    let store = InMemoryStore::new();
    append_asset(&store, "e1", "asset_t").await;
    append_version(&store, "e2", "ver_t", "asset_t", "ht").await;
    append_release(&store, "e3", "rel_t", "asset_t", "ver_t").await;

    transition_release(&store, "e4", "rel_t", "draft", "active", 3_000).await;

    let rel = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_t"))
        .await.unwrap().unwrap();
    assert_eq!(rel.state, "active");
    assert_eq!(rel.updated_at, 3_000);
}

// ── 3. PromptRolloutStarted with percent=25 ───────────────────────────────────

#[tokio::test]
async fn rollout_25_sets_rollout_percent_and_activates() {
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "r25").await;

    start_rollout(&store, "r25_e5", release_id.as_str(), 25, 5_000).await;

    let rel = PromptReleaseReadModel::get(&store, &release_id)
        .await.unwrap().unwrap();

    assert_eq!(rel.rollout_percent, Some(25), "rollout_percent must be 25");
    assert_eq!(rel.state, "active", "state must remain active after rollout start");
    assert_eq!(rel.updated_at, 5_000);
}

#[tokio::test]
async fn rollout_percent_increases_with_successive_rollout_events() {
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "incr").await;

    for (pct, at) in [(10u8, 1_000u64), (50, 2_000), (75, 3_000), (100, 4_000)] {
        start_rollout(&store, &format!("incr_e_{pct}"), release_id.as_str(), pct, at).await;
        let rel = PromptReleaseReadModel::get(&store, &release_id)
            .await.unwrap().unwrap();
        assert_eq!(rel.rollout_percent, Some(pct), "rollout_percent should be {pct}");
    }
}

// ── 4. rollout_percent = 100 means full deployment ────────────────────────────

#[tokio::test]
async fn rollout_100_means_full_deployment() {
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "full").await;

    start_rollout(&store, "full_e5", release_id.as_str(), 100, 9_000).await;

    let rel = PromptReleaseReadModel::get(&store, &release_id)
        .await.unwrap().unwrap();

    assert_eq!(rel.rollout_percent, Some(100), "100% rollout = full deployment");
    assert_eq!(rel.state, "active");
}

#[tokio::test]
async fn rollout_0_percent_is_stored_but_release_stays_active() {
    // A 0% rollout is valid (pre-stage before any traffic is sent).
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "zero").await;

    start_rollout(&store, "zero_e5", release_id.as_str(), 0, 1_000).await;

    let rel = PromptReleaseReadModel::get(&store, &release_id)
        .await.unwrap().unwrap();

    assert_eq!(rel.rollout_percent, Some(0));
    assert_eq!(rel.state, "active");
}

// ── 5. Rollout on a non-active (draft) release ────────────────────────────────

#[tokio::test]
async fn rollout_on_draft_release_activates_it() {
    // The projection does not guard against rolling out a draft release —
    // `PromptRolloutStarted` sets state = "active" regardless of prior state.
    // This test documents that observed behaviour.
    let store = InMemoryStore::new();
    append_asset(&store, "e1", "asset_d").await;
    append_version(&store, "e2", "ver_d", "asset_d", "hd").await;
    append_release(&store, "e3", "rel_d", "asset_d", "ver_d").await;

    // Release is still "draft" — no transition event.
    let before = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_d"))
        .await.unwrap().unwrap();
    assert_eq!(before.state, "draft");

    start_rollout(&store, "e4", "rel_d", 25, 2_000).await;

    let after = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_d"))
        .await.unwrap().unwrap();

    // The rollout forces it to "active" — document that this is the current
    // projection behaviour. The service layer (not the store) would enforce
    // the state guard.
    assert_eq!(after.state, "active",
        "PromptRolloutStarted forces state=active even on a draft release");
    assert_eq!(after.rollout_percent, Some(25));
}

#[tokio::test]
async fn rollout_on_unknown_release_is_a_no_op() {
    let store = InMemoryStore::new();
    start_rollout(&store, "e1", "ghost_release", 50, 1_000).await;

    let result = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("ghost_release"))
        .await.unwrap();
    assert!(result.is_none(), "rollout on unknown release must not create a record");
}

#[tokio::test]
async fn rollout_on_retired_release_updates_it() {
    // A retired release that gets a rollout event — projection records it
    // but returns the updated state. Service layer owns the guard.
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "retired").await;
    transition_release(&store, "retired_e5", release_id.as_str(), "active", "retired", 3_000).await;

    let before = PromptReleaseReadModel::get(&store, &release_id)
        .await.unwrap().unwrap();
    assert_eq!(before.state, "retired");

    start_rollout(&store, "retired_e6", release_id.as_str(), 10, 4_000).await;

    let after = PromptReleaseReadModel::get(&store, &release_id)
        .await.unwrap().unwrap();
    // Documents projection behaviour — state snaps to "active" again.
    assert_eq!(after.rollout_percent, Some(10));
}

// ── 6. list_active_releases (list_by_project filtered to active state) ────────

#[tokio::test]
async fn list_by_project_returns_only_active_releases_when_filtered() {
    let store = InMemoryStore::new();

    // Create 3 releases: one active (via transition), one active (via rollout),
    // one remaining draft.
    let (_, _, rel_via_transition) = setup_active_release(&store, "act1").await;

    // Second release: active via rollout only (never explicitly transitioned).
    append_asset(&store, "r2_e1", "asset_r2").await;
    append_version(&store, "r2_e2", "ver_r2", "asset_r2", "hr2").await;
    append_release(&store, "r2_e3", "rel_r2", "asset_r2", "ver_r2").await;
    start_rollout(&store, "r2_e4", "rel_r2", 50, 2_000).await;

    // Third release: stays in draft.
    append_asset(&store, "r3_e1", "asset_r3").await;
    append_version(&store, "r3_e2", "ver_r3", "asset_r3", "hr3").await;
    append_release(&store, "r3_e3", "rel_r3", "asset_r3", "ver_r3").await;

    let all = PromptReleaseReadModel::list_by_project(&store, &project(), 100, 0)
        .await.unwrap();
    assert_eq!(all.len(), 3, "all 3 releases must appear in list_by_project");

    let active: Vec<_> = all.iter().filter(|r| r.state == "active").collect();
    assert_eq!(active.len(), 2, "exactly 2 active releases");

    let active_ids: Vec<&str> = active.iter().map(|r| r.prompt_release_id.as_str()).collect();
    assert!(active_ids.contains(&rel_via_transition.as_str()));
    assert!(active_ids.contains(&"rel_r2"));
    assert!(!active_ids.contains(&"rel_r3"), "draft release must not be in active list");
}

#[tokio::test]
async fn draft_only_project_has_no_active_releases() {
    let store = InMemoryStore::new();
    append_asset(&store, "e1", "asset_draft").await;
    append_version(&store, "e2", "ver_draft", "asset_draft", "hd").await;
    append_release(&store, "e3", "rel_draft", "asset_draft", "ver_draft").await;

    let all = PromptReleaseReadModel::list_by_project(&store, &project(), 100, 0)
        .await.unwrap();
    let active: Vec<_> = all.iter().filter(|r| r.state == "active").collect();
    assert!(active.is_empty());
}

#[tokio::test]
async fn retired_releases_are_excluded_from_active_filter() {
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "retire").await;

    // Retire the release.
    transition_release(&store, "retire_e5", release_id.as_str(), "active", "retired", 5_000).await;

    let all = PromptReleaseReadModel::list_by_project(&store, &project(), 100, 0)
        .await.unwrap();
    let active: Vec<_> = all.iter().filter(|r| r.state == "active").collect();
    assert!(active.is_empty(), "retired release must not appear in active list");
}

// ── 7. Event log completeness ─────────────────────────────────────────────────

#[tokio::test]
async fn all_lifecycle_events_are_written_to_log() {
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "log_check").await;
    start_rollout(&store, "log_check_e5", release_id.as_str(), 50, 6_000).await;

    let all = store.read_stream(None, 100).await.unwrap();
    // asset + version + release + transition + rollout = 5 events
    assert_eq!(all.len(), 5);

    let types: Vec<bool> = all.iter().map(|e| matches!(&e.envelope.payload,
        RuntimeEvent::PromptAssetCreated(_)   |
        RuntimeEvent::PromptVersionCreated(_) |
        RuntimeEvent::PromptReleaseCreated(_) |
        RuntimeEvent::PromptReleaseTransitioned(_) |
        RuntimeEvent::PromptRolloutStarted(_)
    )).collect();
    assert!(types.iter().all(|&t| t), "all 5 events must be of expected types");
}

#[tokio::test]
async fn rollout_percent_persists_after_second_rollout_event() {
    let store = InMemoryStore::new();
    let (_, _, release_id) = setup_active_release(&store, "persist").await;

    start_rollout(&store, "persist_e5", release_id.as_str(), 25, 1_000).await;
    start_rollout(&store, "persist_e6", release_id.as_str(), 75, 2_000).await;

    let rel = PromptReleaseReadModel::get(&store, &release_id)
        .await.unwrap().unwrap();
    assert_eq!(rel.rollout_percent, Some(75), "second rollout event must override first");
    assert_eq!(rel.updated_at, 2_000);
}
