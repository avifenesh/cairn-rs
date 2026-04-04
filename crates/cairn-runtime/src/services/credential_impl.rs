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
            tenant_id,
            credential_id: credential_id.clone(),
            provider_id,
            encrypted_value,
            key_id,
            key_version: Some("v1".to_owned()),
            encrypted_at_ms,
        }));
        self.store.append(&[event]).await?;

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
            let encrypted_at_ms = credential.encrypted_at_ms.ok_or_else(|| {
                RuntimeError::Internal(format!(
                    "credential {} missing encrypted_at_ms for rotation",
                    credential.id
                ))
            })?;
            let plaintext = decrypt_value(
                &credential.tenant_id,
                &credential.provider_id,
                &credential.encrypted_value,
                credential.key_id.as_deref(),
                encrypted_at_ms,
            )?;
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
