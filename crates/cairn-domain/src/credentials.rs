use crate::ids::{CredentialId, TenantId};
use serde::{Deserialize, Serialize};

/// Tenant-scoped credential record for provider and channel access.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialRecord {
    pub id: CredentialId,
    pub tenant_id: TenantId,
    pub name: String,
    pub credential_type: String,
    pub encrypted_value: Vec<u8>,
    pub created_at: u64,
    pub updated_at: u64,
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
        };
        assert_eq!(record.tenant_id.as_str(), "tenant_acme");
    }
}
