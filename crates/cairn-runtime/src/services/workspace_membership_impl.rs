use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{
    RuntimeEvent, WorkspaceKey, WorkspaceMemberAdded, WorkspaceMemberRemoved, WorkspaceMembership,
    WorkspaceRole,
};
use cairn_store::projections::{WorkspaceMembershipReadModel, WorkspaceReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::workspace_memberships::WorkspaceMembershipService;

pub struct WorkspaceMembershipServiceImpl<S> {
    store: Arc<S>,
}

impl<S> WorkspaceMembershipServiceImpl<S> {
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

#[async_trait]
impl<S> WorkspaceMembershipService for WorkspaceMembershipServiceImpl<S>
where
    S: EventLog + WorkspaceReadModel + WorkspaceMembershipReadModel + 'static,
{
    async fn add_member(
        &self,
        workspace_key: WorkspaceKey,
        member_id: String,
        role: WorkspaceRole,
    ) -> Result<WorkspaceMembership, RuntimeError> {
        if WorkspaceReadModel::get(self.store.as_ref(), &workspace_key.workspace_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "workspace",
                id: workspace_key.workspace_id.to_string(),
            });
        }

        if WorkspaceMembershipReadModel::get_member(self.store.as_ref(), &workspace_key, &member_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "workspace_membership",
                id: format!("{}:{member_id}", workspace_key.workspace_id),
            });
        }

        let event = make_envelope(RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
            workspace_key: workspace_key.clone(),
            member_id: cairn_domain::OperatorId::new(member_id.clone()),
            role,
            added_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;

        WorkspaceMembershipReadModel::get_member(self.store.as_ref(), &workspace_key, &member_id)
            .await?
            .map(|rec| WorkspaceMembership {
                workspace_id: workspace_key.workspace_id.clone(),
                operator_id: cairn_domain::OperatorId::new(rec.operator_id),
                role: rec.role,
            })
            .ok_or_else(|| {
                RuntimeError::Internal("workspace membership not found after add".to_owned())
            })
    }

    async fn list_members(
        &self,
        workspace_key: &WorkspaceKey,
    ) -> Result<Vec<WorkspaceMembership>, RuntimeError> {
        if WorkspaceReadModel::get(self.store.as_ref(), &workspace_key.workspace_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "workspace",
                id: workspace_key.workspace_id.to_string(),
            });
        }

        Ok(WorkspaceMembershipReadModel::list_workspace_members(
            self.store.as_ref(),
            workspace_key.workspace_id.as_str(),
        )
        .await?
        .into_iter()
        .map(|rec| WorkspaceMembership {
            workspace_id: workspace_key.workspace_id.clone(),
            operator_id: cairn_domain::OperatorId::new(rec.operator_id),
            role: rec.role,
        })
        .collect())
    }

    async fn remove_member(
        &self,
        workspace_key: WorkspaceKey,
        member_id: String,
    ) -> Result<(), RuntimeError> {
        if WorkspaceReadModel::get(self.store.as_ref(), &workspace_key.workspace_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "workspace",
                id: workspace_key.workspace_id.to_string(),
            });
        }

        if WorkspaceMembershipReadModel::get_member(self.store.as_ref(), &workspace_key, &member_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "workspace_membership",
                id: format!("{}:{member_id}", workspace_key.workspace_id),
            });
        }

        let event = make_envelope(RuntimeEvent::WorkspaceMemberRemoved(
            WorkspaceMemberRemoved {
                workspace_key,
                member_id: cairn_domain::OperatorId::new(member_id),
                removed_at_ms: now_ms(),
            },
        ));
        self.store.append(&[event]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{TenantId, WorkspaceId, WorkspaceKey, WorkspaceRole};
    use cairn_store::InMemoryStore;

    use crate::services::{WorkspaceMembershipServiceImpl, WorkspaceServiceImpl};
    use crate::workspace_memberships::WorkspaceMembershipService;
    use crate::workspaces::WorkspaceService;

    #[tokio::test]
    async fn add_list_remove_workspace_membership() {
        let store = Arc::new(InMemoryStore::new());
        let workspace_service = WorkspaceServiceImpl::new(store.clone());
        let membership_service = WorkspaceMembershipServiceImpl::new(store);

        workspace_service
            .create(
                TenantId::new("tenant_acme"),
                WorkspaceId::new("ws_ops"),
                "Operations".to_owned(),
            )
            .await
            .unwrap();

        let workspace_key = WorkspaceKey::new("tenant_acme", "ws_ops");

        membership_service
            .add_member(
                workspace_key.clone(),
                "alice".to_owned(),
                WorkspaceRole::Admin,
            )
            .await
            .unwrap();
        membership_service
            .add_member(
                workspace_key.clone(),
                "bob".to_owned(),
                WorkspaceRole::Member,
            )
            .await
            .unwrap();

        let members = membership_service
            .list_members(&workspace_key)
            .await
            .unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].operator_id.as_str(), "alice");
        assert_eq!(members[1].operator_id.as_str(), "bob");

        membership_service
            .remove_member(workspace_key.clone(), "alice".to_owned())
            .await
            .unwrap();

        let remaining = membership_service
            .list_members(&workspace_key)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].operator_id.as_str(), "bob");
    }
}
