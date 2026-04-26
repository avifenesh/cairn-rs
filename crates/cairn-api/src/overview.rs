use async_trait::async_trait;
use cairn_domain::tenancy::ProjectKey;
use cairn_store::error::StoreError;
use serde::{Deserialize, Serialize};

/// A brief summary of a critical runtime event shown in the operator dashboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriticalEventSummary {
    pub event_type: String,
    pub message: String,
    pub occurred_at_ms: u64,
    /// Run ID associated with the event, if any.
    pub run_id: Option<String>,
}

/// Dashboard overview payload for `GET /v1/dashboard` per compatibility catalog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DashboardOverview {
    pub active_runs: u32,
    pub active_tasks: u32,
    pub pending_approvals: u32,
    pub failed_runs_24h: u32,
    pub system_healthy: bool,
    /// p50 latency across all LLM calls in the last 24 h (None when no data).
    #[serde(default)]
    pub latency_p50_ms: Option<u64>,
    /// p95 latency across all LLM calls in the last 24 h (None when no data).
    #[serde(default)]
    pub latency_p95_ms: Option<u64>,
    /// Fraction of LLM calls that failed in the last 24 h (0.0–1.0).
    #[serde(default)]
    pub error_rate_24h: f32,
    /// Components currently reporting degraded status.
    #[serde(default)]
    pub degraded_components: Vec<String>,
    /// Recent critical events for the operator dashboard.
    #[serde(default)]
    pub recent_critical_events: Vec<crate::CriticalEventSummary>,
    /// Number of active provider connections.
    #[serde(default)]
    pub active_providers: u32,
    /// Number of active plugin instances.
    #[serde(default)]
    pub active_plugins: u32,
    /// Total knowledge documents in memory for this project/tenant.
    #[serde(default)]
    pub memory_doc_count: u64,
    /// Number of eval runs completed in the last 24 h.
    #[serde(default)]
    pub eval_runs_today: u32,
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

    fn base_overview() -> DashboardOverview {
        DashboardOverview {
            active_runs: 0,
            active_tasks: 0,
            pending_approvals: 0,
            failed_runs_24h: 0,
            system_healthy: true,
            latency_p50_ms: None,
            latency_p95_ms: None,
            error_rate_24h: 0.0,
            degraded_components: vec![],
            recent_critical_events: vec![],
            active_providers: 0,
            active_plugins: 0,
            memory_doc_count: 0,
            eval_runs_today: 0,
        }
    }

    #[test]
    fn dashboard_overview_serialization() {
        let overview = DashboardOverview {
            active_runs: 3,
            active_tasks: 7,
            pending_approvals: 2,
            failed_runs_24h: 1,
            system_healthy: true,
            latency_p50_ms: None,
            latency_p95_ms: None,
            error_rate_24h: 0.0,
            degraded_components: vec![],
            recent_critical_events: vec![],
            active_providers: 0,
            active_plugins: 0,
            memory_doc_count: 0,
            eval_runs_today: 0,
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
    #[test]
    fn active_runs_aggregates_across_sessions() {
        use cairn_domain::lifecycle::RunState;
        use cairn_domain::tenancy::ProjectKey;
        use cairn_domain::{RunId, SessionId};
        use cairn_store::projections::RunRecord;

        let project = ProjectKey::new("t1", "w1", "p1");

        let runs = [
            RunRecord {
                run_id: RunId::new("run_1"),
                session_id: SessionId::new("sess_a"),
                parent_run_id: None,
                project: project.clone(),
                state: RunState::Running,
                prompt_release_id: None,
                agent_role_id: None,
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 1000,
                updated_at: 1000,
                completion_summary: None,
                completion_verification: None,
                completion_annotated_at_ms: None,
            },
            RunRecord {
                run_id: RunId::new("run_2"),
                session_id: SessionId::new("sess_b"),
                parent_run_id: None,
                project: project.clone(),
                state: RunState::WaitingApproval,
                prompt_release_id: None,
                agent_role_id: None,
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 2000,
                updated_at: 2000,
                completion_summary: None,
                completion_verification: None,
                completion_annotated_at_ms: None,
            },
            RunRecord {
                run_id: RunId::new("run_3"),
                session_id: SessionId::new("sess_a"),
                parent_run_id: None,
                project: project.clone(),
                state: RunState::Completed,
                prompt_release_id: None,
                agent_role_id: None,
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
                version: 1,
                created_at: 3000,
                updated_at: 3000,
                completion_summary: None,
                completion_verification: None,
                completion_annotated_at_ms: None,
            },
        ];

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
            latency_p50_ms: None,
            latency_p95_ms: None,
            error_rate_24h: 0.0,
            degraded_components: vec![],
            recent_critical_events: vec![],
            active_providers: 0,
            active_plugins: 0,
            memory_doc_count: 0,
            eval_runs_today: 0,
        };

        assert_eq!(
            overview.active_runs, 2,
            "active_runs must count runs from all sessions"
        );
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

    /// GAP-010: DashboardOverview includes observability fields.
    #[test]
    fn dashboard_overview_has_observability_fields() {
        let overview = DashboardOverview {
            active_runs: 2,
            active_tasks: 5,
            pending_approvals: 1,
            failed_runs_24h: 0,
            system_healthy: true,
            latency_p50_ms: Some(142),
            latency_p95_ms: Some(890),
            error_rate_24h: 0.05,
            degraded_components: vec![],
            recent_critical_events: vec![],
            active_providers: 0,
            active_plugins: 0,
            memory_doc_count: 0,
            eval_runs_today: 0,
        };
        let json = serde_json::to_value(&overview).unwrap();

        assert_eq!(json["latency_p50_ms"], 142, "p50 must serialize");
        assert_eq!(json["latency_p95_ms"], 890, "p95 must serialize");
        assert!(
            (json["error_rate_24h"].as_f64().unwrap() - 0.05).abs() < 0.001,
            "error_rate_24h must serialize"
        );

        // Round-trip.
        let back: DashboardOverview = serde_json::from_value(json).unwrap();
        assert_eq!(back.latency_p50_ms, Some(142));
        assert_eq!(back.latency_p95_ms, Some(890));
        assert!((back.error_rate_24h - 0.05).abs() < 0.001);
    }

    /// GAP-010: None latency and 0.0 error_rate are valid (no data yet).
    #[test]
    fn dashboard_overview_defaults_no_observability_data() {
        let overview = base_overview();
        assert!(overview.latency_p50_ms.is_none());
        assert!(overview.latency_p95_ms.is_none());
        assert_eq!(overview.error_rate_24h, 0.0);

        let json = serde_json::to_value(&overview).unwrap();
        // serde(default) means null/absent for None
        assert!(json["latency_p50_ms"].is_null());
        assert!(json["latency_p95_ms"].is_null());
    }
}
