//! Signal subscription and routing boundary for RFC 002 / RFC 010.

use async_trait::async_trait;
use cairn_domain::{MailboxMessageId, ProjectKey, RunId, SignalId, SignalSubscription};

use crate::error::RuntimeError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalRoutingResult {
    pub routed_count: u32,
    pub mailbox_message_ids: Vec<MailboxMessageId>,
}

#[async_trait]
pub trait SignalRouterService: Send + Sync {
    async fn subscribe(
        &self,
        project: ProjectKey,
        signal_kind: String,
        target_run_id: Option<RunId>,
        target_mailbox_id: Option<String>,
        filter_expression: Option<String>,
    ) -> Result<SignalSubscription, RuntimeError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SignalSubscription>, RuntimeError>;

    async fn route_signal(&self, signal_id: &SignalId)
        -> Result<SignalRoutingResult, RuntimeError>;
}
