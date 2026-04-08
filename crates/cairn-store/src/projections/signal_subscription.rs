use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SignalSubscriptionRecord {
    pub subscription_id: String,
    pub signal_type: String,
    pub target: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub project_tenant: String,
    #[serde(default)]
    pub project_workspace: String,
    #[serde(default)]
    pub project_id: String,
    /// Full project key carried for routing lookups.
    #[serde(skip)]
    pub project: Option<cairn_domain::tenancy::ProjectKey>,
    #[serde(default)]
    pub target_run_id: Option<cairn_domain::ids::RunId>,
    #[serde(default)]
    pub target_mailbox_id: Option<String>,
    #[serde(default)]
    pub filter_expression: Option<String>,
}

#[async_trait]
pub trait SignalSubscriptionReadModel: Send + Sync {
    async fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<SignalSubscriptionRecord>, StoreError>;

    async fn list_by_signal_type(
        &self,
        signal_type: &str,
    ) -> Result<Vec<SignalSubscriptionRecord>, StoreError>;

    /// Alias for list_by_signal_type — used by signal_router_impl.
    async fn list_by_signal_kind(
        &self,
        signal_kind: &str,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<SignalSubscriptionRecord>, StoreError>;

    /// List all subscriptions for a project scope.
    async fn list_by_project(
        &self,
        project: &cairn_domain::tenancy::ProjectKey,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<SignalSubscriptionRecord>, StoreError>;

    async fn upsert_subscription(&self, record: SignalSubscriptionRecord)
        -> Result<(), StoreError>;
}
