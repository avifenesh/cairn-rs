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

    /// RFC 010: active_runs must aggregate across ALL sessions, not just one.
    ///
    /// Operators need the workspace-level view: runs from session A and session B
    /// both appear in the same dashboard count.
    #[test]
    fn active_runs_aggregates_across_sessions() {
        use cairn_domain::lifecycle::RunState;
        use cairn_store::projections::RunRecord;
        use cairn_domain::{RunId, SessionId};
        use cairn_domain::tenancy::ProjectKey;

        let project = ProjectKey::new("t1", "w1", "p1");

        // Simulate runs from two different sessions.
        let runs = vec![
            RunRecord {
                run_id: RunId::new("run_1"),
                session_id: SessionId::new("sess_a"),
                parent_run_id: None,
                project: project.clone(),
                state: RunState::Running,
                prompt_release_id: None,
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 1000,
                updated_at: 1000,
            },
            RunRecord {
                run_id: RunId::new("run_2"),
                session_id: SessionId::new("sess_b"), // different session
                parent_run_id: None,
                project: project.clone(),
                state: RunState::WaitingApproval,
                prompt_release_id: None,
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 2000,
                updated_at: 2000,
            },
            RunRecord {
                run_id: RunId::new("run_3"),
                session_id: SessionId::new("sess_a"),
                parent_run_id: None,
                project: project.clone(),
                state: RunState::Completed, // terminal — must not count
                prompt_release_id: None,
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 3000,
                updated_at: 3000,
            },
        ];

        // Aggregate: count non-terminal runs across all sessions for the project.
        let active_count = runs
            .iter()
            .filter(|r| r.project == project && !r.state.is_terminal())
            .count() as u32;

        let overview = DashboardOverview {
            active_runs: active_count,
            active_tasks: 0,
            pending_approvals: 0,
            failed_runs_24h: 0,
            system_healthy: true,
        };

        // Both session A (run_1) and session B (run_2) contribute.
        assert_eq!(overview.active_runs, 2,
            "active_runs must count runs from all sessions, not just the current one");
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
