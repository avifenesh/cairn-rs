//! Skills catalog handlers.
//!
//! Wires the real skills registry (cairn-api/skills_api + cairn-domain/skills)
//! to the HTTP surface. Replaces the previous hard-coded empty stub at
//! `handlers/memory.rs::list_skills_preserved_handler`.
//!
//! Routes:
//! - `GET  /v1/skills`      — list skills with summary counts (UI shape).
//! - `GET  /v1/skills/:id`  — single skill detail.
//!
//! Invocation (`POST /v1/skills/:id/invoke`) is deliberately NOT wired: the
//! domain `SkillCatalog` does not ship an executor. Invocation will be added
//! once the skill-execution service lands; see issue #147.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use cairn_api::skills_api::SkillSummary;
use cairn_domain::skills::{Skill, SkillStatus};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Query parameters for `GET /v1/skills`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ListSkillsQuery {
    /// Optional comma-separated tag filter. Only skills tagged with ALL of
    /// the listed tags are returned.
    pub tag: Option<String>,
}

/// Summary counts rendered by `SkillsPage.tsx` stat cards.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SkillsSummary {
    pub total: usize,
    pub enabled: usize,
    pub disabled: usize,
}

/// Response body for `GET /v1/skills`.
///
/// Shape matches `ui/src/lib/types.ts::SkillsResponse`.
///
/// `currently_active` is also emitted as `currentlyActive` for byte-for-byte
/// backwards compatibility with the previous stub, which wrote both field
/// names into the response (the UI reads either). Dropping the camelCase
/// duplicate would silently break any out-of-tree client that still keys on
/// it, so we keep both.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ListSkillsResponse {
    pub items: Vec<SkillSummary>,
    pub summary: SkillsSummary,
    /// IDs of skills currently reported as Active by the domain catalog.
    pub currently_active: Vec<String>,
    /// Legacy camelCase alias for `currently_active`. Serialized verbatim
    /// from the same list so the two fields can never drift.
    #[serde(rename = "currentlyActive")]
    pub currently_active_camel: Vec<String>,
}

pub(crate) async fn list_skills_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListSkillsQuery>,
) -> impl IntoResponse {
    let tags_owned: Vec<String> = query
        .tag
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let tag_refs: Vec<&str> = tags_owned.iter().map(String::as_str).collect();

    let catalog = state.skill_catalog.read().await;
    let all: Vec<&Skill> = catalog.list(&tag_refs);

    // Single pass over the catalog: build summaries, count enabled, and
    // collect currently-active IDs in one scan. Avoids three separate
    // iterations over `all`.
    let mut items: Vec<SkillSummary> = Vec::with_capacity(all.len());
    let mut enabled = 0usize;
    let mut currently_active: Vec<String> = Vec::new();
    for skill in &all {
        if skill.enabled {
            enabled += 1;
        }
        if matches!(skill.status, SkillStatus::Active) {
            currently_active.push(skill.skill_id.clone());
        }
        items.push(SkillSummary::from(*skill));
    }
    let total = items.len();
    let disabled = total.saturating_sub(enabled);

    let body = ListSkillsResponse {
        items,
        summary: SkillsSummary {
            total,
            enabled,
            disabled,
        },
        currently_active_camel: currently_active.clone(),
        currently_active,
    };
    (StatusCode::OK, Json(body)).into_response()
}

pub(crate) async fn get_skill_handler(
    State(state): State<Arc<AppState>>,
    Path(skill_id): Path<String>,
) -> impl IntoResponse {
    let catalog = state.skill_catalog.read().await;
    match catalog.get(&skill_id) {
        Some(skill) => (StatusCode::OK, Json(skill.clone())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "skill_not_found",
                "skill_id": skill_id,
            })),
        )
            .into_response(),
    }
}
