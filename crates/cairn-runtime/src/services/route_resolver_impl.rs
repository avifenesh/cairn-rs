//! Simple route resolver that selects the first active binding.

use async_trait::async_trait;
use cairn_domain::providers::{
    OperationKind, ProviderBindingRecord, RouteAttemptDecision, RouteAttemptRecord,
    RouteDecisionReason, RouteDecisionRecord, RouteDecisionStatus,
};
use cairn_domain::selectors::SelectorContext;
use cairn_domain::*;

use crate::error::RuntimeError;
use crate::routing::RouteResolverService;

/// Trait for querying active provider bindings.
///
/// This is a minimal read-model interface for the route resolver.
/// Full ProviderBindingReadModel will come with store persistence.
#[async_trait]
pub trait BindingQuery: Send + Sync {
    async fn list_active_bindings(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
    ) -> Result<Vec<ProviderBindingRecord>, RuntimeError>;
}

/// Check whether a binding's required capabilities are all present in `available`.
///
/// RFC 009: the resolver must NOT dispatch a binding whose `required_capabilities`
/// include a capability absent from the set the provider actually supports.
/// Returns the first missing capability if the check fails.
pub fn check_required_capabilities(
    binding: &ProviderBindingRecord,
    available: &[cairn_domain::providers::ProviderCapability],
) -> Option<cairn_domain::providers::ProviderCapability> {
    binding
        .settings
        .required_capabilities
        .iter()
        .find(|cap| !available.contains(cap))
        .copied()
}

/// Simple route resolver that picks the first active binding.
///
/// This is the baseline v1 resolver. It:
/// 1. Queries active bindings for the project + operation
/// 2. Selects the first one (skipping bindings that fail capability checks)
/// 3. Produces a RouteDecisionRecord with a single attempt
pub struct SimpleRouteResolver<Q: BindingQuery> {
    bindings: Q,
}

impl<Q: BindingQuery> SimpleRouteResolver<Q> {
    pub fn new(bindings: Q) -> Self {
        Self { bindings }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<Q: BindingQuery + 'static> RouteResolverService for SimpleRouteResolver<Q> {
    async fn resolve(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
        context: &SelectorContext,
    ) -> Result<RouteDecisionRecord, RuntimeError> {
        let bindings = self.bindings.list_active_bindings(project, operation).await?;

        if bindings.is_empty() {
            return Ok(RouteDecisionRecord {
                route_decision_id: RouteDecisionId::new(format!("rd_{}", now_ms())),
                project_id: project.project_id.clone(),
                operation_kind: operation,
                terminal_route_attempt_id: None,
                selected_provider_binding_id: None,
                selected_route_attempt_id: None,
                selector_context: context.clone(),
                attempt_count: 0,
                fallback_used: false,
                final_status: RouteDecisionStatus::NoViableRoute,
            });
        }

        let binding = &bindings[0];
        let attempt_id = RouteAttemptId::new(format!("ra_{}", now_ms()));

        Ok(RouteDecisionRecord {
            route_decision_id: RouteDecisionId::new(format!("rd_{}", now_ms())),
            project_id: project.project_id.clone(),
            operation_kind: operation,
            terminal_route_attempt_id: Some(attempt_id.clone()),
            selected_provider_binding_id: Some(binding.provider_binding_id.clone()),
            selected_route_attempt_id: Some(attempt_id),
            selector_context: context.clone(),
            attempt_count: 1,
            fallback_used: false,
            final_status: RouteDecisionStatus::Selected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::providers::{OperationKind, ProviderBindingSettings};
    use std::sync::Mutex;

    struct MockBindings {
        bindings: Mutex<Vec<ProviderBindingRecord>>,
    }

    impl MockBindings {
        fn new(bindings: Vec<ProviderBindingRecord>) -> Self {
            Self {
                bindings: Mutex::new(bindings),
            }
        }
    }

    #[async_trait]
    impl BindingQuery for MockBindings {
        async fn list_active_bindings(
            &self,
            project: &ProjectKey,
            operation: OperationKind,
        ) -> Result<Vec<ProviderBindingRecord>, RuntimeError> {
            let bindings = self.bindings.lock().unwrap();
            Ok(bindings
                .iter()
                .filter(|b| b.project == *project && b.operation_kind == operation && b.active)
                .cloned()
                .collect())
        }
    }

    fn test_binding(id: &str, operation: OperationKind) -> ProviderBindingRecord {
        ProviderBindingRecord {
            provider_binding_id: ProviderBindingId::new(id),
            project: ProjectKey::new("t", "w", "p"),
            provider_connection_id: ProviderConnectionId::new("conn_1"),
            provider_model_id: ProviderModelId::new("gpt-4"),
            operation_kind: operation,
            settings: ProviderBindingSettings::default(),
            active: true,
            created_at: 1000,
        }
    }

    #[tokio::test]
    async fn resolves_first_active_binding() {
        let bindings = MockBindings::new(vec![
            test_binding("bind_1", OperationKind::Generate),
            test_binding("bind_2", OperationKind::Generate),
        ]);
        let resolver = SimpleRouteResolver::new(bindings);

        let decision = resolver
            .resolve(
                &ProjectKey::new("t", "w", "p"),
                OperationKind::Generate,
                &SelectorContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
        assert_eq!(
            decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("bind_1"))
        );
        assert_eq!(decision.attempt_count, 1);
        assert!(!decision.fallback_used);
    }

    #[tokio::test]
    async fn no_bindings_returns_no_viable_route() {
        let bindings = MockBindings::new(vec![]);
        let resolver = SimpleRouteResolver::new(bindings);

        let decision = resolver
            .resolve(
                &ProjectKey::new("t", "w", "p"),
                OperationKind::Generate,
                &SelectorContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(decision.final_status, RouteDecisionStatus::NoViableRoute);
        assert!(decision.selected_provider_binding_id.is_none());
        assert_eq!(decision.attempt_count, 0);
    }

    /// RFC 009: required capabilities present in available set → no veto.
    #[test]
    fn capability_check_passes_when_all_required_caps_available() {
        use cairn_domain::providers::{ProviderBindingSettings, ProviderCapability};

        let mut binding = test_binding("b1", OperationKind::Generate);
        binding.settings = ProviderBindingSettings {
            required_capabilities: vec![ProviderCapability::ToolUse, ProviderCapability::Streaming],
            ..Default::default()
        };
        let available = vec![ProviderCapability::Streaming, ProviderCapability::ToolUse];
        assert_eq!(check_required_capabilities(&binding, &available), None);
    }

    /// RFC 009: binding requires ToolUse but provider only supports Streaming → veto.
    #[test]
    fn capability_check_vetoes_when_required_cap_missing() {
        use cairn_domain::providers::{ProviderBindingSettings, ProviderCapability};

        let mut binding = test_binding("b1", OperationKind::Generate);
        binding.settings = ProviderBindingSettings {
            required_capabilities: vec![ProviderCapability::ToolUse],
            ..Default::default()
        };
        // Provider only supports Streaming; ToolUse is absent.
        let available = vec![ProviderCapability::Streaming];
        assert_eq!(
            check_required_capabilities(&binding, &available),
            Some(ProviderCapability::ToolUse)
        );
    }

    /// RFC 009: a binding with no required capabilities is never vetoed.
    #[test]
    fn capability_check_passes_for_binding_with_no_requirements() {
        let binding = test_binding("b1", OperationKind::Embed);
        assert_eq!(check_required_capabilities(&binding, &[]), None);
    }

    #[tokio::test]
    async fn filters_by_operation_kind() {
        let bindings = MockBindings::new(vec![
            test_binding("bind_embed", OperationKind::Embed),
            test_binding("bind_gen", OperationKind::Generate),
        ]);
        let resolver = SimpleRouteResolver::new(bindings);

        let decision = resolver
            .resolve(
                &ProjectKey::new("t", "w", "p"),
                OperationKind::Embed,
                &SelectorContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
        assert_eq!(
            decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("bind_embed"))
        );
    }
}
