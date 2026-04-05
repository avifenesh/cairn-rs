use cairn_domain::ids::OperatorId;
use cairn_domain::tenancy::{ProjectKey, TenantKey};
use serde::{Deserialize, Serialize};

/// Authenticated principal resolved from a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthPrincipal {
    Operator {
        operator_id: OperatorId,
        tenant: TenantKey,
    },
    ServiceAccount {
        name: String,
        tenant: TenantKey,
    },
    System,
}

impl AuthPrincipal {
    pub fn tenant(&self) -> Option<&TenantKey> {
        match self {
            AuthPrincipal::Operator { tenant, .. } => Some(tenant),
            AuthPrincipal::ServiceAccount { tenant, .. } => Some(tenant),
            AuthPrincipal::System => None,
        }
    }
}

/// Seam for request authentication. Implementors resolve credentials
/// to an authenticated principal.
pub trait Authenticator {
    type Error;

    fn authenticate(&self, token: &str) -> Result<AuthPrincipal, Self::Error>;
}

/// Seam for authorization. Implementors check whether a principal
/// may access a given project scope.
pub trait Authorizer {
    type Error;

    fn authorize(&self, principal: &AuthPrincipal, project: &ProjectKey)
        -> Result<(), Self::Error>;
}

/// In-memory registry mapping bearer tokens to principals.
///
/// Used by `ServiceTokenAuthenticator` to validate operator service tokens
/// during local/self-hosted deployments. Uses interior mutability so tokens
/// can be registered without exclusive access after the registry is shared.
#[derive(Debug, Default)]
pub struct ServiceTokenRegistry {
    tokens: std::sync::RwLock<std::collections::HashMap<String, AuthPrincipal>>,
}

impl ServiceTokenRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, token: String, principal: AuthPrincipal) {
        self.tokens.write().unwrap().insert(token, principal);
    }

    pub fn validate(&self, token: &str) -> Option<AuthPrincipal> {
        self.tokens.read().unwrap().get(token).cloned()
    }

    pub fn len(&self) -> usize {
        self.tokens.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.read().unwrap().is_empty()
    }
}

/// `Authenticator` implementation backed by a `ServiceTokenRegistry`.
#[derive(Clone, Debug)]
pub struct ServiceTokenAuthenticator {
    registry: std::sync::Arc<ServiceTokenRegistry>,
}

impl ServiceTokenAuthenticator {
    pub fn new(registry: std::sync::Arc<ServiceTokenRegistry>) -> Self {
        Self { registry }
    }
}

impl Authenticator for ServiceTokenAuthenticator {
    type Error = String;

    fn authenticate(&self, token: &str) -> Result<AuthPrincipal, Self::Error> {
        self.registry
            .validate(token)
            .ok_or_else(|| format!("invalid service token: {token}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::OperatorId;
    use cairn_domain::tenancy::TenantKey;

    #[test]
    fn operator_principal_has_tenant() {
        let principal = AuthPrincipal::Operator {
            operator_id: OperatorId::new("op_1"),
            tenant: TenantKey::new("tenant_acme"),
        };
        assert!(principal.tenant().is_some());
        assert_eq!(
            principal.tenant().unwrap().tenant_id.as_str(),
            "tenant_acme"
        );
    }

    #[test]
    fn system_principal_has_no_tenant() {
        let principal = AuthPrincipal::System;
        assert!(principal.tenant().is_none());
    }
}
