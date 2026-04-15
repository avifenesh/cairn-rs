//! Plugin CRUD, health, capabilities, and tool search HTTP handlers.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::http::ListResponse;
use cairn_domain::ProjectKey;
use cairn_tools::{
    build_eval_score_request, PluginCapability, PluginHost, PluginLifecycleSnapshot,
    PluginLogEntry, PluginManifest, PluginMetrics, PluginRegistry, PluginState,
    PluginToolDescriptor,
};

use crate::errors::AppApiError;
use crate::state::AppState;
use crate::{DEFAULT_PROJECT_ID, DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID};

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PluginCapabilityStatusItem {
    pub(crate) capability: PluginCapability,
    pub(crate) verified: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PluginCapabilitiesResponse {
    pub(crate) plugin_id: String,
    pub(crate) capabilities: Vec<PluginCapabilityStatusItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PluginToolsResponse {
    pub(crate) plugin_id: String,
    pub(crate) tools: Vec<PluginToolDescriptor>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PluginToolSearchQuery {
    pub(crate) query: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PluginToolMatch {
    pub(crate) plugin_id: String,
    pub(crate) tool_name: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PluginDetailResponse {
    pub(crate) manifest: PluginManifest,
    pub(crate) lifecycle: PluginLifecycleSnapshot,
    pub(crate) metrics: PluginMetrics,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct PluginLogListQuery {
    pub(crate) limit: Option<usize>,
}

impl PluginLogListQuery {
    pub(crate) fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct PluginEvalScoreRequest {
    pub(crate) input: serde_json::Value,
    pub(crate) expected: Option<serde_json::Value>,
    pub(crate) actual: serde_json::Value,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

pub(crate) async fn list_plugins_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let items = state.plugin_registry.list_all();
    (
        StatusCode::OK,
        Json(ListResponse {
            items,
            has_more: false,
        }),
    )
        .into_response()
}

pub(crate) async fn create_plugin_handler(
    State(state): State<Arc<AppState>>,
    Json(manifest): Json<PluginManifest>,
) -> impl IntoResponse {
    if let Err(err) = state.plugin_registry.register(manifest.clone()) {
        tracing::warn!("plugin register conflict: {err}");
        return AppApiError::new(StatusCode::CONFLICT, "conflict", err.to_string()).into_response();
    }

    let host_result = match state.plugin_host.lock() {
        Ok(mut host) => host.register(manifest.clone()),
        Err(_) => {
            let _ = state.plugin_registry.unregister(&manifest.id);
            tracing::error!("plugin host mutex poisoned during register");
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "plugin host unavailable",
            )
            .into_response();
        }
    };

    match host_result {
        Ok(()) => (StatusCode::CREATED, Json(manifest)).into_response(),
        Err(err) => {
            let _ = state.plugin_registry.unregister(&manifest.id);
            AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
                .into_response()
        }
    }
}

pub(crate) async fn get_plugin_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(manifest) = state.plugin_registry.get(&id) else {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found")
            .into_response();
    };

    let lifecycle = match state.plugin_host.lock() {
        Ok(host) => match host.lifecycle_snapshot(&id) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                return AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
                    .into_response();
            }
        },
        Err(_) => {
            tracing::error!("plugin host mutex poisoned during lifecycle_snapshot");
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "plugin host unavailable",
            )
            .into_response();
        }
    };

    let metrics = state.plugin_registry.metrics(&id).unwrap_or_default();
    (
        StatusCode::OK,
        Json(PluginDetailResponse {
            manifest,
            lifecycle,
            metrics,
        }),
    )
        .into_response()
}

pub(crate) async fn delete_plugin_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.plugin_registry.get(&id).is_none() {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found")
            .into_response();
    }

    if let Ok(mut host) = state.plugin_host.lock() {
        if host.state(&id).is_some() {
            let _ = host.shutdown(&id);
        }
    }

    match state.plugin_registry.unregister(&id) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn plugin_health_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.plugin_registry.get(&id).is_none() {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found")
            .into_response();
    }

    match state.plugin_host.lock() {
        Ok(mut host) => match host.health_check(&id) {
            Ok(response) => (StatusCode::OK, Json(response)).into_response(),
            Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
                .into_response(),
        },
        Err(_) => {
            tracing::error!("plugin host mutex poisoned during health_check");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "plugin host unavailable",
            )
            .into_response()
        }
    }
}

pub(crate) async fn plugin_metrics_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.plugin_registry.get(&id).is_none() {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found")
            .into_response();
    }
    (
        StatusCode::OK,
        Json(state.plugin_registry.metrics(&id).unwrap_or_default()),
    )
        .into_response()
}

pub(crate) async fn plugin_logs_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<PluginLogListQuery>,
) -> impl IntoResponse {
    match state.plugin_registry.list_logs(&id, query.limit()) {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse::<PluginLogEntry> {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(_) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found").into_response()
        }
    }
}

pub(crate) async fn plugin_pending_signals_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<PluginLogListQuery>,
) -> impl IntoResponse {
    match state
        .plugin_registry
        .list_pending_signals(&id, query.limit())
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse::<cairn_domain::SignalRecord> {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(_) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found").into_response()
        }
    }
}

pub(crate) async fn plugin_eval_score_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PluginEvalScoreRequest>,
) -> impl IntoResponse {
    let manifest = match state.plugin_registry.get(&id) {
        Some(m) => m,
        None => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found")
                .into_response();
        }
    };

    // Run the plugin synchronously (blocking on the Mutex since plugin I/O is synchronous).
    let result: Result<cairn_tools::EvalScoreResult, String> = tokio::task::spawn_blocking({
        let manifest = manifest.clone();
        let id = id.clone();
        let expected = body.expected.clone();
        let actual = body.actual.clone();
        let plugin_host = state.plugin_host.clone();

        move || -> Result<cairn_tools::EvalScoreResult, String> {
            let mut host = plugin_host.lock().map_err(|e| e.to_string())?;

            // Register and spawn the plugin if not already running.
            if host.state(&id).is_none() {
                host.register(manifest).map_err(|e| e.to_string())?;
            }

            if host.state(&id) == Some(PluginState::Discovered) {
                host.spawn(&id).map_err(|e| e.to_string())?;
            }

            if host.state(&id) == Some(PluginState::Spawning)
                || host.state(&id) == Some(PluginState::Handshaking)
            {
                host.handshake(&id).map_err(|e| e.to_string())?;
            }

            // Build the eval.score request.
            // target = { "actual": actual_output }
            // samples = [{ "expected": expected_output }]
            let target = serde_json::json!({ "actual": actual });
            let sample = serde_json::json!({ "expected": expected });
            let project =
                ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID);
            let req = build_eval_score_request("eval_1", "inv_1", &project, target, vec![sample]);

            let response = host.send_request(&id, &req).map_err(|e| e.to_string())?;

            // Shut down the plugin after the call.
            let _ = host.shutdown(&id);

            // Parse result.score and result.passed from the JSON-RPC result.
            let score = response
                .result
                .get("score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let passed = response
                .result
                .get("passed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let reasoning = response
                .result
                .get("feedback")
                .and_then(|v| v.as_str())
                .map(str::to_owned);

            Ok(cairn_tools::EvalScoreResult {
                score,
                passed,
                reasoning,
            })
        }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);

    match result {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "score": result.score,
                "passed": result.passed,
                "reasoning": result.reasoning,
            })),
        )
            .into_response(),
        Err(err) => AppApiError::new(
            StatusCode::BAD_REQUEST,
            "plugin_eval_failed",
            err.to_string(),
        )
        .into_response(),
    }
}

pub(crate) async fn plugin_capabilities_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let manifest = match state.plugin_registry.get(&id) {
        Some(m) => m,
        None => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found")
                .into_response()
        }
    };

    let verifications = match state.plugin_host.lock() {
        Ok(host) => match host.capability_verification(&id) {
            Ok(v) => v,
            Err(err) => {
                return AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
                    .into_response();
            }
        },
        Err(_) => {
            tracing::error!("plugin host mutex poisoned during capability_verification");
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "plugin host unavailable",
            )
            .into_response();
        }
    };

    // Build the response by pairing manifest capabilities with verification status.
    // The verifications list is positionally aligned with the manifest capabilities.
    let capabilities: Vec<serde_json::Value> = manifest
        .capabilities
        .iter()
        .enumerate()
        .map(|(i, cap)| {
            let verified = verifications.get(i).map(|v| v.verified).unwrap_or(false);
            serde_json::json!({
                "capability": cap,
                "verified": verified,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "plugin_id": id,
            "capabilities": capabilities,
        })),
    )
        .into_response()
}

pub(crate) async fn plugin_tools_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.plugin_registry.get(&id).is_none() {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "plugin not found")
            .into_response();
    }
    match state.plugin_host.lock() {
        Ok(host) => match host.get_tools(&id) {
            Ok(tools) => (
                StatusCode::OK,
                Json(PluginToolsResponse {
                    plugin_id: id,
                    tools,
                }),
            )
                .into_response(),
            Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
                .into_response(),
        },
        Err(_) => {
            tracing::error!("plugin host mutex poisoned during get_tools");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "plugin host unavailable",
            )
            .into_response()
        }
    }
}

pub(crate) async fn plugin_tools_search_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PluginToolSearchQuery>,
) -> impl IntoResponse {
    let q = query.query.as_deref().unwrap_or("").to_lowercase();
    let all_plugins = state.plugin_registry.list_all();
    let mut matches: Vec<PluginToolMatch> = Vec::new();
    if let Ok(host) = state.plugin_host.lock() {
        for manifest in &all_plugins {
            if let Ok(tools) = host.get_tools(&manifest.id) {
                for tool in tools {
                    if q.is_empty()
                        || tool.name.to_lowercase().contains(&q)
                        || tool.description.to_lowercase().contains(&q)
                    {
                        matches.push(PluginToolMatch {
                            plugin_id: manifest.id.clone(),
                            tool_name: tool.name,
                            description: tool.description,
                        });
                    }
                }
            }
        }
    }
    (StatusCode::OK, Json(matches)).into_response()
}

/// `DELETE /v1/plugins/:id`
/// Unregister a plugin -- shuts down its host process and removes it from the
/// registry. Identical to `delete_plugin_handler`; this is the semantic alias
/// used by the route catalog.
pub(crate) async fn unregister_plugin_handler(
    state: State<Arc<AppState>>,
    path: Path<String>,
) -> impl IntoResponse {
    delete_plugin_handler(state, path).await
}
