//! Admin handlers: DB status, snapshot/restore, projection rebuild,
//! event inspection, token rotation, and SQLite backup.

#[allow(unused_imports)]
use crate::*;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use cairn_api::auth::AuthPrincipal;
use cairn_store::pg::PgMigrationRunner;
use cairn_store::EventPosition;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── DB status handler ─────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct DbStatusResponse {
    /// `"postgres"` or `"in_memory"`.
    backend: &'static str,
    /// `true` when the Postgres pool is reachable.
    connected: bool,
    /// Number of migrations recorded in `_cairn_migrations`.
    /// `null` when using the in-memory backend.
    migration_count: Option<usize>,
    /// Whether the schema is fully up to date (all known migrations applied).
    schema_current: Option<bool>,
}

/// `GET /v1/db/status` — Postgres connection health + migration state.
///
/// Returns `backend = "in_memory"` when no Postgres URL was supplied.
/// When Postgres is configured, checks connectivity and reports the number
/// of applied migrations so operators can diagnose schema drift.
pub(crate) async fn db_status_handler(State(state): State<AppState>) -> Json<DbStatusResponse> {
    if let Some(pg) = &state.pg {
        let connected = pg.adapter.health_check().await.is_ok();
        let (migration_count, schema_current) = if connected {
            let pool = pg.adapter.pool().clone();
            let runner = PgMigrationRunner::new(pool);
            match runner.applied().await {
                Ok(applied) => {
                    const TOTAL_KNOWN: usize = 20;
                    let count = applied.len();
                    (Some(count), Some(count >= TOTAL_KNOWN))
                }
                Err(_) => (None, Some(false)),
            }
        } else {
            (None, Some(false))
        };
        Json(DbStatusResponse {
            backend: "postgres",
            connected,
            migration_count,
            schema_current,
        })
    } else if let Some(sq) = &state.sqlite {
        let connected = sq.adapter.health_check().await.is_ok();
        Json(DbStatusResponse {
            backend: "sqlite",
            connected,
            migration_count: None, // SQLite uses single-shot migrate(), no versioned log
            schema_current: Some(connected),
        })
    } else {
        Json(DbStatusResponse {
            backend: "in_memory",
            connected: true,
            migration_count: None,
            schema_current: None,
        })
    }
}

// ── Admin snapshot / restore ──────────────────────────────────────────────────

/// `POST /v1/admin/rotate-token` — rotate the admin bearer token at runtime.
///
/// Requires the current admin token in the Authorization header.
/// Body: `{ "new_token": "..." }` (min 16 chars).
///
/// The token registry is shared (same Arc) between the main.rs and lib.rs
/// routers, so a single revoke+register updates both.
pub(crate) async fn rotate_token_handler(
    // T6c-C5: admin-only; AdminRoleGuard fails closed with 403.
    _admin: cairn_app::extractors::AdminRoleGuard,
    State(state): State<AppState>,
    Json(body): Json<RotateTokenRequest>,
) -> impl IntoResponse {
    let new_token = body.new_token.trim().to_owned();
    if new_token.len() < 16 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "new_token must be at least 16 characters"})),
        );
    }

    // Find and revoke the old admin token, then register the new one.
    let entries = state.tokens.all_entries();
    for (old_token, principal) in &entries {
        if let AuthPrincipal::ServiceAccount { name, .. } = principal {
            if name == "admin" {
                state.tokens.revoke(old_token);
                state.tokens.register(new_token, principal.clone());
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "rotated"})),
                );
            }
        }
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "admin token not found in registry"})),
    )
}

#[derive(Deserialize)]
pub(crate) struct RotateTokenRequest {
    new_token: String,
}

pub(crate) async fn admin_snapshot_handler(
    // T6c-C5: snapshot exposes the full event log — admin only.
    _admin: cairn_app::extractors::AdminRoleGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let snap = state.runtime.store.dump_events();
    let json = match serde_json::to_vec_pretty(&snap) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Response::builder()
                    .status(500)
                    .body(axum::body::Body::from(e.to_string()))
                    .unwrap(),
            )
                .into_response();
        }
    };
    let filename = format!(
        "cairn-snapshot-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "application/json; charset=utf-8")
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        )
        .header("X-Event-Count", snap.event_count.to_string())
        .body(axum::body::Body::from(json))
        .unwrap()
        .into_response()
}

pub(crate) async fn backup_handler(
    // T6c-C5: backup copies the DB file — admin only.
    _admin: cairn_app::extractors::AdminRoleGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let Some(sqlite) = state.sqlite.as_ref() else {
        return not_found("SQLite backup is only available when the SQLite backend is active")
            .into_response();
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let backup_path = PathBuf::from(format!("{}.backup-{timestamp}", sqlite.path.display()));

    let size_bytes = match tokio::fs::copy(&sqlite.path, &backup_path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return internal_error(format!(
                "failed to back up SQLite database {}: {error}",
                sqlite.path.display()
            ))
            .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "backed_up",
            "path": backup_path.to_string_lossy(),
            "size_bytes": size_bytes,
        })),
    )
        .into_response()
}

/// `POST /v1/admin/restore` — restore from a snapshot uploaded as a JSON body.
///
/// Clears all in-memory state and replays the uploaded event log. Returns the
/// count of replayed events. This is irreversible — take a snapshot first.
pub(crate) async fn admin_restore_handler(
    // T6c-C5: restore replaces the event log — admin only.
    _admin: cairn_app::extractors::AdminRoleGuard,
    State(state): State<AppState>,
    axum::Json(snap): axum::Json<cairn_store::snapshot::StoreSnapshot>,
) -> impl IntoResponse {
    let event_count = snap.event_count;
    let replayed = state.runtime.store.load_snapshot(snap);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok":           true,
            "event_count":  event_count,
            "replayed":     replayed,
        })),
    )
}

// ── Projection rebuild + event inspection handlers ────────────────────────────

/// `POST /v1/admin/rebuild-projections` — replay the full event log through
/// all in-memory projections, rebuilding every read model from scratch.
///
/// This is the primary operational recovery tool: if a projection diverges
/// from the event log (e.g. after a bug fix), call this endpoint to restore
/// consistency without losing events.
///
/// Internally the handler performs a snapshot → restore cycle: it exports the
/// current event log and immediately replays it, which exercises
/// `apply_projection` on every stored event in order.
///
/// Returns: `{ events_replayed: N, duration_ms: N }`
pub(crate) async fn rebuild_projections_handler(
    // T6c-C5: projection rebuild touches every read model — admin only.
    _admin: cairn_app::extractors::AdminRoleGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let t0 = std::time::Instant::now();
    let snap = state.runtime.store.dump_events();
    let events_replayed = state.runtime.store.load_snapshot(snap);
    let duration_ms = t0.elapsed().as_millis() as u64;

    tracing::info!(events_replayed, duration_ms, "projections rebuilt");

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "events_replayed": events_replayed,
            "duration_ms":     duration_ms,
        })),
    )
}

/// `GET /v1/admin/event-count` — total event count and a per-type breakdown.
///
/// Returns: `{ total: N, by_type: { "session_created": 5, ... } }`
///
/// Useful for a quick health check on event log cardinality and for spotting
/// unexpected event type distributions.
pub(crate) async fn event_count_handler(
    // T6c-C5: cross-tenant aggregate count — admin only.
    _admin: cairn_app::extractors::AdminRoleGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let events = match state.runtime.store.read_stream(None, usize::MAX).await {
        Ok(v) => v,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let total = events.len() as u64;
    let mut by_type: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for ev in &events {
        *by_type
            .entry(event_type_name(&ev.envelope.payload).to_owned())
            .or_insert(0) += 1;
    }

    // Sort the breakdown for deterministic output.
    let mut sorted: Vec<(String, u64)> = by_type.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let by_type_obj: serde_json::Map<String, serde_json::Value> = sorted
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::Number(v.into())))
        .collect();

    Ok(Json(serde_json::json!({
        "total":   total,
        "by_type": serde_json::Value::Object(by_type_obj),
    })))
}

/// `GET /v1/admin/event-log?from=0&limit=100` — paginated raw event access.
///
/// Returns events in ascending position order.  Each event includes its
/// position, stored_at timestamp, event type name, and the full payload.
///
/// Query params:
/// - `from`  — return events with position ≥ this value (default: 0 = all)
/// - `limit` — max events per page (default: 100, max: 500)
///
/// Returns: `{ events: [...], total: N, has_more: bool }`
#[derive(Deserialize)]
pub(crate) struct EventLogQuery {
    #[serde(default)]
    from: u64,
    #[serde(default = "default_event_log_limit")]
    limit: usize,
}

pub(crate) fn default_event_log_limit() -> usize {
    100
}

pub(crate) async fn admin_event_log_handler(
    // T6c-C5: raw event log reveals every tenant's payloads — admin only.
    _admin: cairn_app::extractors::AdminRoleGuard,
    State(state): State<AppState>,
    Query(q): Query<EventLogQuery>,
) -> impl IntoResponse {
    let limit = q.limit.min(500);
    let after = if q.from > 0 {
        Some(EventPosition(q.from - 1))
    } else {
        None
    };

    let events = match state.runtime.store.read_stream(after, limit + 1).await {
        Ok(v) => v,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let has_more = events.len() > limit;
    let page: Vec<serde_json::Value> = events
        .into_iter()
        .take(limit)
        .map(|e| {
            serde_json::json!({
                "position":   e.position.0,
                "stored_at":  e.stored_at,
                "event_type": event_type_name(&e.envelope.payload),
                "event_id":   e.envelope.event_id.as_str(),
                "payload":    e.envelope.payload,
            })
        })
        .collect();

    let total = page.len();
    Ok(Json(serde_json::json!({
        "events":   page,
        "total":    total,
        "has_more": has_more,
    })))
}
