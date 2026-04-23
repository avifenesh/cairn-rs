//! Concrete credential service implementation.

use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use async_trait::async_trait;
use cairn_domain::credentials::{CredentialRecord, CredentialRotationRecord};
use cairn_domain::*;
use cairn_store::projections::{CredentialReadModel, TenantReadModel};
use cairn_store::EventLog;
use sha2::{Digest, Sha256};

use super::event_helpers::make_envelope;
use crate::credentials::CredentialService;
use crate::error::RuntimeError;

pub struct CredentialServiceImpl<S> {
    store: Arc<S>,
}

impl<S> CredentialServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn derive_key_material(key_id: Option<&str>) -> [u8; 32] {
    let seed = key_id.unwrap_or("cairn-local-test-key");
    let digest = Sha256::digest(seed.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest[..32]);
    key
}

fn derive_nonce(tenant_id: &TenantId, provider_id: &str, encrypted_at_ms: u64) -> [u8; 12] {
    let digest = Sha256::digest(
        format!("{}:{provider_id}:{encrypted_at_ms}", tenant_id.as_str()).as_bytes(),
    );
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&digest[..12]);
    nonce
}

fn encrypt_value(
    tenant_id: &TenantId,
    provider_id: &str,
    plaintext_value: &str,
    key_id: Option<&str>,
    encrypted_at_ms: u64,
) -> Result<Vec<u8>, RuntimeError> {
    let key_material = derive_key_material(key_id);
    let key = Key::<Aes256Gcm>::from_slice(&key_material);
    let cipher = Aes256Gcm::new(key);
    let nonce_bytes = derive_nonce(tenant_id, provider_id, encrypted_at_ms);
    let nonce = Nonce::from_slice(&nonce_bytes);
    cipher
        .encrypt(nonce, plaintext_value.as_bytes())
        .map_err(|e| RuntimeError::Internal(format!("credential encryption failed: {e}")))
}

fn decrypt_value(
    tenant_id: &TenantId,
    provider_id: &str,
    encrypted_value: &[u8],
    key_id: Option<&str>,
    encrypted_at_ms: u64,
) -> Result<String, RuntimeError> {
    let key_material = derive_key_material(key_id);
    let key = Key::<Aes256Gcm>::from_slice(&key_material);
    let cipher = Aes256Gcm::new(key);
    let nonce_bytes = derive_nonce(tenant_id, provider_id, encrypted_at_ms);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, encrypted_value)
        .map_err(|e| RuntimeError::Internal(format!("credential decryption failed: {e}")))?;
    String::from_utf8(plaintext)
        .map_err(|e| RuntimeError::Internal(format!("credential plaintext invalid utf-8: {e}")))
}

pub(crate) fn decrypt_credential_record(record: &CredentialRecord) -> Result<String, RuntimeError> {
    let encrypted_at_ms = record.encrypted_at_ms.ok_or_else(|| {
        RuntimeError::Internal(format!("credential {} missing encrypted_at_ms", record.id))
    })?;
    decrypt_value(
        &record.tenant_id,
        &record.provider_id,
        &record.encrypted_value,
        record.key_id.as_deref(),
        encrypted_at_ms,
    )
}

#[async_trait]
impl<S> CredentialService for CredentialServiceImpl<S>
where
    S: EventLog + CredentialReadModel + TenantReadModel + Send + Sync + 'static,
{
    async fn store(
        &self,
        tenant_id: TenantId,
        provider_id: String,
        plaintext_value: String,
        key_id: Option<String>,
    ) -> Result<CredentialRecord, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        // Closes #217: reject duplicate `(tenant_id, provider_id)` so two
        // back-to-back POSTs with the same provider don't silently
        // accumulate active credentials (the projection keyed on
        // credential_id would happily return two rows). A revoked
        // credential with the same provider_id is allowed — operators
        // rotate by revoke-then-create, and blocking that would be a
        // silent regression. Callers who genuinely need to replace an
        // active credential must revoke it first.
        //
        // Concurrency caveat (acknowledged): this is a read-then-write
        // sequence, so two simultaneous `store` calls on the same
        // `(tenant_id, provider_id)` can both pass the pre-check and
        // both append. We close most of that window below by
        // re-reading the projection AFTER append and, if a duplicate
        // slipped in, emitting a `CredentialRevoked` event for our just-
        // written record and returning 409 to the caller. This keeps
        // the happy path O(1) write and hardens the race without
        // requiring a per-backend unique index (portable-DB rule in
        // CLAUDE.md — Postgres v1 target but must work on SQLite/
        // InMemory too). A dedicated projection-level unique index is
        // the correct long-term home for this; tracked as follow-up.
        //
        // `list_by_tenant` is the cheapest portable check: the read
        // model is already built per tenant, and tenants typically carry
        // O(10) credentials. Switching to a dedicated projection lookup
        // is a measurable optimization only if a tenant grows past a
        // few hundred active credentials.
        let existing =
            CredentialReadModel::list_by_tenant(self.store.as_ref(), &tenant_id, usize::MAX, 0)
                .await?;
        if existing
            .iter()
            .any(|c| c.active && c.provider_id == provider_id)
        {
            return Err(RuntimeError::Conflict {
                entity: "credential",
                id: format!("provider={provider_id} tenant={tenant_id}"),
            });
        }

        let encrypted_at_ms = now_ms();
        let encrypted_value = encrypt_value(
            &tenant_id,
            &provider_id,
            &plaintext_value,
            key_id.as_deref(),
            encrypted_at_ms,
        )?;
        let credential_id = CredentialId::new(format!("cred_{encrypted_at_ms}"));
        let event = make_envelope(RuntimeEvent::CredentialStored(CredentialStored {
            tenant_id: tenant_id.clone(),
            credential_id: credential_id.clone(),
            provider_id: provider_id.clone(),
            encrypted_value,
            key_id,
            key_version: Some("v1".to_owned()),
            encrypted_at_ms,
        }));
        self.store.append(&[event]).await?;

        // Post-append race resolution: re-read and check whether another
        // concurrent `store` for the same (tenant_id, provider_id) also
        // slipped past the pre-check. The event log is append-ordered so
        // at this point both writers have landed and can see each other.
        // Whichever writer's credential_id sorts later revokes itself and
        // returns 409; the other keeps its record. Deterministic tie-break
        // (`credential_id` ordering) means both writers agree on the
        // winner without a second round-trip or cross-writer coordination.
        let post =
            CredentialReadModel::list_by_tenant(self.store.as_ref(), &tenant_id, usize::MAX, 0)
                .await?;
        let actives: Vec<&CredentialRecord> = post
            .iter()
            .filter(|c| c.active && c.provider_id == provider_id)
            .collect();
        if actives.len() > 1 {
            let our_is_loser = actives
                .iter()
                .map(|c| c.id.as_str())
                .any(|id| id > credential_id.as_str());
            if our_is_loser {
                let revoke_event =
                    make_envelope(RuntimeEvent::CredentialRevoked(CredentialRevoked {
                        tenant_id: tenant_id.clone(),
                        credential_id: credential_id.clone(),
                        revoked_at_ms: now_ms(),
                    }));
                self.store.append(&[revoke_event]).await?;
                return Err(RuntimeError::Conflict {
                    entity: "credential",
                    id: format!("provider={provider_id} tenant={tenant_id}"),
                });
            }
        }

        CredentialReadModel::get(self.store.as_ref(), &credential_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("credential not found after store".to_owned()))
    }

    async fn get(&self, id: &CredentialId) -> Result<Option<CredentialRecord>, RuntimeError> {
        Ok(CredentialReadModel::get(self.store.as_ref(), id).await?)
    }

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<CredentialRecord>, RuntimeError> {
        Ok(
            CredentialReadModel::list_by_tenant(self.store.as_ref(), tenant_id, limit, offset)
                .await?,
        )
    }

    async fn revoke(&self, id: &CredentialId) -> Result<CredentialRecord, RuntimeError> {
        let existing = CredentialReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "credential",
                id: id.to_string(),
            })?;

        if !existing.active {
            return Ok(existing);
        }

        let event = make_envelope(RuntimeEvent::CredentialRevoked(CredentialRevoked {
            tenant_id: existing.tenant_id.clone(),
            credential_id: existing.id.clone(),
            revoked_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;

        CredentialReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("credential not found after revoke".to_owned()))
    }

    async fn rotate_key(
        &self,
        tenant_id: TenantId,
        old_key_id: String,
        new_key_id: String,
    ) -> Result<CredentialRotationRecord, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        let started_at_ms = now_ms();
        let credentials =
            CredentialReadModel::list_by_tenant(self.store.as_ref(), &tenant_id, usize::MAX, 0)
                .await?;
        let candidates: Vec<_> = credentials
            .into_iter()
            .filter(|credential| credential.active)
            .filter(|credential| credential.key_id.as_deref() == Some(old_key_id.as_str()))
            .collect();

        let mut events = Vec::with_capacity(candidates.len() + 1);
        let mut rotated_ids = Vec::with_capacity(candidates.len());

        for (idx, credential) in candidates.iter().enumerate() {
            let plaintext = decrypt_credential_record(credential)?;
            let rotated_at_ms = started_at_ms.saturating_add(idx as u64);
            let encrypted_value = encrypt_value(
                &credential.tenant_id,
                &credential.provider_id,
                &plaintext,
                Some(new_key_id.as_str()),
                rotated_at_ms,
            )?;
            rotated_ids.push(credential.id.to_string());
            events.push(make_envelope(RuntimeEvent::CredentialStored(
                CredentialStored {
                    tenant_id: credential.tenant_id.clone(),
                    credential_id: credential.id.clone(),
                    provider_id: credential.provider_id.clone(),
                    encrypted_value,
                    key_id: Some(new_key_id.clone()),
                    key_version: credential
                        .key_version
                        .clone()
                        .or_else(|| Some("v1".to_owned())),
                    encrypted_at_ms: rotated_at_ms,
                },
            )));
        }

        let rotation_id = format!("credrot_{started_at_ms}");
        events.push(make_envelope(RuntimeEvent::CredentialKeyRotated(
            CredentialKeyRotated {
                tenant_id: tenant_id.clone(),
                rotation_id: rotation_id.clone(),
                old_key_id: old_key_id.clone(),
                new_key_id: new_key_id.clone(),
                credential_ids_rotated: rotated_ids.clone(),
            },
        )));

        self.store.append(&events).await?;

        Ok(CredentialRotationRecord {
            rotation_id,
            tenant_id,
            credential_id: CredentialId::new(""),
            rotated_at: now_ms(),
            rotated_by: None,
            old_key_id,
            new_key_id,
            rotated_credentials: rotated_ids.len() as u32,
            started_at_ms,
            completed_at_ms: Some(now_ms()),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::decrypt_value;
    use cairn_domain::TenantId;
    use cairn_store::projections::CredentialRotationReadModel;
    use cairn_store::InMemoryStore;

    use crate::credentials::CredentialService;
    use crate::services::{CredentialServiceImpl, TenantServiceImpl};
    use crate::tenants::TenantService;

    #[tokio::test]
    async fn credential_store_get_revoke_round_trip() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        tenant_service
            .create(TenantId::new("tenant_acme"), "Acme".to_owned())
            .await
            .unwrap();

        let service = CredentialServiceImpl::new(store);
        let plaintext = "super-secret-token";
        let stored = service
            .store(
                TenantId::new("tenant_acme"),
                "openai".to_owned(),
                plaintext.to_owned(),
                Some("kek-primary".to_owned()),
            )
            .await
            .unwrap();

        let fetched = service.get(&stored.id).await.unwrap().unwrap();
        assert_eq!(stored, fetched);
        assert_ne!(fetched.encrypted_value, plaintext.as_bytes());
        assert!(fetched.active);

        let revoked = service.revoke(&stored.id).await.unwrap();
        assert!(!revoked.active);
        assert!(revoked.revoked_at_ms.is_some());
    }

    #[tokio::test]
    async fn key_rotation_reencrypts_all_tenant_credentials() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        tenant_service
            .create(TenantId::new("tenant_acme"), "Acme".to_owned())
            .await
            .unwrap();

        let service = CredentialServiceImpl::new(store.clone());
        let inputs = [
            ("openai", "token-a"),
            ("anthropic", "token-b"),
            ("slack", "token-c"),
        ];

        let mut expected_plaintexts = std::collections::HashMap::new();
        for (provider_id, plaintext) in inputs {
            let stored = service
                .store(
                    TenantId::new("tenant_acme"),
                    provider_id.to_owned(),
                    plaintext.to_owned(),
                    Some("key_a".to_owned()),
                )
                .await
                .unwrap();
            expected_plaintexts.insert(stored.id.to_string(), plaintext.to_owned());
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        let rotation = service
            .rotate_key(
                TenantId::new("tenant_acme"),
                "key_a".to_owned(),
                "key_b".to_owned(),
            )
            .await
            .unwrap();

        assert_eq!(rotation.rotated_credentials, 3);
        assert!(rotation.completed_at_ms.is_some());

        let credentials = service
            .list(&TenantId::new("tenant_acme"), 10, 0)
            .await
            .unwrap();
        assert_eq!(credentials.len(), 3);
        for credential in credentials {
            assert_eq!(credential.key_id.as_deref(), Some("key_b"));
            let encrypted_at_ms = credential.encrypted_at_ms.unwrap();
            let decrypted = decrypt_value(
                &credential.tenant_id,
                &credential.provider_id,
                &credential.encrypted_value,
                Some("key_b"),
                encrypted_at_ms,
            )
            .unwrap();
            assert_eq!(
                decrypted,
                expected_plaintexts
                    .get(&credential.id.to_string())
                    .unwrap()
                    .as_str()
            );
            assert!(decrypt_value(
                &credential.tenant_id,
                &credential.provider_id,
                &credential.encrypted_value,
                Some("key_a"),
                encrypted_at_ms,
            )
            .is_err());
        }

        let rotations = CredentialRotationReadModel::list_rotations(
            store.as_ref(),
            &TenantId::new("tenant_acme"),
        )
        .await
        .unwrap();
        assert_eq!(rotations.len(), 1);
        assert_eq!(rotations[0].rotated_credentials, 3);
        assert_eq!(rotations[0].old_key_id, "key_a");
        assert_eq!(rotations[0].new_key_id, "key_b");
    }
}
