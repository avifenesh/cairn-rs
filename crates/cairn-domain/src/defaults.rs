use crate::tenancy::Scope;
use serde::{Deserialize, Serialize};

/// A single defaults entry at a specific scope layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultsLayer {
    pub scope: Scope,
    pub key: String,
    pub value: serde_json::Value,
}

/// Resolves a configuration key by walking a scope chain from most specific
/// to least specific, returning the first match.
///
/// The scope chain is ordered project -> workspace -> tenant -> system
/// (most specific first).
pub trait DefaultsResolver: Send + Sync {
    fn resolve(&self, scope_chain: &[DefaultsLayer], key: &str) -> Option<serde_json::Value>;
}

/// Simple resolver that returns the first matching key in scope-chain order.
pub struct LayeredDefaultsResolver;

impl DefaultsResolver for LayeredDefaultsResolver {
    fn resolve(&self, scope_chain: &[DefaultsLayer], key: &str) -> Option<serde_json::Value> {
        scope_chain
            .iter()
            .find(|layer| layer.key == key)
            .map(|layer| layer.value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_returns_most_specific_match() {
        let chain = vec![
            DefaultsLayer {
                scope: Scope::Project,
                key: "timeout_ms".to_owned(),
                value: serde_json::json!(5000),
            },
            DefaultsLayer {
                scope: Scope::Workspace,
                key: "timeout_ms".to_owned(),
                value: serde_json::json!(10000),
            },
            DefaultsLayer {
                scope: Scope::Tenant,
                key: "timeout_ms".to_owned(),
                value: serde_json::json!(30000),
            },
        ];

        let resolver = LayeredDefaultsResolver;
        let result = resolver.resolve(&chain, "timeout_ms");
        assert_eq!(result, Some(serde_json::json!(5000)));
    }

    #[test]
    fn resolver_returns_none_for_missing_key() {
        let chain = vec![DefaultsLayer {
            scope: Scope::System,
            key: "other_key".to_owned(),
            value: serde_json::json!("value"),
        }];

        let resolver = LayeredDefaultsResolver;
        assert!(resolver.resolve(&chain, "timeout_ms").is_none());
    }

    #[test]
    fn resolver_falls_back_to_tenant_when_project_missing() {
        let chain = vec![
            DefaultsLayer {
                scope: Scope::Project,
                key: "other_setting".to_owned(),
                value: serde_json::json!(true),
            },
            DefaultsLayer {
                scope: Scope::Tenant,
                key: "timeout_ms".to_owned(),
                value: serde_json::json!(30000),
            },
        ];

        let resolver = LayeredDefaultsResolver;
        let result = resolver.resolve(&chain, "timeout_ms");
        assert_eq!(result, Some(serde_json::json!(30000)));
    }
}
