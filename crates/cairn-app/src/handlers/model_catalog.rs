//! Public model catalog handlers.
//!
//! Exposes the bundled LiteLLM catalog (plus cairn's TOML overlay and any
//! runtime operator overrides) via `GET /v1/models/catalog`. Unlike the
//! admin CRUD endpoints at `/v1/admin/models`, this is a read-only, filter-
//! and-paginate surface callable by any authenticated operator — it's what
//! the UI "pick a model" dropdowns and cost calculator read from.
//!
//! Endpoints:
//!
//! - `GET /v1/models/catalog` — filtered / paginated list.
//! - `GET /v1/models/catalog/providers` — unique provider families with counts.
//!
//! The catalog is boot-time static (LiteLLM JSON + TOML overlay are embedded
//! via `include_str!`). The providers-summary result is cached on
//! `AppState::model_catalog_providers_cache` — a `OnceLock` keyed on "the
//! first call wins". Runtime overrides via the admin CRUD API do NOT
//! invalidate the cache by design: overrides are rare + synchronized with
//! provider-connection edits, and the cost of a stale count is cosmetic
//! (the main `/catalog` list always reflects the live registry). Hot-reload
//! of the catalog from upstream is tracked as a follow-up.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use cairn_domain::model_catalog::{ModelEntry, ModelTier};
use serde::{Deserialize, Serialize};

use crate::errors::AppApiError;
use crate::state::AppState;

/// Default page size. Chosen to render the full bundled catalog (≈500 entries
/// after chat-mode filter) inside a couple of pages without forcing the UI
/// to paginate aggressively.
const DEFAULT_LIMIT: usize = 100;
/// Hard ceiling on `limit`. Prevents an unbounded client from streaming the
/// whole registry in one call; the UI paginates, and scripted callers can
/// walk pages just as easily as pulling 5 000 entries in one shot.
const MAX_LIMIT: usize = 1000;

/// Query parameters for `GET /v1/models/catalog`.
///
/// All fields optional. Boolean filters are tri-state: absent means "don't
/// filter on this axis". Query-string booleans accept `true`/`false`/`1`/`0`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ListModelCatalogQuery {
    /// Exact-match provider family, e.g. `openai`, `anthropic`, `openrouter`.
    pub provider: Option<String>,
    /// Exact-match routing tier: `brain`, `mid`, or `light`.
    pub tier: Option<String>,
    /// Case-insensitive substring across `id`, `display_name`, and `provider`.
    pub search: Option<String>,
    /// Only entries whose `supports_tools` matches this value.
    pub supports_tools: Option<bool>,
    /// Only entries whose `supports_json_mode` matches this value.
    pub supports_json_mode: Option<bool>,
    /// Only entries whose `reasoning` flag matches this value.
    pub reasoning: Option<bool>,
    /// Cost ceiling on `cost_per_1m_input`. Entries with a higher input
    /// cost are excluded. Applies to metered models only (free models have
    /// cost 0.0 and always pass).
    pub max_cost_per_1m: Option<f64>,
    /// Shortcut filter: input+output cost both zero.
    pub free_only: Option<bool>,
    /// Page size (default 100, capped at 1000).
    pub limit: Option<usize>,
    /// Zero-based offset into the sorted result set.
    pub offset: Option<usize>,
}

/// Response body for `GET /v1/models/catalog`.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ListModelCatalogResponse {
    pub items: Vec<ModelEntry>,
    /// Total count after filters, BEFORE pagination. Lets the UI render
    /// "showing 100 of 387".
    pub total: usize,
    /// True when `offset + items.len() < total`, i.e. another page exists.
    #[serde(rename = "hasMore")]
    pub has_more: bool,
}

/// One row in the providers summary response.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderCount {
    pub name:  String,
    pub count: usize,
}

/// Response body for `GET /v1/models/catalog/providers`.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct CatalogProvidersResponse {
    pub providers: Vec<ProviderCount>,
}

fn parse_tier(raw: &str) -> Result<ModelTier, AppApiError> {
    match raw.to_ascii_lowercase().as_str() {
        "brain" => Ok(ModelTier::Brain),
        "mid" => Ok(ModelTier::Mid),
        "light" => Ok(ModelTier::Light),
        other => Err(AppApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            format!("invalid tier '{other}' (expected brain, mid, or light)"),
        )),
    }
}

/// `GET /v1/models/catalog` — filtered, sorted, paginated model list.
pub(crate) async fn list_model_catalog_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListModelCatalogQuery>,
) -> Result<Json<ListModelCatalogResponse>, AppApiError> {
    // Validate pagination bounds up-front — an oversized `limit` is a client
    // bug, not a server suggestion to clamp silently.
    let limit = match q.limit {
        None => DEFAULT_LIMIT,
        Some(0) => {
            return Err(AppApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                "limit must be >= 1",
            ));
        }
        Some(n) if n > MAX_LIMIT => {
            return Err(AppApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                format!("limit must be <= {MAX_LIMIT}"),
            ));
        }
        Some(n) => n,
    };
    let offset = q.offset.unwrap_or(0);

    let tier_filter = match q.tier.as_deref() {
        Some(t) => Some(parse_tier(t)?),
        None => None,
    };

    let all = state.model_registry.all();

    // Graceful fallback: a corrupt bundled JSON could leave us with an empty
    // registry. We signal that distinctly so the UI can render a clear
    // message instead of "no models match your filter".
    if all.is_empty() {
        return Err(AppApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "model_catalog_unavailable",
            "model catalog is empty — check server startup logs",
        ));
    }

    let search_lc = q.search.as_deref().map(str::to_ascii_lowercase);

    // Single-pass filter. Keeps the iteration on owned ModelEntry values
    // (ModelRegistry::all clones internally) so we don't re-take the read
    // lock once per predicate.
    let mut filtered: Vec<ModelEntry> = all
        .into_iter()
        .filter(|e| {
            if let Some(p) = &q.provider {
                if &e.provider != p {
                    return false;
                }
            }
            if let Some(tier) = tier_filter {
                if e.tier != tier {
                    return false;
                }
            }
            if let Some(needle) = search_lc.as_deref() {
                let hay_id = e.id.to_ascii_lowercase();
                let hay_name = e.display_name.to_ascii_lowercase();
                let hay_prov = e.provider.to_ascii_lowercase();
                if !hay_id.contains(needle)
                    && !hay_name.contains(needle)
                    && !hay_prov.contains(needle)
                {
                    return false;
                }
            }
            if let Some(st) = q.supports_tools {
                if e.supports_tools != st {
                    return false;
                }
            }
            if let Some(sj) = q.supports_json_mode {
                if e.supports_json_mode != sj {
                    return false;
                }
            }
            if let Some(r) = q.reasoning {
                if e.reasoning != r {
                    return false;
                }
            }
            if let Some(max) = q.max_cost_per_1m {
                if e.cost_per_1m_input > max {
                    return false;
                }
            }
            if q.free_only == Some(true)
                && (e.cost_per_1m_input != 0.0 || e.cost_per_1m_output != 0.0)
            {
                return false;
            }
            true
        })
        .collect();

    // Deterministic order: provider ASC, input cost ASC, id ASC. The id
    // tie-break makes the result stable across runs even when two entries
    // share a provider and cost (common for LiteLLM's regional Bedrock
    // duplicates).
    filtered.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| {
                a.cost_per_1m_input
                    .partial_cmp(&b.cost_per_1m_input)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.id.cmp(&b.id))
    });

    let total = filtered.len();
    let page: Vec<ModelEntry> = filtered.into_iter().skip(offset).take(limit).collect();
    let has_more = offset.saturating_add(page.len()) < total;

    Ok(Json(ListModelCatalogResponse {
        items: page,
        total,
        has_more,
    }))
}

/// `GET /v1/models/catalog/providers` — unique providers with entry counts.
///
/// Result sorted by name ASC for deterministic dropdown rendering.
pub(crate) async fn list_catalog_providers_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CatalogProvidersResponse>, AppApiError> {
    // Fast path: return the cached summary if present. The cache is built
    // from `ModelRegistry::all()` on first call; overrides registered via
    // the admin CRUD path are therefore not reflected here. See module doc.
    if let Some(providers) = state.model_catalog_providers_cache.get() {
        return Ok(Json(CatalogProvidersResponse {
            providers: providers.clone(),
        }));
    }

    let entries = state.model_registry.all();
    if entries.is_empty() {
        return Err(AppApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "model_catalog_unavailable",
            "model catalog is empty — check server startup logs",
        ));
    }

    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for e in entries {
        *counts.entry(e.provider).or_insert(0) += 1;
    }
    let providers: Vec<ProviderCount> = counts
        .into_iter()
        .map(|(name, count)| ProviderCount { name, count })
        .collect();

    // Ignore the `Result`: another request may have populated the cache
    // between our `get()` and `set()`. Either snapshot is correct.
    let _ = state
        .model_catalog_providers_cache
        .set(providers.clone());

    Ok(Json(CatalogProvidersResponse { providers }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tier_accepts_canonical_forms() {
        assert_eq!(parse_tier("brain").unwrap(), ModelTier::Brain);
        assert_eq!(parse_tier("Mid").unwrap(), ModelTier::Mid);
        assert_eq!(parse_tier("LIGHT").unwrap(), ModelTier::Light);
    }

    #[test]
    fn parse_tier_rejects_unknown() {
        let err = parse_tier("wizard").unwrap_err();
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
