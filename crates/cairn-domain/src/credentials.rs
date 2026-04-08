use crate::ids::{CredentialId, TenantId};
use serde::{Deserialize, Serialize};

/// Tenant-scoped credential record for provider and channel access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRecord {
    pub id: CredentialId,
    pub tenant_id: TenantId,
    pub name: String,
    pub credential_type: String,
    pub encrypted_value: Vec<u8>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub encrypted_at_ms: Option<u64>,
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub key_version: Option<String>,
    #[serde(default)]
    pub revoked_at_ms: Option<u64>,
}

fn default_true() -> bool {
    true
}

/// Audit record for a credential rotation event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialRotationRecord {
    pub rotation_id: String,
    pub tenant_id: TenantId,
    #[serde(default)]
    pub credential_id: CredentialId,
    #[serde(default)]
    pub rotated_at: u64,
    #[serde(default)]
    pub rotated_by: Option<String>,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub completed_at_ms: Option<u64>,
    #[serde(default)]
    pub old_key_id: String,
    #[serde(default)]
    pub new_key_id: String,
    /// Count of credentials rotated in this operation.
    #[serde(default)]
    pub rotated_credentials: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_record_carries_tenant_scope() {
        let record = CredentialRecord {
            id: CredentialId::new("cred_1"),
            tenant_id: TenantId::new("tenant_acme"),
            name: "openai-api-key".to_owned(),
            credential_type: "api_key".to_owned(),
            encrypted_value: vec![1, 2, 3],
            created_at: 100,
            updated_at: 100,
            active: true,
            provider_id: String::new(),
            encrypted_at_ms: None,
            key_id: None,
            key_version: None,
            revoked_at_ms: None,
        };
        assert_eq!(record.tenant_id.as_str(), "tenant_acme");
    }
}
