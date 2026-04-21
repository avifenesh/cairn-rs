//! Admin-only RFC-011 debug endpoint.
//!
//! ⚠ SECURITY: the entire module is gated behind the `debug-endpoints`
//! Cargo feature, OFF by default. Production release builds MUST NOT
//! enable this feature. See `SECURITY.md` for the full threat model.
//!
//! When compiled in, exposes `GET /v1/admin/debug/partition?kind=<run|
//! task>&id=<id>` which returns the FF ExecutionId and Valkey partition
//! placement for a given run or task. The endpoint is used by RFC-011
//! co-location integration tests and should not be reachable in any
//! production deployment.
//!
//! Information returned includes:
//!   * `execution_id` — FF-internal id; never otherwise on the HTTP surface
//!   * `partition_index` — Valkey shard placement for this execution
//!   * `partition_hash_tag` — `{fp:N}` string
//!   * `derivation` — `session_flow` (co-located via session) or `solo` (bare A2A task)
//!
//! Auth: [`AdminRoleGuard`] fails closed with 403 for non-admin callers.
//! Absent the feature, the route is not registered and an identical
//! request returns 404.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use ff_core::partition::execution_partition;

use cairn_domain::{ProjectKey, RunId, TaskId};
use cairn_fabric::id_map;
use cairn_store::projections::{RunReadModel, TaskReadModel};

use crate::errors::AppApiError;
use crate::extractors::AdminRoleGuard;
use crate::state::AppState;

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct DebugPartitionQuery {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct DebugPartitionResponse {
    pub kind: String,
    pub id: String,
    pub execution_id: String,
    pub partition_index: u16,
    pub partition_hash_tag: String,
    pub session_id: Option<String>,
    pub project: ProjectKey,
    pub derivation: &'static str,
}

/// `GET /v1/admin/debug/partition?kind=<run|task>&id=<id>`
///
/// See module-level rustdoc for the security contract. Feature-gated by
/// `debug-endpoints`; route is not registered without the feature.
pub(crate) async fn debug_partition_handler(
    _admin: AdminRoleGuard,
    State(state): State<Arc<AppState>>,
    Query(query): Query<DebugPartitionQuery>,
) -> impl IntoResponse {
    // `fabric` must be `Some` in any production (or production-like)
    // deployment; the only path to `None` is a read-only test fixture
    // that never installs Fabric at all. Surface a 503 in that case so
    // the endpoint is honest about what it cannot answer.
    let Some(fabric) = state.fabric.as_ref() else {
        return AppApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "fabric_unavailable",
            "debug endpoint requires the fabric runtime; this deployment has none",
        )
        .into_response();
    };
    let partition_config = fabric.runtime.partition_config;
    let store = &state.runtime.store;

    match query.kind.as_str() {
        "run" => {
            let run_id = RunId::new(query.id.clone());
            let record = match RunReadModel::get(store.as_ref(), &run_id).await {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "run not found")
                        .into_response();
                }
                Err(err) => {
                    return AppApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "store_error",
                        err.to_string(),
                    )
                    .into_response();
                }
            };
            let eid = id_map::session_run_to_execution_id(
                &record.project,
                &record.session_id,
                &record.run_id,
                &partition_config,
            );
            let partition = execution_partition(&eid, &partition_config);
            let resp = DebugPartitionResponse {
                kind: "run".to_owned(),
                id: record.run_id.to_string(),
                execution_id: eid.to_string(),
                partition_index: partition.index,
                partition_hash_tag: partition.to_string(),
                session_id: Some(record.session_id.to_string()),
                project: record.project.clone(),
                derivation: "session_flow",
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        "task" => {
            let task_id = TaskId::new(query.id.clone());
            let record = match TaskReadModel::get(store.as_ref(), &task_id).await {
                Ok(Some(t)) => t,
                Ok(None) => {
                    return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "task not found")
                        .into_response();
                }
                Err(err) => {
                    return AppApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "store_error",
                        err.to_string(),
                    )
                    .into_response();
                }
            };
            let resolved_session = match record.session_id.clone() {
                Some(sid) => Some(sid),
                None => match &record.parent_run_id {
                    Some(prid) => match RunReadModel::get(store.as_ref(), prid).await {
                        Ok(Some(run)) => Some(run.session_id),
                        Ok(None) => None,
                        Err(err) => {
                            return AppApiError::new(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "store_error",
                                err.to_string(),
                            )
                            .into_response();
                        }
                    },
                    None => None,
                },
            };
            let (eid, derivation) = match resolved_session.as_ref() {
                Some(sid) => (
                    id_map::session_task_to_execution_id(
                        &record.project,
                        sid,
                        &record.task_id,
                        &partition_config,
                    ),
                    "session_flow",
                ),
                None => (
                    id_map::task_to_execution_id(
                        &record.project,
                        &record.task_id,
                        &partition_config,
                    ),
                    "solo",
                ),
            };
            let partition = execution_partition(&eid, &partition_config);
            let resp = DebugPartitionResponse {
                kind: "task".to_owned(),
                id: record.task_id.to_string(),
                execution_id: eid.to_string(),
                partition_index: partition.index,
                partition_hash_tag: partition.to_string(),
                session_id: resolved_session.map(|s| s.to_string()),
                project: record.project.clone(),
                derivation,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        other => AppApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_kind",
            format!("kind must be 'run' or 'task', got: {other}"),
        )
        .into_response(),
    }
}
