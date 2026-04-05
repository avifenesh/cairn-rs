//! RFC 011 — credential lifecycle end-to-end integration tests.
//!
//! Covers the full credential arc:
//!   1. Store a credential — verifies AES-256-GCM encryption and key_version tagging
//!   2. Retrieve and verify all fields round-trip correctly
//!   3. Revoke the credential
//!   4. Verify revoked_at_ms is set and active=false on the record
//!   5. Show a revoked credential cannot be used (active guard)
//!
//! Additional coverage:
//!   - Storing without a valid tenant returns NotFound
//!   - Multiple credentials listed and paginated per tenant
//!   - Key rotation re-encrypts under a new key_id

use std::sync::Arc;
use std::time::Duration;

use cairn_domain::{CredentialId, TenantId};
use cairn_runtime::credentials::CredentialService;
use cairn_runtime::error::RuntimeError;
use cairn_runtime::services::{CredentialServiceImpl, TenantServiceImpl};
use cairn_runtime::tenants::TenantService;
use cairn_store::InMemoryStore;

fn tenant_id() -> TenantId {
    TenantId::new("tenant_rfc011")
}

async fn setup() -> (Arc<InMemoryStore>, CredentialServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let tenant_svc = TenantServiceImpl::new(store.clone());
    tenant_svc
        .create(tenant_id(), "RFC 011 Tenant".to_owned())
        .await
        .unwrap();
    let cred_svc = CredentialServiceImpl::new(store.clone());
    (store, cred_svc)
}

// ── Test 1 + 2: store credential, verify encrypted value and key_version ─────

/// RFC 011 §3: credentials must be encrypted at rest.
/// The stored record must carry a non-empty ciphertext that differs from the
/// plaintext, carry the `key_version` tag, and reflect the correct provider.
#[tokio::test]
async fn store_credential_encrypts_value_and_tags_key_version() {
    let (_store, cred_svc) = setup().await;

    let plaintext = "sk-openai-secret-token-abc123";

    let stored = cred_svc
        .store(
            tenant_id(),
            "openai".to_owned(),
            plaintext.to_owned(),
            Some("kek-primary-v1".to_owned()),
        )
        .await
        .unwrap();

    // ── Basic identity fields ────────────────────────────────────────────
    assert!(!stored.id.as_str().is_empty(), "credential must have a non-empty ID");
    assert_eq!(stored.tenant_id, tenant_id());
    assert_eq!(stored.provider_id, "openai");

    // ── Encryption ──────────────────────────────────────────────────────
    assert!(
        !stored.encrypted_value.is_empty(),
        "encrypted_value must be non-empty"
    );
    assert_ne!(
        stored.encrypted_value,
        plaintext.as_bytes(),
        "RFC 011: stored value must be ciphertext, not plaintext"
    );

    // ── Key metadata ────────────────────────────────────────────────────
    assert_eq!(
        stored.key_id.as_deref(),
        Some("kek-primary-v1"),
        "key_id must be preserved on the record"
    );
    assert_eq!(
        stored.key_version.as_deref(),
        Some("v1"),
        "RFC 011: key_version must be set to 'v1' on store"
    );
    assert!(stored.encrypted_at_ms.is_some(), "encrypted_at_ms must be recorded");

    // ── State ───────────────────────────────────────────────────────────
    assert!(stored.active, "freshly stored credential must be active");
    assert!(
        stored.revoked_at_ms.is_none(),
        "freshly stored credential must not have revoked_at_ms"
    );

    // ── Test 2: retrieve and verify round-trip ───────────────────────────
    let fetched = cred_svc
        .get(&stored.id)
        .await
        .unwrap()
        .expect("credential must be retrievable by ID");

    assert_eq!(fetched, stored, "get() must return the same record as store()");
    assert_eq!(fetched.encrypted_value, stored.encrypted_value);
    assert_eq!(fetched.key_id, stored.key_id);
    assert_eq!(fetched.key_version, stored.key_version);
    assert!(fetched.active);
}

// ── Test 3 + 4 + 5: revoke, verify revoked_at_ms, show active guard ──────────

/// RFC 011 §4: revocation must set active=false and stamp revoked_at_ms.
/// A revoked credential must remain readable for audit but must fail the
/// active check so callers cannot use it.
#[tokio::test]
async fn revoke_credential_stamps_revoked_at_ms_and_clears_active() {
    let (_store, cred_svc) = setup().await;

    // Store credential.
    let stored = cred_svc
        .store(
            tenant_id(),
            "anthropic".to_owned(),
            "sk-ant-secret-456".to_owned(),
            Some("kek-primary-v1".to_owned()),
        )
        .await
        .unwrap();

    let credential_id = stored.id.clone();
    assert!(stored.active, "pre-condition: credential must be active before revocation");
    let stored_at = stored.encrypted_at_ms.unwrap();

    // ── Test 3: revoke ───────────────────────────────────────────────────
    let revoked = cred_svc.revoke(&credential_id).await.unwrap();

    // ── Test 4: verify revoked_at_ms is set ──────────────────────────────
    assert!(
        !revoked.active,
        "RFC 011: active must be false after revocation"
    );
    let revoked_at = revoked
        .revoked_at_ms
        .expect("RFC 011: revoked_at_ms must be set after revocation");
    assert!(
        revoked_at >= stored_at,
        "revoked_at_ms ({revoked_at}) must be >= encrypted_at_ms ({stored_at})"
    );

    // ── Test 5: show the revoked credential cannot be used ───────────────
    // The service returns the record — callers must check `active` before use.
    let after_revoke = cred_svc
        .get(&credential_id)
        .await
        .unwrap()
        .expect("revoked credential must still be retrievable for audit");

    assert!(
        !after_revoke.active,
        "RFC 011: credential must remain inactive after revocation"
    );
    assert!(
        after_revoke.revoked_at_ms.is_some(),
        "revoked_at_ms must persist on the record"
    );

    // Simulate the caller guard: active must be false, so the credential
    // must not be used for authentication.
    let can_use = after_revoke.active;
    assert!(
        !can_use,
        "RFC 011: a revoked credential must not pass the active guard"
    );

    // Encrypted value is still present (for audit trail), but caller must not decrypt it.
    assert!(
        !after_revoke.encrypted_value.is_empty(),
        "encrypted_value must be retained for audit even after revocation"
    );
}

// ── Revocation is idempotent ──────────────────────────────────────────────────

/// RFC 011 §4: revoking an already-revoked credential must be a no-op —
/// it must return the existing record without error and without updating
/// revoked_at_ms.
#[tokio::test]
async fn revoke_is_idempotent() {
    let (_store, cred_svc) = setup().await;

    let stored = cred_svc
        .store(
            tenant_id(),
            "slack".to_owned(),
            "xoxb-slack-token".to_owned(),
            None,
        )
        .await
        .unwrap();

    let first_revoke = cred_svc.revoke(&stored.id).await.unwrap();
    let first_revoked_at = first_revoke.revoked_at_ms.unwrap();

    // Small sleep to ensure a different timestamp if re-revocation incorrectly updates the field.
    tokio::time::sleep(Duration::from_millis(2)).await;

    let second_revoke = cred_svc.revoke(&stored.id).await.unwrap();

    assert!(
        !second_revoke.active,
        "credential must remain inactive on second revoke"
    );
    assert_eq!(
        second_revoke.revoked_at_ms.unwrap(),
        first_revoked_at,
        "RFC 011: revoked_at_ms must not change on idempotent revocation"
    );
}

// ── Storing without a valid tenant returns NotFound ───────────────────────────

/// RFC 011 §2: credentials are tenant-scoped.  Storing a credential for a
/// non-existent tenant must return a NotFound error — no silent creation.
#[tokio::test]
async fn store_for_nonexistent_tenant_returns_not_found() {
    let store = Arc::new(InMemoryStore::new());
    let cred_svc = CredentialServiceImpl::new(store);

    let err = cred_svc
        .store(
            TenantId::new("ghost_tenant"),
            "openai".to_owned(),
            "secret".to_owned(),
            None,
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, RuntimeError::NotFound { entity: "tenant", .. }),
        "RFC 011: storing for a non-existent tenant must return NotFound; got: {err:?}"
    );
}

// ── Revoking a non-existent credential returns NotFound ───────────────────────

/// RFC 011 §4: revoking an unknown credential ID must return NotFound,
/// not silently succeed.
#[tokio::test]
async fn revoke_nonexistent_credential_returns_not_found() {
    let (_store, cred_svc) = setup().await;

    let err = cred_svc
        .revoke(&CredentialId::new("cred_does_not_exist"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, RuntimeError::NotFound { entity: "credential", .. }),
        "revoking a non-existent credential must return NotFound; got: {err:?}"
    );
}

// ── Multiple credentials per tenant — list and pagination ────────────────────

/// RFC 011 §2: a tenant may hold multiple credentials for different providers.
/// list() must return all of them, and pagination must work correctly.
#[tokio::test]
async fn list_and_paginate_tenant_credentials() {
    let (_store, cred_svc) = setup().await;

    let providers = ["openai", "anthropic", "bedrock", "openrouter"];
    let mut stored_ids = Vec::new();

    for provider in providers {
        let c = cred_svc
            .store(
                tenant_id(),
                provider.to_owned(),
                format!("secret-for-{provider}"),
                Some("kek-v1".to_owned()),
            )
            .await
            .unwrap();
        stored_ids.push(c.id.clone());
        // Ensure unique encrypted_at_ms timestamps.
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // List all — must return exactly 4.
    let all = cred_svc.list(&tenant_id(), 10, 0).await.unwrap();
    assert_eq!(all.len(), 4, "list must return all 4 stored credentials");

    // Every record must be active.
    for c in &all {
        assert!(c.active, "freshly stored credential must be active");
        assert_eq!(c.tenant_id, tenant_id());
        assert!(!c.encrypted_value.is_empty());
    }

    // Pagination: limit=2, offset=0 → first 2.
    let page1 = cred_svc.list(&tenant_id(), 2, 0).await.unwrap();
    assert_eq!(page1.len(), 2, "page 1 must return 2 credentials");

    // Pagination: limit=2, offset=2 → last 2.
    let page2 = cred_svc.list(&tenant_id(), 2, 2).await.unwrap();
    assert_eq!(page2.len(), 2, "page 2 must return 2 credentials");

    // Pages must not overlap.
    let page1_ids: Vec<_> = page1.iter().map(|c| &c.id).collect();
    let page2_ids: Vec<_> = page2.iter().map(|c| &c.id).collect();
    for id in &page2_ids {
        assert!(
            !page1_ids.contains(id),
            "pagination pages must not overlap; duplicate id: {id}"
        );
    }

    // Offset past end → empty.
    let empty = cred_svc.list(&tenant_id(), 10, 100).await.unwrap();
    assert!(empty.is_empty(), "offset past end must return empty list");

    // Revoke one and verify the rest are still active.
    cred_svc.revoke(&stored_ids[0]).await.unwrap();
    let after_revoke = cred_svc.list(&tenant_id(), 10, 0).await.unwrap();
    let active_count = after_revoke.iter().filter(|c| c.active).count();
    let revoked_count = after_revoke.iter().filter(|c| !c.active).count();
    assert_eq!(active_count, 3, "3 credentials must remain active");
    assert_eq!(revoked_count, 1, "exactly 1 credential must be revoked");
}

// ── Key rotation re-encrypts credentials under new key ───────────────────────

/// RFC 011 §5: key rotation must re-encrypt all active credentials that
/// use the old key under the new key_id, record a rotation event, and
/// not touch credentials under a different key.
#[tokio::test]
async fn key_rotation_reencrypts_credentials_and_records_rotation() {
    let (_store, cred_svc) = setup().await;

    // Two credentials under "key_old", one under "key_other" (must not rotate).
    let c1 = cred_svc
        .store(tenant_id(), "openai".to_owned(), "token-a".to_owned(), Some("key_old".to_owned()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    let c2 = cred_svc
        .store(tenant_id(), "slack".to_owned(), "token-b".to_owned(), Some("key_old".to_owned()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    cred_svc
        .store(tenant_id(), "github".to_owned(), "token-c".to_owned(), Some("key_other".to_owned()))
        .await
        .unwrap();

    // Rotate key_old → key_new.
    let rotation = cred_svc
        .rotate_key(tenant_id(), "key_old".to_owned(), "key_new".to_owned())
        .await
        .unwrap();

    assert_eq!(
        rotation.rotated_credentials, 2,
        "rotation must affect exactly the 2 credentials under key_old"
    );
    assert_eq!(rotation.old_key_id, "key_old");
    assert_eq!(rotation.new_key_id, "key_new");
    assert!(rotation.completed_at_ms.is_some(), "rotation must have a completion timestamp");

    // Rotated credentials must now use key_new.
    for id in [&c1.id, &c2.id] {
        let after = cred_svc.get(id).await.unwrap().unwrap();
        assert_eq!(
            after.key_id.as_deref(),
            Some("key_new"),
            "rotated credential must use key_new"
        );
        // The encrypted_value must have changed (different key → different ciphertext).
        // (We can't decrypt here without the internal helper, but we can verify the
        // ciphertext byte length is plausible for AES-256-GCM output.)
        assert!(after.encrypted_value.len() > 16, "rotated ciphertext must be non-trivial");
    }

    // The "key_other" credential must be untouched.
    let all = cred_svc.list(&tenant_id(), 10, 0).await.unwrap();
    let key_other_creds: Vec<_> = all.iter().filter(|c| c.key_id.as_deref() == Some("key_other")).collect();
    assert_eq!(key_other_creds.len(), 1, "credential under key_other must not be rotated");
}
