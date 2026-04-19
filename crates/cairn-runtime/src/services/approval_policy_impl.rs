use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cairn_domain::{
    ApprovalPolicyCreated, ApprovalPolicyRecord, ProjectKey, PromptReleaseId, RuntimeEvent,
    TenantId, WorkspaceRole,
};
use cairn_store::projections::ApprovalPolicyReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::approval_policies::ApprovalPolicyService;
use crate::error::RuntimeError;

static APPROVAL_POLICY_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct ApprovalPolicyServiceImpl<S> {
    store: Arc<S>,
    attachments: Mutex<HashMap<String, Vec<PromptReleaseId>>>,
}

impl<S> ApprovalPolicyServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            attachments: Mutex::new(HashMap::new()),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn next_policy_id() -> String {
    format!(
        "approval_policy_{}",
        APPROVAL_POLICY_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[async_trait]
impl<S> ApprovalPolicyService for ApprovalPolicyServiceImpl<S>
where
    S: EventLog + ApprovalPolicyReadModel + 'static,
{
    async fn create(
        &self,
        tenant_id: TenantId,
        name: String,
        required_approvers: u32,
        allowed_approver_roles: Vec<WorkspaceRole>,
        auto_approve_after_ms: Option<u64>,
        auto_reject_after_ms: Option<u64>,
    ) -> Result<ApprovalPolicyRecord, RuntimeError> {
        let policy_id = next_policy_id();
        let event = make_envelope(RuntimeEvent::ApprovalPolicyCreated(ApprovalPolicyCreated {
            project: ProjectKey::new(tenant_id.as_str(), "", ""),
            tenant_id,
            policy_id: policy_id.clone(),
            name,
            required_approvers,
            allowed_approver_roles,
            auto_approve_after_ms,
            auto_reject_after_ms,
            created_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;
        self.get(&policy_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("approval policy not found after create".into()))
    }

    async fn get(&self, policy_id: &str) -> Result<Option<ApprovalPolicyRecord>, RuntimeError> {
        let mut policy =
            ApprovalPolicyReadModel::get_policy(self.store.as_ref(), policy_id).await?;
        if let Some(record) = policy.as_mut() {
            if let Some(attached) = self
                .attachments
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(policy_id)
                .cloned()
            {
                record.attached_release_ids = attached;
            }
        }
        Ok(policy)
    }

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalPolicyRecord>, RuntimeError> {
        let mut policies =
            ApprovalPolicyReadModel::list_by_tenant(self.store.as_ref(), tenant_id, limit, offset)
                .await?;
        let attachments = self.attachments.lock().unwrap_or_else(|e| e.into_inner());
        for policy in &mut policies {
            if let Some(attached) = attachments.get(&policy.policy_id).cloned() {
                policy.attached_release_ids = attached;
            }
        }
        Ok(policies)
    }

    async fn attach_to_release(
        &self,
        policy_id: &str,
        release_id: PromptReleaseId,
    ) -> Result<ApprovalPolicyRecord, RuntimeError> {
        let mut policy = self
            .get(policy_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "approval_policy",
                id: policy_id.to_owned(),
            })?;

        let mut attachments = self.attachments.lock().unwrap_or_else(|e| e.into_inner());
        let attached = attachments.entry(policy_id.to_owned()).or_default();
        if !attached.iter().any(|existing| existing == &release_id) {
            attached.push(release_id);
        }
        policy.attached_release_ids = attached.clone();
        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use cairn_domain::events::EventEnvelope;
    use cairn_domain::RuntimeEvent;
    use cairn_domain::{ApprovalPolicyRecord, PromptReleaseId, TenantId, WorkspaceRole};
    use cairn_store::error::StoreError;
    use cairn_store::event_log::EventPosition;
    use cairn_store::projections::ApprovalPolicyReadModel;

    use crate::approval_policies::ApprovalPolicyService;
    use crate::services::ApprovalPolicyServiceImpl;

    /// Minimal mock store for approval policy tests.
    struct MockPolicyStore {
        policies: std::sync::Mutex<HashMap<String, ApprovalPolicyRecord>>,
    }

    impl MockPolicyStore {
        fn new() -> Self {
            Self {
                policies: std::sync::Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl cairn_store::event_log::EventLog for MockPolicyStore {
        async fn append(
            &self,
            events: &[EventEnvelope<RuntimeEvent>],
        ) -> Result<Vec<EventPosition>, StoreError> {
            // Project ApprovalPolicyCreated events into the mock store.
            let mut policies = self.policies.lock().unwrap_or_else(|e| e.into_inner());
            for envelope in events {
                if let RuntimeEvent::ApprovalPolicyCreated(e) = &envelope.payload {
                    policies.insert(
                        e.policy_id.clone(),
                        ApprovalPolicyRecord {
                            policy_id: e.policy_id.clone(),
                            tenant_id: e.tenant_id.clone(),
                            name: e.name.clone(),
                            required_approvers: e.required_approvers,
                            allowed_approver_roles: e.allowed_approver_roles.clone(),
                            auto_approve_after_ms: e.auto_approve_after_ms,
                            auto_reject_after_ms: e.auto_reject_after_ms,
                            attached_release_ids: vec![],
                        },
                    );
                }
            }
            Ok((0..events.len()).map(|i| EventPosition(i as u64)).collect())
        }

        async fn read_by_entity(
            &self,
            _entity: &cairn_store::event_log::EntityRef,
            _after: Option<EventPosition>,
            _limit: usize,
        ) -> Result<Vec<cairn_store::event_log::StoredEvent>, StoreError> {
            Ok(vec![])
        }

        async fn read_stream(
            &self,
            _after: Option<EventPosition>,
            _limit: usize,
        ) -> Result<Vec<cairn_store::event_log::StoredEvent>, StoreError> {
            Ok(vec![])
        }

        async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
            Ok(None)
        }

        async fn find_by_causation_id(
            &self,
            _causation_id: &str,
        ) -> Result<Option<EventPosition>, StoreError> {
            Ok(None)
        }
    }

    #[async_trait]
    impl ApprovalPolicyReadModel for MockPolicyStore {
        async fn get_policy(
            &self,
            policy_id: &str,
        ) -> Result<Option<ApprovalPolicyRecord>, StoreError> {
            Ok(self
                .policies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(policy_id)
                .cloned())
        }

        async fn list_by_tenant(
            &self,
            tenant_id: &TenantId,
            _limit: usize,
            _offset: usize,
        ) -> Result<Vec<ApprovalPolicyRecord>, StoreError> {
            Ok(self
                .policies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .filter(|p| &p.tenant_id == tenant_id)
                .cloned()
                .collect())
        }
    }

    #[tokio::test]
    async fn policy_create_returns_record_with_correct_fields() {
        let store = Arc::new(MockPolicyStore::new());
        let svc = ApprovalPolicyServiceImpl::new(store);

        let policy = svc
            .create(
                TenantId::new("tenant_1"),
                "Compliance Review".to_owned(),
                2,
                vec![WorkspaceRole::Admin],
                Some(3_600_000),
                None,
            )
            .await
            .unwrap();

        assert!(!policy.policy_id.is_empty());
        assert_eq!(policy.name, "Compliance Review");
        assert_eq!(policy.required_approvers, 2);
        assert_eq!(policy.allowed_approver_roles, vec![WorkspaceRole::Admin]);
        assert_eq!(policy.auto_approve_after_ms, Some(3_600_000));
        assert!(policy.attached_release_ids.is_empty());
    }

    #[tokio::test]
    async fn attach_to_release_adds_release_to_policy() {
        let store = Arc::new(MockPolicyStore::new());
        let svc = ApprovalPolicyServiceImpl::new(store);

        let policy = svc
            .create(
                TenantId::new("t1"),
                "Gate".to_owned(),
                1,
                vec![],
                None,
                None,
            )
            .await
            .unwrap();

        let release_id = PromptReleaseId::new("release_1");
        let updated = svc
            .attach_to_release(&policy.policy_id, release_id.clone())
            .await
            .unwrap();
        assert_eq!(updated.attached_release_ids, vec![release_id]);
    }

    #[tokio::test]
    async fn attach_to_release_is_idempotent() {
        let store = Arc::new(MockPolicyStore::new());
        let svc = ApprovalPolicyServiceImpl::new(store);

        let policy = svc
            .create(
                TenantId::new("t1"),
                "Gate".to_owned(),
                1,
                vec![],
                None,
                None,
            )
            .await
            .unwrap();

        let release_id = PromptReleaseId::new("release_1");
        svc.attach_to_release(&policy.policy_id, release_id.clone())
            .await
            .unwrap();
        let final_policy = svc
            .attach_to_release(&policy.policy_id, release_id)
            .await
            .unwrap();
        assert_eq!(
            final_policy.attached_release_ids.len(),
            1,
            "duplicate attach must not add a second entry"
        );
    }
}
