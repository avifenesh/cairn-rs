use async_trait::async_trait;
use cairn_domain::tenancy::ProjectKey;
use cairn_store::error::StoreError;
use serde::{Deserialize, Serialize};

/// Dashboard overview payload for `GET /v1/dashboard` per compatibility catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardOverview {
    pub active_runs: u32,
    pub active_tasks: u32,
    pub pending_approvals: u32,
    pub failed_runs_24h: u32,
    pub system_healthy: bool,
}

/// System status payload for `GET /v1/status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemStatus {
    pub runtime_ok: bool,
    pub store_ok: bool,
    pub uptime_secs: u64,
}

/// Cost summary payload for `GET /v1/costs`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostSummary {
    pub total_provider_calls: u64,
    pub total_tokens_used: u64,
}

/// Metrics read model for `GET /v1/metrics`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub total_runs: u64,
    pub total_tasks: u64,
    pub total_tool_invocations: u64,
    pub total_approvals: u64,
}

/// Overview endpoint trait combining dashboard, status, costs, and metrics.
#[async_trait]
pub trait OverviewEndpoints: Send + Sync {
    /// `GET /v1/dashboard`
    async fn dashboard(&self, project: &ProjectKey) -> Result<DashboardOverview, StoreError>;

    /// `GET /v1/status`
    async fn status(&self) -> Result<SystemStatus, StoreError>;

    /// `GET /v1/costs`
    async fn costs(&self, project: &ProjectKey) -> Result<CostSummary, StoreError>;

    /// `GET /v1/metrics`
    async fn metrics(&self, project: &ProjectKey) -> Result<MetricsSummary, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_overview_serialization() {
        let overview = DashboardOverview {
            active_runs: 3,
            active_tasks: 7,
            pending_approvals: 2,
            failed_runs_24h: 1,
            system_healthy: true,
        };
        let json = serde_json::to_value(&overview).unwrap();
        assert_eq!(json["active_runs"], 3);
        assert_eq!(json["system_healthy"], true);
    }

    #[test]
    fn system_status_serialization() {
        let status = SystemStatus {
            runtime_ok: true,
            store_ok: true,
            uptime_secs: 3600,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["uptime_secs"], 3600);
    }

    #[test]
    fn metrics_summary_serialization() {
        let metrics = MetricsSummary {
            total_runs: 100,
            total_tasks: 500,
            total_tool_invocations: 1200,
            total_approvals: 42,
        };
        let json = serde_json::to_value(&metrics).unwrap();
        assert_eq!(json["total_tool_invocations"], 1200);
    }
}
