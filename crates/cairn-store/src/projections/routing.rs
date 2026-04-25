use async_trait::async_trait;
use cairn_domain::providers::{ProviderCallRecord, RouteAttemptRecord, RouteDecisionRecord};
use cairn_domain::{ProjectKey, RouteAttemptId, RouteDecisionId, RunId};

use crate::error::StoreError;

/// Read-model for route decision records.
#[async_trait]
pub trait RouteDecisionReadModel: Send + Sync {
    async fn get(
        &self,
        decision_id: &RouteDecisionId,
    ) -> Result<Option<RouteDecisionRecord>, StoreError>;

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RouteDecisionRecord>, StoreError>;
}

/// Read-model for route attempt records (RFC 009 first-class entities).
#[async_trait]
pub trait RouteAttemptReadModel: Send + Sync {
    async fn get(
        &self,
        attempt_id: &RouteAttemptId,
    ) -> Result<Option<RouteAttemptRecord>, StoreError> {
        let _ = attempt_id;
        Ok(None)
    }

    async fn list_by_decision(
        &self,
        route_decision_id: &RouteDecisionId,
        limit: usize,
    ) -> Result<Vec<RouteAttemptRecord>, StoreError> {
        let _ = (route_decision_id, limit);
        Ok(vec![])
    }
}

/// Read-model for provider call records.
#[async_trait]
pub trait ProviderCallReadModel: Send + Sync {
    async fn get(
        &self,
        call_id: &cairn_domain::ProviderCallId,
    ) -> Result<Option<ProviderCallRecord>, StoreError>;

    async fn list_by_decision(
        &self,
        decision_id: &RouteDecisionId,
        limit: usize,
    ) -> Result<Vec<ProviderCallRecord>, StoreError>;

    /// List provider calls attached to a run, ordered by start time (ascending).
    ///
    /// Default implementation returns an empty vector so backends that do not
    /// yet surface run-scoped provider calls compile without panic.
    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<ProviderCallRecord>, StoreError> {
        let _ = (run_id, limit);
        Ok(vec![])
    }
}
