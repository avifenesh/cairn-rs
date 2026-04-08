//! RFC 002 credential rotation lifecycle integration tests.
//!
//! Validates the credential management pipeline through InMemoryStore:
//! - CredentialStored creates an active CredentialRecord.
//! - CredentialKeyRotated appends a CredentialRotationRecord with old/new key IDs.
//! - CredentialRevoked marks the record inactive with revoked_at_ms.
//! - Cross-tenant isolation: each tenant sees only its own credentials.

use std::sync::Arc;

use cairn_domain::events::{CredentialKeyRotated, CredentialRevoked, CredentialStored};
use cairn_domain::{CredentialId, EventEnvelope, EventId, EventSource, RuntimeEvent, TenantId};
use cairn_store::{
    projections::{CredentialReadModel, CredentialRotationReadModel},
    EventLog, InMemoryStore,
};

/// Type-disambiguated helpers to avoid E0782 ambiguity between
/// CredentialReadModel::list_by_tenant and CredentialRotationReadModel::list_by_tenant.
async fn list_credentials(
    store: &InMemoryStore,
    tenant_id: &cairn_domain::TenantId,
    limit: usize,
    offset: usize,
) -> Vec<cairn_domain::credentials::CredentialRecord> {
    <InMemoryStore as CredentialReadModel>::list_by_tenant(store, tenant_id, limit, offset)
        .await
        .unwrap()
}

async fn list_rotations(
    store: &InMemoryStore,
    tenant_id: &cairn_domain::TenantId,
) -> Vec<cairn_domain::credentials::CredentialRotationRecord> {
    CredentialRotationReadModel::list_rotations(store, tenant_id)
        .await
        .unwrap()
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant(n: &str) -> TenantId {
    TenantId::new(format!("tenant_cred_{n}"))
}
fn cred_id(n: &str) -> CredentialId {
    CredentialId::new(format!("cred_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::System, payload.into())
}

fn store_event(
    n: &str,
    tenant_n: &str,
    provider: &str,
    key_id: Option<&str>,
    key_version: Option<&str>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_store_{n}"),
        RuntimeEvent::CredentialStored(CredentialStored {
            tenant_id: tenant(tenant_n),
            credential_id: cred_id(n),
            provider_id: provider.to_owned(),
            encrypted_value: vec![0xCA, 0xFE, 0xBA, 0xBE], // mock ciphertext
            key_id: key_id.map(str::to_owned),
            key_version: key_version.map(str::to_owned),
            encrypted_at_ms: ts,
        }),
    )
}

fn rotate_event(
    rotation_id: &str,
    tenant_n: &str,
    old_key: &str,
    new_key: &str,
    cred_ids: &[&str],
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_rotate_{rotation_id}"),
        RuntimeEvent::CredentialKeyRotated(CredentialKeyRotated {
            tenant_id: tenant(tenant_n),
            rotation_id: rotation_id.to_owned(),
            old_key_id: old_key.to_owned(),
            new_key_id: new_key.to_owned(),
            credential_ids_rotated: cred_ids.iter().map(|s| s.to_string()).collect(),
        }),
    )
}

fn revoke_event(n: &str, tenant_n: &str, ts: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_revoke_{n}"),
        RuntimeEvent::CredentialRevoked(CredentialRevoked {
            tenant_id: tenant(tenant_n),
            credential_id: cred_id(n),
            revoked_at_ms: ts,
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): CredentialStored creates an active record with all fields preserved.
#[tokio::test]
async fn credential_stored_is_readable() {
    let store = Arc::new(InMemoryStore::new());

    // (1) Append CredentialStored.
    store
        .append(&[store_event(
            "1",
            "a",
            "openai",
            Some("key_v1"),
            Some("1.0"),
            1_000,
        )])
        .await
        .unwrap();

    // (2) Verify record.
    let rec = CredentialReadModel::get(store.as_ref(), &cred_id("1"))
        .await
        .unwrap()
        .expect("credential must exist after CredentialStored");

    assert_eq!(rec.id, cred_id("1"));
    assert_eq!(rec.tenant_id, tenant("a"));
    assert_eq!(rec.provider_id, "openai");
    assert!(rec.active, "new credential must be active");
    assert!(
        rec.revoked_at_ms.is_none(),
        "new credential must not be revoked"
    );
    assert_eq!(rec.key_id.as_deref(), Some("key_v1"));
    assert_eq!(rec.key_version.as_deref(), Some("1.0"));
    assert_eq!(rec.encrypted_at_ms, Some(1_000));
    // Encrypted value is stored.
    assert!(
        !rec.encrypted_value.is_empty(),
        "encrypted_value must be stored"
    );
}

/// (3) + (4): CredentialKeyRotated appends a rotation audit record.
#[tokio::test]
async fn credential_key_rotation_creates_rotation_record() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            store_event("r1", "b", "anthropic", Some("key_old"), Some("1.0"), 1_000),
            store_event("r2", "b", "anthropic", Some("key_old"), Some("1.0"), 2_000),
        ])
        .await
        .unwrap();

    // (3) Append CredentialKeyRotated — rotates both credentials.
    store
        .append(&[rotate_event(
            "rot_001",
            "b",
            "key_old",
            "key_new_v2",
            &["cred_r1", "cred_r2"],
        )])
        .await
        .unwrap();

    // (4) Verify rotation record.
    let rotations = list_rotations(store.as_ref(), &tenant("b")).await;

    assert_eq!(rotations.len(), 1, "one rotation record must exist");
    let rot = &rotations[0];
    assert_eq!(rot.rotation_id, "rot_001");
    assert_eq!(rot.tenant_id, tenant("b"));
    assert_eq!(rot.old_key_id, "key_old");
    assert_eq!(rot.new_key_id, "key_new_v2");
    assert_eq!(rot.rotated_credentials, 2, "2 credentials were rotated");
}

/// Multiple rotations accumulate in the audit log.
#[tokio::test]
async fn multiple_rotations_accumulate_in_audit_log() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            store_event("m1", "c", "openai", Some("key_v1"), None, 1_000),
            rotate_event("rot_1", "c", "key_v1", "key_v2", &["cred_m1"]),
            rotate_event("rot_2", "c", "key_v2", "key_v3", &["cred_m1"]),
            rotate_event("rot_3", "c", "key_v3", "key_v4", &["cred_m1"]),
        ])
        .await
        .unwrap();

    let rotations = list_rotations(store.as_ref(), &tenant("c")).await;

    assert_eq!(rotations.len(), 3, "all 3 rotation records must accumulate");

    // Rotation IDs are distinct.
    let rot_ids: Vec<&str> = rotations.iter().map(|r| r.rotation_id.as_str()).collect();
    assert!(rot_ids.contains(&"rot_1"));
    assert!(rot_ids.contains(&"rot_2"));
    assert!(rot_ids.contains(&"rot_3"));
}

/// (5) + (6): CredentialRevoked marks the record inactive with revoked_at_ms.
#[tokio::test]
async fn credential_revoked_marks_inactive() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[store_event(
            "rev1",
            "d",
            "bedrock",
            Some("key_v1"),
            None,
            1_000,
        )])
        .await
        .unwrap();

    // Verify active before revocation.
    let before = CredentialReadModel::get(store.as_ref(), &cred_id("rev1"))
        .await
        .unwrap()
        .unwrap();
    assert!(before.active, "credential must be active before revocation");
    assert!(before.revoked_at_ms.is_none());

    // (5) Append CredentialRevoked.
    store
        .append(&[revoke_event("rev1", "d", 9_000)])
        .await
        .unwrap();

    // (6) Verify inactive state.
    let after = CredentialReadModel::get(store.as_ref(), &cred_id("rev1"))
        .await
        .unwrap()
        .unwrap();

    assert!(
        !after.active,
        "credential must be inactive after CredentialRevoked"
    );
    assert_eq!(
        after.revoked_at_ms,
        Some(9_000),
        "revoked_at_ms must be set to the event timestamp"
    );
    assert_eq!(
        after.updated_at, 9_000,
        "updated_at must reflect revocation time"
    );

    // Revoked credential is still readable (audit trail preserved).
    let found = CredentialReadModel::get(store.as_ref(), &cred_id("rev1"))
        .await
        .unwrap();
    assert!(
        found.is_some(),
        "revoked credential must remain readable (not deleted)"
    );
}

/// (7): Cross-tenant isolation — each tenant sees only its own credentials.
#[tokio::test]
async fn cross_tenant_credential_isolation() {
    let store = Arc::new(InMemoryStore::new());

    // Tenant X: 2 credentials.
    store
        .append(&[
            store_event("x1", "x", "openai", None, None, 1_000),
            store_event("x2", "x", "anthropic", None, None, 2_000),
        ])
        .await
        .unwrap();

    // Tenant Y: 1 credential.
    store
        .append(&[store_event("y1", "y", "bedrock", None, None, 3_000)])
        .await
        .unwrap();

    // Tenant X sees its own 2 credentials.
    let x_creds = list_credentials(store.as_ref(), &tenant("x"), 100, 0).await;
    assert_eq!(x_creds.len(), 2, "tenant X must have 2 credentials");
    assert!(
        x_creds.iter().all(|c| c.tenant_id == tenant("x")),
        "all X credentials must be scoped to tenant X"
    );

    // Tenant Y sees its own 1 credential.
    let y_creds = list_credentials(store.as_ref(), &tenant("y"), 100, 0).await;
    assert_eq!(y_creds.len(), 1, "tenant Y must have 1 credential");
    assert_eq!(y_creds[0].id, cred_id("y1"));

    // get by ID for a different tenant returns correct record (keyed by cred_id globally).
    let x_cred = CredentialReadModel::get(store.as_ref(), &cred_id("x1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(x_cred.tenant_id, tenant("x"), "cred_x1 belongs to tenant X");

    // Y's credential_id is not visible to X's listing.
    assert!(
        !x_creds.iter().any(|c| c.id == cred_id("y1")),
        "tenant X listing must not include tenant Y's credentials"
    );

    // Rotations are also tenant-scoped.
    store
        .append(&[
            rotate_event(
                "rot_x",
                "x",
                "key_x_old",
                "key_x_new",
                &["cred_x1", "cred_x2"],
            ),
            rotate_event("rot_y", "y", "key_y_old", "key_y_new", &["cred_y1"]),
        ])
        .await
        .unwrap();

    let x_rots = list_rotations(store.as_ref(), &tenant("x")).await;
    let y_rots = list_rotations(store.as_ref(), &tenant("y")).await;

    assert_eq!(x_rots.len(), 1, "tenant X must have 1 rotation record");
    assert_eq!(y_rots.len(), 1, "tenant Y must have 1 rotation record");
    assert_eq!(x_rots[0].rotation_id, "rot_x");
    assert_eq!(y_rots[0].rotation_id, "rot_y");
    assert!(
        !x_rots.iter().any(|r| r.rotation_id == "rot_y"),
        "tenant X must not see tenant Y's rotation"
    );
}

/// Full lifecycle: store → rotate → revoke in sequence, event log tracks all.
#[tokio::test]
async fn full_credential_lifecycle_in_event_log() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            store_event("lc1", "e", "openai", Some("k1"), Some("v1"), 1_000),
            rotate_event("rot_lc", "e", "k1", "k2", &["cred_lc1"]),
            revoke_event("lc1", "e", 5_000),
        ])
        .await
        .unwrap();

    // Event log contains all three events.
    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 3);

    let has_stored = events.iter().any(|e| matches!(&e.envelope.payload, RuntimeEvent::CredentialStored(s) if s.credential_id == cred_id("lc1")));
    let has_rotated = events.iter().any(|e| matches!(&e.envelope.payload, RuntimeEvent::CredentialKeyRotated(r) if r.rotation_id == "rot_lc"));
    let has_revoked = events.iter().any(|e| matches!(&e.envelope.payload, RuntimeEvent::CredentialRevoked(r) if r.credential_id == cred_id("lc1")));

    assert!(has_stored, "CredentialStored must be in log");
    assert!(has_rotated, "CredentialKeyRotated must be in log");
    assert!(has_revoked, "CredentialRevoked must be in log");

    // Final state: credential inactive, rotation recorded.
    let cred = CredentialReadModel::get(store.as_ref(), &cred_id("lc1"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !cred.active,
        "credential must be inactive after full lifecycle"
    );
    assert_eq!(cred.revoked_at_ms, Some(5_000));

    let rots = list_rotations(store.as_ref(), &tenant("e")).await;
    assert_eq!(rots.len(), 1);
    assert_eq!(rots[0].old_key_id, "k1");
    assert_eq!(rots[0].new_key_id, "k2");
}
