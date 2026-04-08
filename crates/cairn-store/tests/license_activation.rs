//! RFC 014 — License activation and entitlement override tests.
//!
//! Validates the full commercial lifecycle through the event log and
//! synchronous projection:
//!
//! - `LicenseActivated` populates `LicenseReadModel` with tier and timestamp
//!   fields (`valid_from_ms`, `valid_until_ms`).
//! - `LicenseRecord` carries `issued_at` (≡ valid_from) and `expires_at`
//!   (≡ valid_until) without data loss.
//! - `EntitlementOverrideSet` creates an override record linked to the tenant.
//! - `list_overrides` returns all overrides scoped to the requesting tenant.
//! - Multiple overrides for the same tenant coexist independently.
//! - Cross-tenant isolation: each tenant's license and overrides are private.

use cairn_domain::{
    commercial::ProductTier,
    events::{EntitlementOverrideSet, LicenseActivated},
    tenancy::OwnershipKey,
    EventEnvelope, EventId, EventSource, RuntimeEvent, TenantId,
};
use cairn_store::{projections::LicenseReadModel, EventLog, InMemoryStore};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tenant_ownership(tenant_id: &str) -> OwnershipKey {
    OwnershipKey::Tenant(cairn_domain::tenancy::TenantKey::new(TenantId::new(
        tenant_id,
    )))
}

async fn activate_license(
    store: &InMemoryStore,
    event_id: &str,
    tenant_id: &str,
    license_id: &str,
    tier: ProductTier,
    valid_from_ms: u64,
    valid_until_ms: Option<u64>,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        tenant_ownership(tenant_id),
        RuntimeEvent::LicenseActivated(LicenseActivated {
            tenant_id: TenantId::new(tenant_id),
            license_id: license_id.to_owned(),
            tier,
            valid_from_ms,
            valid_until_ms,
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn set_override(
    store: &InMemoryStore,
    event_id: &str,
    tenant_id: &str,
    feature: &str,
    allowed: bool,
    reason: Option<&str>,
    at: u64,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        tenant_ownership(tenant_id),
        RuntimeEvent::EntitlementOverrideSet(EntitlementOverrideSet {
            tenant_id: TenantId::new(tenant_id),
            feature: feature.to_owned(),
            allowed,
            reason: reason.map(ToOwned::to_owned),
            set_at_ms: at,
        }),
    );
    store.append(&[env]).await.unwrap();
}

// ── 1. LicenseActivated populates the read model ──────────────────────────────

#[tokio::test]
async fn license_activated_appears_in_read_model() {
    let store = InMemoryStore::new();

    activate_license(
        &store,
        "e1",
        "tenant_lic",
        "lic_001",
        ProductTier::TeamSelfHosted,
        1_000,
        None,
    )
    .await;

    let record = LicenseReadModel::get_active(&store, &TenantId::new("tenant_lic"))
        .await
        .unwrap()
        .expect("license must exist after LicenseActivated");

    assert_eq!(record.tenant_id.as_str(), "tenant_lic");
    assert_eq!(record.license_key.as_deref(), Some("lic_001"));
}

#[tokio::test]
async fn get_active_returns_none_for_unknown_tenant() {
    let store = InMemoryStore::new();
    let result = LicenseReadModel::get_active(&store, &TenantId::new("ghost_tenant"))
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn activated_license_stores_correct_product_tier() {
    let store = InMemoryStore::new();

    for (event_id, tenant, tier) in [
        ("e1", "tenant_local", ProductTier::LocalEval),
        ("e2", "tenant_team", ProductTier::TeamSelfHosted),
        ("e3", "tenant_enterprise", ProductTier::EnterpriseSelfHosted),
    ] {
        activate_license(&store, event_id, tenant, "lic", tier, 1_000, None).await;
        let rec = LicenseReadModel::get_active(&store, &TenantId::new(tenant))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rec.tier, tier, "tier must be preserved for {tenant}");
    }
}

// ── 3. License carries valid_from / valid_until timestamps ────────────────────

#[tokio::test]
async fn license_stores_valid_from_as_issued_at() {
    let store = InMemoryStore::new();
    activate_license(
        &store,
        "e1",
        "tenant_ts",
        "lic_ts",
        ProductTier::TeamSelfHosted,
        7_000,
        None,
    )
    .await;

    let rec = LicenseReadModel::get_active(&store, &TenantId::new("tenant_ts"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(rec.issued_at, 7_000, "issued_at must equal valid_from_ms");
}

#[tokio::test]
async fn license_stores_valid_until_as_expires_at_when_set() {
    let store = InMemoryStore::new();
    activate_license(
        &store,
        "e1",
        "tenant_exp",
        "lic_exp",
        ProductTier::EnterpriseSelfHosted,
        1_000,
        Some(99_999),
    )
    .await;

    let rec = LicenseReadModel::get_active(&store, &TenantId::new("tenant_exp"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(rec.issued_at, 1_000, "issued_at must be valid_from_ms");
    assert_eq!(
        rec.expires_at,
        Some(99_999),
        "expires_at must be valid_until_ms"
    );
}

#[tokio::test]
async fn license_with_no_expiry_has_expires_at_none() {
    let store = InMemoryStore::new();
    activate_license(
        &store,
        "e1",
        "tenant_noexp",
        "lic_noexp",
        ProductTier::LocalEval,
        5_000,
        None,
    )
    .await;

    let rec = LicenseReadModel::get_active(&store, &TenantId::new("tenant_noexp"))
        .await
        .unwrap()
        .unwrap();

    assert!(
        rec.expires_at.is_none(),
        "perpetual license must have expires_at = None"
    );
}

#[tokio::test]
async fn license_reactivation_overwrites_previous_record() {
    let store = InMemoryStore::new();

    activate_license(
        &store,
        "e1",
        "tenant_reup",
        "old_lic",
        ProductTier::LocalEval,
        1_000,
        Some(5_000),
    )
    .await;

    activate_license(
        &store,
        "e2",
        "tenant_reup",
        "new_lic",
        ProductTier::EnterpriseSelfHosted,
        6_000,
        None,
    )
    .await;

    let rec = LicenseReadModel::get_active(&store, &TenantId::new("tenant_reup"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        rec.license_key.as_deref(),
        Some("new_lic"),
        "second activation must replace first"
    );
    assert_eq!(rec.tier, ProductTier::EnterpriseSelfHosted);
    assert_eq!(rec.issued_at, 6_000);
    assert!(rec.expires_at.is_none());
}

// ── 4. EntitlementOverrideSet stores override ─────────────────────────────────

#[tokio::test]
async fn entitlement_override_appears_in_list_overrides() {
    let store = InMemoryStore::new();

    activate_license(
        &store,
        "e1",
        "tenant_ov",
        "lic_ov",
        ProductTier::TeamSelfHosted,
        1_000,
        None,
    )
    .await;
    set_override(
        &store,
        "e2",
        "tenant_ov",
        "eval_matrices",
        true,
        None,
        2_000,
    )
    .await;

    let overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_ov"))
        .await
        .unwrap();

    assert_eq!(overrides.len(), 1, "one override must be present");
    assert_eq!(overrides[0].feature, "eval_matrices");
    assert!(overrides[0].allowed);
}

#[tokio::test]
async fn list_overrides_returns_empty_for_tenant_with_no_overrides() {
    let store = InMemoryStore::new();
    activate_license(
        &store,
        "e1",
        "tenant_clean",
        "lic_c",
        ProductTier::LocalEval,
        1_000,
        None,
    )
    .await;

    let overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_clean"))
        .await
        .unwrap();
    assert!(overrides.is_empty());
}

// ── 5. Override linked to correct tenant ──────────────────────────────────────

#[tokio::test]
async fn override_carries_correct_tenant_id() {
    let store = InMemoryStore::new();
    set_override(
        &store,
        "e1",
        "tenant_link",
        "multi_provider",
        true,
        Some("pilot programme"),
        3_000,
    )
    .await;

    let overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_link"))
        .await
        .unwrap();

    assert_eq!(overrides.len(), 1);
    assert_eq!(
        overrides[0].tenant_id.as_str(),
        "tenant_link",
        "override must be linked to the correct tenant"
    );
    assert_eq!(overrides[0].reason.as_deref(), Some("pilot programme"));
    assert_eq!(overrides[0].set_at_ms, 3_000);
}

#[tokio::test]
async fn disabling_override_is_stored_with_allowed_false() {
    let store = InMemoryStore::new();
    set_override(
        &store,
        "e1",
        "tenant_deny",
        "eval_matrices",
        false,
        Some("plan downgrade"),
        9_000,
    )
    .await;

    let overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_deny"))
        .await
        .unwrap();

    assert!(
        !overrides[0].allowed,
        "disabled override must have allowed = false"
    );
    assert_eq!(overrides[0].reason.as_deref(), Some("plan downgrade"));
}

#[tokio::test]
async fn override_on_same_feature_replaces_previous() {
    let store = InMemoryStore::new();
    set_override(
        &store,
        "e1",
        "tenant_replace",
        "multi_provider",
        true,
        None,
        1_000,
    )
    .await;
    set_override(
        &store,
        "e2",
        "tenant_replace",
        "multi_provider",
        false,
        Some("revoked"),
        2_000,
    )
    .await;

    let overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_replace"))
        .await
        .unwrap();

    // The projection keys on (tenant, feature) so the second write replaces the first.
    assert_eq!(overrides.len(), 1, "same feature override must be upserted");
    assert!(
        !overrides[0].allowed,
        "latest override must take precedence"
    );
    assert_eq!(overrides[0].reason.as_deref(), Some("revoked"));
}

// ── 6. list_overrides returns all overrides ───────────────────────────────────

#[tokio::test]
async fn list_overrides_returns_all_features_for_tenant() {
    let store = InMemoryStore::new();

    for (i, feature) in ["eval_matrices", "multi_provider", "governance_compliance"]
        .iter()
        .enumerate()
    {
        set_override(
            &store,
            &format!("e{i}"),
            "tenant_all",
            feature,
            true,
            None,
            1_000 + i as u64,
        )
        .await;
    }

    let overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_all"))
        .await
        .unwrap();

    assert_eq!(
        overrides.len(),
        3,
        "all 3 feature overrides must be returned"
    );
    let features: Vec<&str> = overrides.iter().map(|o| o.feature.as_str()).collect();
    assert!(features.contains(&"eval_matrices"));
    assert!(features.contains(&"multi_provider"));
    assert!(features.contains(&"governance_compliance"));
}

#[tokio::test]
async fn list_overrides_scoped_to_tenant_only() {
    let store = InMemoryStore::new();

    set_override(
        &store,
        "e1",
        "tenant_scope_a",
        "feature_x",
        true,
        None,
        1_000,
    )
    .await;
    set_override(
        &store,
        "e2",
        "tenant_scope_b",
        "feature_y",
        false,
        None,
        1_000,
    )
    .await;

    let a_overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_scope_a"))
        .await
        .unwrap();
    let b_overrides = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_scope_b"))
        .await
        .unwrap();

    assert_eq!(a_overrides.len(), 1);
    assert_eq!(b_overrides.len(), 1);
    assert_eq!(a_overrides[0].feature, "feature_x");
    assert_eq!(b_overrides[0].feature, "feature_y");
}

// ── 7. Cross-tenant isolation ──────────────────────────────────────────────────

#[tokio::test]
async fn license_is_scoped_to_individual_tenant() {
    let store = InMemoryStore::new();

    activate_license(
        &store,
        "e1",
        "tenant_iso_a",
        "lic_a",
        ProductTier::TeamSelfHosted,
        1_000,
        Some(10_000),
    )
    .await;
    activate_license(
        &store,
        "e2",
        "tenant_iso_b",
        "lic_b",
        ProductTier::EnterpriseSelfHosted,
        2_000,
        None,
    )
    .await;

    let rec_a = LicenseReadModel::get_active(&store, &TenantId::new("tenant_iso_a"))
        .await
        .unwrap()
        .unwrap();
    let rec_b = LicenseReadModel::get_active(&store, &TenantId::new("tenant_iso_b"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        rec_a.tier,
        ProductTier::TeamSelfHosted,
        "tenant_a tier must be independent"
    );
    assert_eq!(
        rec_b.tier,
        ProductTier::EnterpriseSelfHosted,
        "tenant_b tier must be independent"
    );
    assert_eq!(rec_a.expires_at, Some(10_000));
    assert!(rec_b.expires_at.is_none());
}

#[tokio::test]
async fn overrides_of_different_tenants_do_not_bleed() {
    let store = InMemoryStore::new();

    set_override(&store, "e1", "tenant_p", "eval_matrices", true, None, 1_000).await;
    set_override(
        &store,
        "e2",
        "tenant_q",
        "eval_matrices",
        false,
        None,
        1_000,
    )
    .await;

    let p = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_p"))
        .await
        .unwrap();
    let q = LicenseReadModel::list_overrides(&store, &TenantId::new("tenant_q"))
        .await
        .unwrap();

    assert!(p[0].allowed, "tenant_p override must be allowed");
    assert!(!q[0].allowed, "tenant_q override must be denied");
}

// ── 8. Event log completeness ─────────────────────────────────────────────────

#[tokio::test]
async fn license_events_are_written_to_log() {
    let store = InMemoryStore::new();

    activate_license(
        &store,
        "e1",
        "tenant_log",
        "lic_log",
        ProductTier::TeamSelfHosted,
        1_000,
        Some(5_000),
    )
    .await;
    set_override(
        &store,
        "e2",
        "tenant_log",
        "eval_matrices",
        true,
        None,
        2_000,
    )
    .await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 2);

    assert!(
        matches!(&all[0].envelope.payload, RuntimeEvent::LicenseActivated(ev)
        if ev.tenant_id.as_str() == "tenant_log" && ev.valid_from_ms == 1_000)
    );
    assert!(
        matches!(&all[1].envelope.payload, RuntimeEvent::EntitlementOverrideSet(ev)
        if ev.tenant_id.as_str() == "tenant_log" && ev.feature == "eval_matrices")
    );
}
