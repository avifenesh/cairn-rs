//! Integration CRUD and dynamic webhook handlers.
//!
//! Extracted from `lib.rs` — contains register, list, get, delete
//! integration endpoints plus override management and dynamic webhook receiver.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::errors::AppApiError;
use crate::state::AppState;

// ── Handlers ────────────────────────────────────────────────────────────────

/// POST /v1/integrations — register a new integration at runtime.
pub(crate) async fn register_integration_handler(
    State(state): State<Arc<AppState>>,
    Json(config): Json<cairn_integrations::IntegrationConfig>,
) -> impl IntoResponse {
    match state.integrations.register_from_config(config).await {
        Ok(()) => {
            let statuses = state.integrations.all_statuses().await;
            Json(serde_json::json!({
                "status": "registered",
                "integrations": statuses,
            }))
            .into_response()
        }
        Err(e) => AppApiError::new(
            StatusCode::BAD_REQUEST,
            "integration_registration_failed",
            e.to_string(),
        )
        .into_response(),
    }
}

/// GET /v1/integrations — list all registered integrations + status.
pub(crate) async fn list_integrations_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let statuses = state.integrations.all_statuses().await;
    Json(serde_json::json!({ "integrations": statuses })).into_response()
}

/// GET /v1/integrations/:integration_id — get integration detail + config.
pub(crate) async fn get_integration_handler(
    State(state): State<Arc<AppState>>,
    Path(integration_id): Path<String>,
) -> impl IntoResponse {
    let integration = match state.integrations.get(&integration_id).await {
        Some(i) => i,
        None => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "integration_not_found",
                format!("no integration with id '{integration_id}'"),
            )
            .into_response();
        }
    };
    let config = state.integrations.get_config(&integration_id).await;
    let overrides = state.integrations.get_overrides(&integration_id).await;
    let stats = integration.queue_stats().await;
    Json(serde_json::json!({
        "id": integration.id(),
        "display_name": integration.display_name(),
        "configured": integration.is_configured(),
        "config": config,
        "overrides": overrides,
        "queue_stats": stats,
        "default_agent_prompt": integration.default_agent_prompt(),
        "auth_exempt_paths": integration.auth_exempt_paths(),
    }))
    .into_response()
}

/// DELETE /v1/integrations/:integration_id — remove an integration.
pub(crate) async fn delete_integration_handler(
    State(state): State<Arc<AppState>>,
    Path(integration_id): Path<String>,
) -> impl IntoResponse {
    match state.integrations.unregister(&integration_id).await {
        Ok(()) => Json(serde_json::json!({
            "status": "removed",
            "id": integration_id,
        }))
        .into_response(),
        Err(e) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "integration_not_found",
            e.to_string(),
        )
        .into_response(),
    }
}

/// GET /v1/integrations/:integration_id/overrides
pub(crate) async fn get_integration_overrides_handler(
    State(state): State<Arc<AppState>>,
    Path(integration_id): Path<String>,
) -> impl IntoResponse {
    let overrides = state.integrations.get_overrides(&integration_id).await;
    Json(serde_json::json!({ "overrides": overrides })).into_response()
}

/// PUT /v1/integrations/:integration_id/overrides — set operator overrides.
pub(crate) async fn set_integration_overrides_handler(
    State(state): State<Arc<AppState>>,
    Path(integration_id): Path<String>,
    Json(overrides): Json<cairn_integrations::IntegrationOverrides>,
) -> impl IntoResponse {
    if state.integrations.get(&integration_id).await.is_none() {
        return AppApiError::new(
            StatusCode::NOT_FOUND,
            "integration_not_found",
            format!("no integration with id '{integration_id}'"),
        )
        .into_response();
    }
    state
        .integrations
        .set_overrides(&integration_id, overrides)
        .await;
    Json(serde_json::json!({
        "status": "overrides_set",
        "id": integration_id,
    }))
    .into_response()
}

/// DELETE /v1/integrations/:integration_id/overrides — reset to defaults.
pub(crate) async fn clear_integration_overrides_handler(
    State(state): State<Arc<AppState>>,
    Path(integration_id): Path<String>,
) -> impl IntoResponse {
    state.integrations.clear_overrides(&integration_id).await;
    Json(serde_json::json!({
        "status": "overrides_cleared",
        "id": integration_id,
    }))
    .into_response()
}

/// POST /v1/webhooks/:integration_id — dynamic webhook receiver.
///
/// Looks up the integration in the registry, delegates verification and
/// event parsing to the plugin, then dispatches based on event-to-action mappings.
pub(crate) async fn dynamic_webhook_handler(
    State(state): State<Arc<AppState>>,
    Path(integration_id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let integration = match state.integrations.get(&integration_id).await {
        Some(i) => i,
        None => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "integration_not_found",
                format!("no integration with id '{integration_id}'"),
            )
            .into_response();
        }
    };

    // Verify the webhook signature.
    if let Err(e) = integration.verify_webhook(&headers, &body).await {
        return AppApiError::new(
            StatusCode::UNAUTHORIZED,
            "webhook_verification_failed",
            e.to_string(),
        )
        .into_response();
    }

    // Parse the event.
    let event = match integration.parse_event(&headers, &body).await {
        Ok(e) => e,
        Err(e) => {
            return AppApiError::new(StatusCode::BAD_REQUEST, "event_parse_failed", e.to_string())
                .into_response();
        }
    };

    // Match against event-to-action mappings (use overrides if set, else defaults).
    let actions = state
        .integrations
        .effective_event_actions(&integration_id)
        .await;
    let matched = actions.iter().find(|a| {
        cairn_integrations::github::GitHubPlugin::event_matches(&event.event_key, &a.event_pattern)
    });

    match matched.map(|a| &a.action) {
        Some(cairn_integrations::EventAction::Ignore) | None => Json(serde_json::json!({
            "status": "ignored",
            "event": event.event_key,
            "integration": integration_id,
        }))
        .into_response(),
        Some(cairn_integrations::EventAction::Acknowledge) => {
            tracing::info!(
                integration = %integration_id,
                event = %event.event_key,
                "Webhook acknowledged"
            );
            Json(serde_json::json!({
                "status": "acknowledged",
                "event": event.event_key,
                "integration": integration_id,
            }))
            .into_response()
        }
        Some(cairn_integrations::EventAction::CreateAndOrchestrate) => {
            tracing::info!(
                integration = %integration_id,
                event = %event.event_key,
                title = ?event.title,
                "Webhook -> create and orchestrate"
            );
            Json(serde_json::json!({
                "status": "queued",
                "event": event.event_key,
                "integration": integration_id,
                "title": event.title,
            }))
            .into_response()
        }
    }
}
