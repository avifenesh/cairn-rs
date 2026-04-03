//! External worker API endpoint boundary.
//!
//! Routes external worker heartbeats, progress, and outcome reports
//! through cairn-runtime's ExternalWorkerService.

use async_trait::async_trait;
use cairn_domain::tenancy::ProjectKey;
use cairn_runtime::error::RuntimeError;
use serde::{Deserialize, Serialize};

/// API-facing worker report request.
///
/// This stays API-local on purpose instead of re-exporting the domain
/// `ExternalWorkerReport`: the HTTP surface accepts string IDs plus
/// optional progress/outcome fields, then the API/runtime boundary
/// fills runtime-owned fields such as `reported_at_ms` before forwarding.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerReportRequest {
    pub worker_id: String,
    pub task_id: String,
    pub lease_token: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// API endpoint boundary for external worker operations.
#[async_trait]
pub trait ExternalWorkerEndpoints: Send + Sync {
    /// POST /v1/workers/report — accept a worker heartbeat/progress/outcome.
    async fn report(
        &self,
        project: &ProjectKey,
        request: &WorkerReportRequest,
    ) -> Result<(), RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_report_request_serialization() {
        let req = WorkerReportRequest {
            worker_id: "worker_1".to_owned(),
            task_id: "task_1".to_owned(),
            lease_token: 42,
            run_id: Some("run_1".to_owned()),
            message: Some("50% done".to_owned()),
            percent: Some(500),
            outcome: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["workerId"], "worker_1");
        assert_eq!(json["leaseToken"], 42);
        assert!(json.get("outcome").is_none()); // skipped when None
    }

    #[test]
    fn worker_report_with_outcome() {
        let req = WorkerReportRequest {
            worker_id: "worker_2".to_owned(),
            task_id: "task_2".to_owned(),
            lease_token: 7,
            run_id: None,
            message: None,
            percent: None,
            outcome: Some("completed".to_owned()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["outcome"], "completed");
    }
}
