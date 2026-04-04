use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{
    DefaultSetting, DefaultSettingCleared, DefaultSettingSet, DefaultsLayer, DefaultsResolver,
    LayeredDefaultsResolver, ProjectKey, RuntimeEvent, Scope,
};
use cairn_store::projections::DefaultsReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::defaults::DefaultsService;
use crate::error::RuntimeError;

pub struct DefaultsServiceImpl<S> {
    store: Arc<S>,
    resolver: LayeredDefaultsResolver,
}

impl<S> DefaultsServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            resolver: LayeredDefaultsResolver,
        }
    }
}

fn normalized_scope_id(scope: Scope, scope_id: &str) -> String {
    match scope {
        Scope::System => "system".to_owned(),
        _ => scope_id.to_owned(),
    }
}

#[async_trait]
impl<S> DefaultsService for DefaultsServiceImpl<S>
where
    S: EventLog + DefaultsReadModel + Send + Sync + 'static,
{
    async fn set(
        &self,
        scope: Scope,
        scope_id: String,
        key: String,
        value: serde_json::Value,
    ) -> Result<DefaultSetting, RuntimeError> {
        let normalized_scope_id = normalized_scope_id(scope, &scope_id);
        let event = make_envelope(RuntimeEvent::DefaultSettingSet(DefaultSettingSet {
            scope,
            scope_id: normalized_scope_id.clone(),
            key: key.clone(),
            value,
        }));
        self.store.append(&[event]).await?;
        DefaultsReadModel::get(self.store.as_ref(), scope, &normalized_scope_id, &key)
            .await?
            .ok_or_else(|| RuntimeError::Internal("default setting not found after set".to_owned()))
    }

    async fn clear(&self, scope: Scope, scope_id: String, key: String) -> Result<(), RuntimeError> {
        let normalized_scope_id = normalized_scope_id(scope, &scope_id);
        let event = make_envelope(RuntimeEvent::DefaultSettingCleared(DefaultSettingCleared {
            scope,
            scope_id: normalized_scope_id,
            key,
        }));
        self.store.append(&[event]).await?;
        Ok(())
    }

    async fn resolve(
        &self,
        project_key: &ProjectKey,
        key: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let mut layers = Vec::new();
        for (scope, scope_id) in [
            (Scope::Project, project_key.project_id.as_str()),
            (Scope::Workspace, project_key.workspace_id.as_str()),
            (Scope::Tenant, project_key.tenant_id.as_str()),
            (Scope::System, "system"),
        ] {
            if let Some(setting) =
                DefaultsReadModel::get(self.store.as_ref(), scope, scope_id, key).await?
            {
                layers.push(DefaultsLayer {
                    scope: setting.scope,
                    key: setting.key,
                    value: setting.value,
                });
            }
        }
        Ok(self.resolver.resolve(&layers, key))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{ProjectKey, Scope};
    use cairn_store::InMemoryStore;

    use crate::defaults::DefaultsService;
    use crate::services::DefaultsServiceImpl;

    #[tokio::test]
    async fn defaults_resolve_workspace_override_then_tenant_fallback_after_clear() {
        let store = Arc::new(InMemoryStore::new());
        let service = DefaultsServiceImpl::new(store);
        let project = ProjectKey::new("tenant_defaults", "ws_defaults", "project_defaults");

        service
            .set(
                Scope::Tenant,
                "tenant_defaults".to_owned(),
                "model".to_owned(),
                serde_json::json!("gpt-4"),
            )
            .await
            .unwrap();
        service
            .set(
                Scope::Workspace,
                "ws_defaults".to_owned(),
                "model".to_owned(),
                serde_json::json!("gpt-3.5"),
            )
            .await
            .unwrap();

        let first = service.resolve(&project, "model").await.unwrap();
        assert_eq!(first, Some(serde_json::json!("gpt-3.5")));

        service
            .clear(
                Scope::Workspace,
                "ws_defaults".to_owned(),
                "model".to_owned(),
            )
            .await
            .unwrap();

        let second = service.resolve(&project, "model").await.unwrap();
        assert_eq!(second, Some(serde_json::json!("gpt-4")));
    }
}
