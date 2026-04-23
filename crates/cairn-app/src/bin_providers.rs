//! Provider handlers: Ollama models, generate, embed, chat/stream,
//! provider connection discovery/test, model pull/delete/info.

#[allow(unused_imports)]
use crate::*;

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
#[allow(unused_imports)]
use cairn_runtime::{OllamaEmbeddingProvider, OllamaModel};

/// Optional `?tenant_id=` scope on provider playground routes (generate /
/// embed / chat-stream), honoured **only for admin callers**.
///
/// Pre-#157, these handlers hardcoded `TenantId::new("default_tenant")`,
/// which silently served the wrong tenant's providers to any operator
/// not running on the default scope. The fix is to use the tenant the
/// auth middleware already attached via `TenantScope`. An admin service
/// account may override it with `?tenant_id=` for cross-tenant tooling;
/// a non-admin caller supplying the param gets a 403 so the handler
/// cannot be used to exfiltrate another tenant's registered providers.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct PlaygroundScopeQuery {
    pub tenant_id: Option<String>,
    #[allow(dead_code)]
    pub workspace_id: Option<String>,
    #[allow(dead_code)]
    pub project_id: Option<String>,
}

impl PlaygroundScopeQuery {
    /// Resolve the effective tenant for a playground call.
    ///
    /// Rules (implemented by the match below):
    /// - Operator caller, no `?tenant_id=` → authenticated tenant.
    /// - Operator caller, `?tenant_id=T` matching the authenticated
    ///   tenant → authenticated tenant (no-op override).
    /// - Operator caller, `?tenant_id=T` mismatched → **403**. Non-admin
    ///   callers can never read another tenant's providers.
    /// - Admin caller, `?tenant_id=T` → `T` (cross-tenant tooling).
    /// - Admin caller, no `?tenant_id=`, with an attached tenant
    ///   (ServiceAccount has `tenant` set) → that tenant.
    /// - Admin caller, no attached tenant (`AuthPrincipal::System`) and
    ///   no override → `DEFAULT_TENANT_ID`.
    #[allow(clippy::result_large_err)] // axum Response is the natural error shape
    pub(crate) fn resolve_tenant(
        &self,
        authenticated: Option<&cairn_domain::TenantId>,
        is_admin: bool,
    ) -> Result<cairn_domain::TenantId, axum::response::Response> {
        let requested = self
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        match (authenticated, is_admin, requested) {
            // Operator with no override — use their tenant.
            (Some(auth), false, None) => Ok(auth.clone()),
            // Operator with matching override — fine.
            (Some(auth), false, Some(req)) if req == auth.as_str() => Ok(auth.clone()),
            // Operator attempting cross-tenant access — refuse.
            (Some(auth), false, Some(req)) => Err((
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({
                    "error": "cross-tenant playground access requires an admin token",
                    "authenticated_tenant": auth.as_str(),
                    "requested_tenant":     req,
                })),
            )
                .into_response()),
            // Admin (or no attached tenant) with explicit override.
            (_, _, Some(req)) => Ok(cairn_domain::TenantId::new(req)),
            // Admin (or no attached tenant) with no override: default.
            (Some(auth), true, None) => Ok(auth.clone()),
            (None, _, None) => Ok(cairn_domain::TenantId::new(
                cairn_app::state::DEFAULT_TENANT_ID,
            )),
        }
    }
}

/// Derive the authenticated tenant for a playground call.
///
/// Single source of truth: the tenant baked into the `AuthPrincipal`
/// itself (Operator / ServiceAccount carry one; `System` does not).
/// Falls back to the request-extension tenant the auth middleware
/// attaches only when the principal lacks one — kept as a safety net
/// in case the middleware contract ever changes, so the handler
/// doesn't silently start returning the wrong tenant. After resolving
/// the authenticated tenant, dispatch to `PlaygroundScopeQuery::resolve_tenant`
/// to apply admin override / non-admin 403 rules.
#[allow(clippy::result_large_err)]
fn resolve_playground_tenant(
    scope: &PlaygroundScopeQuery,
    principal: &cairn_api::auth::AuthPrincipal,
    auth_tenant: Option<&axum::Extension<cairn_domain::TenantId>>,
    is_admin: bool,
) -> Result<cairn_domain::TenantId, axum::response::Response> {
    let principal_tenant = principal.tenant().map(|t| t.tenant_id.clone());
    let ext_tenant = auth_tenant.map(|axum::Extension(t)| t.clone());
    let authenticated = principal_tenant.or(ext_tenant);
    scope.resolve_tenant(authenticated.as_ref(), is_admin)
}

// ── Ollama handler ────────────────────────────────────────────────────────────

/// `GET /v1/providers/ollama/models` — list models available in the local Ollama registry.
///
/// Returns `200` with a JSON array of model names when Ollama is configured and
/// reachable, `503` when Ollama is not wired (OLLAMA_HOST unset), and `502`
/// when the daemon cannot be reached at call time.
pub(crate) async fn ollama_models_handler(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(provider) = &state.ollama {
        match provider.list_models().await {
            Ok(models) => {
                let names: Vec<&str> = models
                    .iter()
                    .map(|m: &OllamaModel| m.name.as_str())
                    .collect();
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "host":   provider.host(),
                        "models": names,
                        "count":  names.len(),
                    })),
                )
                    .into_response()
            }
            Err(e) => (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "error": format!("Ollama unreachable: {e}")
                })),
            )
                .into_response(),
        }
    } else {
        // No Ollama configured — return 503 so callers know to use provider connections instead.
        (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": "Ollama not configured — set OLLAMA_HOST to enable local model management"
            })),
        )
            .into_response()
    }
}

// ── Provider connection discovery ─────────────────────────────────────────────

/// Unified model record returned by `discover-models`.
#[derive(serde::Serialize, Clone, Debug)]
pub(crate) struct DiscoveredModel {
    model_id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameter_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantization: Option<String>,
    /// Inferred capabilities: "generate", "embed", "rerank".
    capabilities: Vec<String>,
    /// Maximum context window in tokens, if known (from /v1/models or /api/show).
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window_tokens: Option<u32>,
}

/// Query params shared by `discover-models` and `test`.
#[derive(serde::Deserialize, Default)]
pub(crate) struct DiscoverModelsQuery {
    /// Override endpoint URL (for ad-hoc discovery before connection is registered).
    endpoint_url: Option<String>,
    /// API key to use with `endpoint_url`.
    api_key: Option<String>,
    /// Override adapter type: "ollama" | "openai_compat" (inferred from connection if absent).
    adapter_type: Option<String>,
}

pub(crate) fn decrypt_provider_credential(
    record: &cairn_domain::credentials::CredentialRecord,
) -> Result<String, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    use sha2::{Digest, Sha256};

    let seed = record.key_id.as_deref().unwrap_or("cairn-local-test-key");
    let digest = Sha256::digest(seed.as_bytes());
    let mut key_material = [0u8; 32];
    key_material.copy_from_slice(&digest[..32]);

    let key = Key::<Aes256Gcm>::from_slice(&key_material);
    let cipher = Aes256Gcm::new(key);

    let encrypted_at_ms = record
        .encrypted_at_ms
        .ok_or_else(|| "credential missing encrypted_at_ms".to_owned())?;
    let nonce_digest = Sha256::digest(
        format!(
            "{}:{}:{encrypted_at_ms}",
            record.tenant_id.as_str(),
            record.provider_id
        )
        .as_bytes(),
    );
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&nonce_digest[..12]);

    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, record.encrypted_value.as_ref())
        .map_err(|e| format!("credential decryption failed: {e}"))?;
    String::from_utf8(plaintext).map_err(|e| format!("credential plaintext invalid utf-8: {e}"))
}

pub(crate) async fn resolve_connection_probe_material(
    state: &AppState,
    connection_id: &str,
) -> (Option<String>, Option<String>) {
    let system_project = cairn_domain::ProjectKey::new("system", "system", "system");

    let endpoint_key = format!("provider_endpoint_{connection_id}");
    let endpoint_url = match state
        .runtime
        .defaults
        .resolve(&system_project, &endpoint_key)
        .await
    {
        Ok(Some(setting)) => setting
            .as_str()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        _ => None,
    };

    let credential_key = format!("provider_credential_{connection_id}");
    let credential_id = match state
        .runtime
        .defaults
        .resolve(&system_project, &credential_key)
        .await
    {
        Ok(Some(setting)) => setting.as_str().map(str::to_owned),
        _ => None,
    };

    let api_key = match credential_id {
        Some(credential_id) => match state
            .runtime
            .credentials
            .get(&cairn_domain::CredentialId::new(credential_id))
            .await
        {
            Ok(Some(record)) if record.active => decrypt_provider_credential(&record).ok(),
            _ => None,
        },
        None => None,
    };

    // Resolve $ENV_VAR references — if the stored key starts with '$', treat it as an env var name.
    let api_key = api_key.map(|key| {
        if let Some(var_name) = key.strip_prefix('$') {
            std::env::var(var_name).unwrap_or(key)
        } else {
            key
        }
    });

    (endpoint_url, api_key)
}

/// `GET /v1/providers/connections/:id/discover-models`
///
/// Queries the live provider endpoint for available models.
///
/// - `ollama`        → `GET {host}/api/tags`
/// - `openai_compat` → `GET {base_url}/models`
///
/// Use `?endpoint_url=` for ad-hoc queries before registering the connection.
pub(crate) async fn discover_models_handler(
    State(state): State<AppState>,
    Path(connection_id): Path<String>,
    Query(query): Query<DiscoverModelsQuery>,
) -> impl IntoResponse {
    use cairn_domain::ProviderConnectionId;
    use cairn_store::projections::ProviderConnectionReadModel;

    let conn_id = ProviderConnectionId::new(connection_id.clone());
    let adapter_type =
        match ProviderConnectionReadModel::get(state.runtime.store.as_ref(), &conn_id).await {
            Ok(Some(rec)) => rec.adapter_type.to_lowercase(),
            Ok(None) => {
                if query.endpoint_url.is_none() {
                    return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({
                    "error": format!("provider connection '{connection_id}' not found"),
                    "hint": "pass ?endpoint_url=... to discover without a registered connection",
                }))).into_response();
                }
                query
                    .adapter_type
                    .clone()
                    .unwrap_or_else(|| "openai_compat".to_owned())
            }
            Err(e) => return internal_error(format!("store error: {e}")).into_response(),
        };
    // Allow query param to override stored adapter_type.
    let adapter_type = query
        .adapter_type
        .as_deref()
        .unwrap_or(&adapter_type)
        .to_lowercase();

    let (stored_endpoint, stored_api_key) =
        if query.endpoint_url.is_none() && query.api_key.is_none() {
            resolve_connection_probe_material(&state, &connection_id).await
        } else {
            (None, None)
        };

    if adapter_type == "ollama" {
        discover_ollama_models_live(
            &state,
            query.endpoint_url.as_deref().or(stored_endpoint.as_deref()),
        )
        .await
    } else {
        discover_openai_compat_models_live(
            &state,
            query.endpoint_url.as_deref().or(stored_endpoint.as_deref()),
            query.api_key.as_deref().or(stored_api_key.as_deref()),
        )
        .await
    }
}

pub(crate) async fn discover_ollama_models_live(
    state: &AppState,
    endpoint_override: Option<&str>,
) -> axum::response::Response {
    let host =
        match endpoint_override {
            Some(url) => url.trim_end_matches('/').to_owned(),
            None => match &state.ollama {
                Some(p) => p.host().to_owned(),
                None => return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": "Ollama not configured — set OLLAMA_HOST or pass ?endpoint_url="
                    })),
                )
                    .into_response(),
            },
        };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    match client.get(format!("{host}/api/tags")).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(body) => {
                    let names: Vec<String> = body
                        .get("models")
                        .and_then(|m| m.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m.get("name")?.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();

                    // Call /api/show for each model to get num_ctx.
                    // Best-effort: silently ignore failures for individual models.
                    let mut models: Vec<DiscoveredModel> = Vec::with_capacity(names.len());
                    for name in &names {
                        let ctx = fetch_ollama_context_window(&client, &host, name).await;
                        models.push(ollama_name_to_discovered_with_ctx(name, ctx));
                    }

                    (
                        StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "provider": "ollama",
                            "endpoint": host,
                            "models":   models,
                        })),
                    )
                        .into_response()
                }
                Err(e) => internal_error(format!("parse error: {e}")).into_response(),
            }
        }
        Ok(resp) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Ollama returned HTTP {}", resp.status()),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Ollama unreachable: {e}"),
            })),
        )
            .into_response(),
    }
}

/// Call Ollama's `POST /api/show` for a single model and extract `num_ctx`.
///
/// Returns `None` on any error (network, parse, field absent) so discovery
/// can fall back to `known_context_window`.
pub(crate) async fn fetch_ollama_context_window(
    client: &reqwest::Client,
    host: &str,
    model_name: &str,
) -> Option<u32> {
    let resp = client
        .post(format!("{host}/api/show"))
        .json(&serde_json::json!({ "name": model_name }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    // Ollama /api/show nests context length as model_info.*.context_length
    // or directly as model_info.llama.context_length.
    // Try a few paths before giving up.
    if let Some(n) = body
        .pointer("/model_info/llama.context_length")
        .or_else(|| body.pointer("/model_info/context_length"))
    {
        return n.as_u64().map(|v| v as u32);
    }
    // Older Ollama versions expose it under /parameters/num_ctx.
    if let Some(n) = body.pointer("/parameters/num_ctx") {
        return n.as_u64().map(|v| v as u32);
    }
    // Some versions put it under /details/parameter_size or model_info flatly.
    body.get("model_info")
        .and_then(|mi| mi.as_object())
        .and_then(|obj| obj.values().find_map(|v| v.as_u64().map(|n| n as u32)))
        .filter(|&n| n >= 512) // sanity: must be a plausible context size
}

pub(crate) async fn discover_openai_compat_models_live(
    state: &AppState,
    endpoint_override: Option<&str>,
    api_key_override: Option<&str>,
) -> axum::response::Response {
    let (base_url, api_key) = match endpoint_override {
        Some(url) => (url.trim_end_matches('/').to_owned(), api_key_override.map(str::to_owned).unwrap_or_default()),
        None => match &state.openai_compat {
            Some(p) => (p.base_url.as_str().trim_end_matches('/').to_owned(), std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default()),
            None => return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                "error": "OpenAI-compat not configured — set OPENAI_COMPAT_BASE_URL or pass ?endpoint_url="
            }))).into_response(),
        },
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let mut req = client.get(format!("{base_url}/models"));
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(body) => {
                    // Each item in `data` is a full model object — pass the
                    // whole object so we can extract context_length / max_model_len.
                    let models: Vec<DiscoveredModel> = body
                        .get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(openai_model_obj_to_discovered)
                                .collect()
                        })
                        .unwrap_or_default();
                    (
                        StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "provider": "openai_compat",
                            "endpoint": base_url,
                            "models":   models,
                        })),
                    )
                        .into_response()
                }
                Err(e) => internal_error(format!("parse error: {e}")).into_response(),
            }
        }
        Ok(resp) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Provider returned HTTP {}", resp.status()),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Provider unreachable: {e}"),
            })),
        )
            .into_response(),
    }
}

pub(crate) fn ollama_name_to_discovered_with_ctx(
    name: &str,
    ctx_window: Option<u32>,
) -> DiscoveredModel {
    let (base, tag) = name.split_once(':').unwrap_or((name, ""));
    let mut parts = tag.split('-');
    let param_size = parts
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let quantization = parts
        .filter(|s| s.to_lowercase().starts_with('q') || s.contains('_'))
        .max_by_key(|s| s.len())
        .map(str::to_owned);
    let lower = base.to_lowercase();
    let capabilities =
        if lower.contains("embed") || lower.contains("nomic") || lower.contains("all-minilm") {
            vec!["embed".to_owned()]
        } else if lower.contains("rerank") {
            vec!["rerank".to_owned()]
        } else {
            vec!["generate".to_owned()]
        };
    // Fall back to well-known defaults when the provider didn't report context length.
    let ctx = ctx_window.or_else(|| known_context_window(name));
    DiscoveredModel {
        model_id: name.to_owned(),
        name: name.to_owned(),
        parameter_size: param_size,
        quantization,
        capabilities,
        context_window_tokens: ctx,
    }
}

/// Convert a full OpenAI-compat model JSON object (from /v1/models `data` array)
/// into a `DiscoveredModel`, extracting the context window if present.
pub(crate) fn openai_model_obj_to_discovered(obj: &serde_json::Value) -> Option<DiscoveredModel> {
    let id = obj.get("id")?.as_str()?;
    let lower = id.to_lowercase();
    let capabilities = if lower.contains("embed") || lower.contains("embedding") {
        vec!["embed".to_owned()]
    } else if lower.contains("rerank") {
        vec!["rerank".to_owned()]
    } else {
        vec!["generate".to_owned()]
    };
    // OpenAI-compat providers use various field names for context window.
    let ctx = obj
        .get("context_length")
        .or_else(|| obj.get("max_model_len"))
        .or_else(|| obj.get("context_window"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .or_else(|| known_context_window(id));
    Some(DiscoveredModel {
        model_id: id.to_owned(),
        name: id.to_owned(),
        parameter_size: None,
        quantization: None,
        capabilities,
        context_window_tokens: ctx,
    })
}

/// Return the known context window size for well-known model families.
///
/// Used as a fallback when the provider doesn't report context_length.
pub(crate) fn known_context_window(model_id: &str) -> Option<u32> {
    // Use the provider registry for known models.
    let ctx = cairn_domain::provider_registry::context_window_for(model_id);
    // 128_000 is the registry's default — treat as "unknown" so callers
    // can fall back to their own logic.
    if ctx != 128_000 {
        return Some(ctx as u32);
    }
    // Fallback: substring matching for models not in the static registry
    // (e.g. Ollama local models, niche open-source variants).
    let lower = model_id.to_lowercase();
    if lower.contains("qwen3-coder") {
        Some(262_144)
    } else if lower.contains("qwen") {
        Some(32_768)
    } else if lower.contains("llama-2") || lower.contains("llama2") {
        Some(4_096)
    } else if lower.contains("phi-2") || lower.contains("phi2") {
        Some(2_048)
    } else if lower.contains("nomic-embed") || lower.contains("all-minilm") {
        Some(8_192)
    } else {
        None
    }
}

/// Estimate input token count from text length (rough: 1 token ≈ 4 chars).
pub(crate) fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

/// Compute a safe `max_output_tokens` given the context window and input length.
///
/// Strategy:
/// - Reserve `input_estimate + safety_margin` tokens for input + overhead.
/// - Cap output at `context_window / 4` so one response can't consume the
///   entire window (leaves room for multi-turn history).
/// - Minimum output is always at least 256 tokens so short models don't
///   truncate prematurely.
///
/// Returns `None` if context_window is unknown (caller should use its own
/// default).
pub(crate) fn compute_max_output_tokens(context_window: u32, input_estimate: u32) -> u32 {
    const SAFETY_MARGIN: u32 = 512; // reserved for system prompt overhead
    let available = context_window.saturating_sub(input_estimate + SAFETY_MARGIN);
    let quarter_ctx = context_window / 4;
    available.min(quarter_ctx).max(256)
}

/// `GET /v1/providers/connections/:id/test`
///
/// Probes the provider endpoint and returns reachability + round-trip latency.
///
/// Response: `{ "ok": bool, "latency_ms": u64, "provider": str, "status": u16, "detail": str }`
pub(crate) async fn test_connection_handler(
    State(state): State<AppState>,
    Path(connection_id): Path<String>,
    Query(query): Query<DiscoverModelsQuery>,
) -> impl IntoResponse {
    use cairn_domain::ProviderConnectionId;
    use cairn_store::projections::ProviderConnectionReadModel;

    let conn_id = ProviderConnectionId::new(connection_id.clone());
    let adapter_type =
        match ProviderConnectionReadModel::get(state.runtime.store.as_ref(), &conn_id).await {
            Ok(Some(rec)) => rec.adapter_type.to_lowercase(),
            Ok(None) => {
                if query.endpoint_url.is_none() {
                    return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({
                    "error": format!("provider connection '{connection_id}' not found"),
                    "hint": "pass ?endpoint_url=... to test without a registered connection",
                }))).into_response();
                }
                query
                    .adapter_type
                    .clone()
                    .unwrap_or_else(|| "openai_compat".to_owned())
            }
            Err(e) => return internal_error(format!("store error: {e}")).into_response(),
        };
    let adapter_type = query
        .adapter_type
        .as_deref()
        .unwrap_or(&adapter_type)
        .to_lowercase();

    let (stored_endpoint, stored_api_key) =
        if query.endpoint_url.is_none() && query.api_key.is_none() {
            resolve_connection_probe_material(&state, &connection_id).await
        } else {
            (None, None)
        };

    let (probe_url, auth_header) = if adapter_type == "ollama" {
        let host = query
            .endpoint_url
            .as_deref()
            .map(|u| u.trim_end_matches('/').to_owned())
            .or_else(|| stored_endpoint.clone())
            .or_else(|| state.ollama.as_ref().map(|p| p.host().to_owned()));
        match host {
            Some(h) => (format!("{h}/api/tags"), None),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({ "error": "Ollama not configured" })),
                )
                    .into_response();
            }
        }
    } else if adapter_type.contains("bedrock") {
        // Bedrock: probe the runtime endpoint with a simple GET to check reachability.
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        let key = query
            .api_key
            .clone()
            .or_else(|| stored_api_key.clone())
            .or_else(|| std::env::var("BEDROCK_API_KEY").ok())
            .unwrap_or_default();
        let base = query
            .endpoint_url
            .as_deref()
            .map(|u| u.trim_end_matches('/').to_owned())
            .or_else(|| stored_endpoint.clone())
            .unwrap_or_else(|| format!("https://bedrock-runtime.{region}.amazonaws.com"));
        let auth = if key.is_empty() {
            None
        } else {
            Some(format!("Bearer {key}"))
        };
        // Probe the base URL — a 403 or 404 still means reachable.
        (base, auth)
    } else {
        let base = query
            .endpoint_url
            .as_deref()
            .map(|u| u.trim_end_matches('/').to_owned())
            .or_else(|| stored_endpoint.clone())
            .or_else(|| state.openai_compat.as_ref().map(|p| p.base_url.to_string()));
        match base {
            Some(b) => {
                let key = query
                    .api_key
                    .clone()
                    .or_else(|| stored_api_key.clone())
                    .or_else(|| std::env::var("OPENAI_COMPAT_API_KEY").ok())
                    .unwrap_or_default();
                let auth = if key.is_empty() {
                    None
                } else {
                    Some(format!("Bearer {key}"))
                };
                (format!("{b}/models"), auth)
            }
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({ "error": "Provider not configured" })),
                )
                    .into_response();
            }
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let start = std::time::Instant::now();
    let mut req = client.get(&probe_url);
    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }
    match req.send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let status = resp.status().as_u16();
            // For Bedrock: any HTTP response (even 403) means endpoint is reachable.
            let ok = if adapter_type.contains("bedrock") {
                status != 0
            } else {
                resp.status().is_success()
            };
            let detail = if ok && resp.status().is_success() {
                "reachable"
            } else if ok {
                "reachable (auth required)"
            } else {
                "returned non-2xx"
            };
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "ok":         ok,
                    "latency_ms": latency_ms,
                    "provider":   adapter_type,
                    "status":     status,
                    "detail":     detail,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "ok":         false,
                "latency_ms": start.elapsed().as_millis() as u64,
                "provider":   adapter_type,
                "status":     0u16,
                "detail":     format!("connection error: {e}"),
            })),
        )
            .into_response(),
    }
}

/// `POST /v1/providers/ollama/generate` — run a prompt through the local Ollama LLM.
///
/// Body: `{ "model": "llama3", "prompt": "Hello, world!" }`
/// Response: `{ "text", "model", "tokens_in", "tokens_out", "latency_ms" }`
///
/// Returns 503 when OLLAMA_HOST is not configured, 502 when the daemon is
/// unreachable, 500 on model errors.
#[derive(serde::Deserialize)]
pub(crate) struct OllamaGenerateRequest {
    model: Option<String>,
    /// Single-turn prompt (used when `messages` is absent).
    #[serde(default)]
    prompt: String,
    /// Multi-turn conversation history. When present, `prompt` is ignored.
    /// Each element must be `{"role": "user"|"assistant"|"system", "content": "..."}`.
    #[serde(default)]
    messages: Option<Vec<serde_json::Value>>,
    /// Explicit max output tokens override.  When set, bypasses dynamic budgeting.
    #[serde(default)]
    max_tokens: Option<u32>,
}

pub(crate) async fn ollama_generate_handler(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<cairn_api::auth::AuthPrincipal>,
    auth_tenant: Option<axum::Extension<cairn_domain::TenantId>>,
    Query(scope): Query<PlaygroundScopeQuery>,
    Json(body): Json<OllamaGenerateRequest>,
) -> impl IntoResponse {
    if let Err(msg) = validate::check_all(&[
        validate::valid_id("model", &body.model),
        validate::max_len_str("prompt", &body.prompt, validate::MAX_PROMPT_LEN),
    ]) {
        return bad_request(msg).into_response();
    }
    if body.prompt.is_empty() && body.messages.as_ref().is_none_or(|m| m.is_empty()) {
        return bad_request("prompt or messages is required").into_response();
    }

    let default_model = state.runtime.runtime_config.default_generate_model().await;
    let model_id = body.model.as_deref().unwrap_or(&default_model).to_owned();

    let is_admin = cairn_app::is_admin_principal(&principal);
    let tenant_id =
        match resolve_playground_tenant(&scope, &principal, auth_tenant.as_ref(), is_admin) {
            Ok(t) => t,
            Err(resp) => return resp,
        };
    let provider: Arc<dyn cairn_domain::providers::GenerationProvider> = match state
        .runtime
        .provider_registry
        .resolve_generation_for_model(
            &tenant_id,
            &model_id,
            cairn_runtime::ProviderResolutionPurpose::Generate,
        )
        .await
    {
        Ok(Some(provider)) => provider,
        Ok(None) => {
            let is_bedrock_model = model_id.contains('.');
            let is_brain_model = model_id.to_lowercase() == "openrouter/free"
                || model_id.to_lowercase().contains("gemma-3-27b")
                || model_id.to_lowercase().contains("qwen3-coder")
                || model_id.to_lowercase().contains("gemma-4")
                || model_id.to_lowercase().contains("gemma4")
                || model_id.to_lowercase().contains("cyankiwi")
                || model_id.to_lowercase().contains("brain");

            if is_bedrock_model {
                if let Some(ref bedrock) = state.bedrock {
                    bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else {
                    return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                            "error": "Bedrock provider not configured — set BEDROCK_API_KEY and BEDROCK_MODEL_ID"
                        }))).into_response();
                }
            } else if let Some(ref ollama) = state.ollama {
                ollama.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if is_brain_model {
                if let Some(ref brain) = state.openai_compat_brain {
                    brain.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else if let Some(ref worker) = state.openai_compat_worker {
                    worker.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else if let Some(ref or_) = state.openai_compat_openrouter {
                    or_.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else if let Some(ref bedrock) = state.bedrock {
                    bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else {
                    return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                            "error": "Brain provider not configured — set CAIRN_BRAIN_URL, OPENROUTER_API_KEY, or BEDROCK_API_KEY"
                        }))).into_response();
                }
            } else if let Some(ref worker) = state.openai_compat_worker {
                worker.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if let Some(ref brain) = state.openai_compat_brain {
                brain.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if let Some(ref or_) = state.openai_compat_openrouter {
                or_.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if let Some(ref bedrock) = state.bedrock {
                bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else {
                return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                        "error": "No LLM provider configured — set OLLAMA_HOST, CAIRN_BRAIN_URL, CAIRN_WORKER_URL, OPENROUTER_API_KEY, or BEDROCK_API_KEY"
                    }))).into_response();
            }
        }
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    let messages = vec![serde_json::json!({
        "role":    "user",
        "content": body.prompt,
    })];

    // ── Dynamic token budgeting ───────────────────────────────────────────────
    // Estimate how many input tokens the prompt uses, then compute a safe
    // max_output_tokens that fits within the model's context window.
    //
    // Fallback chain:
    //   1. body.max_tokens (explicit caller override) — if set, honour it
    //   2. known_context_window(model_id)             — model-family defaults
    //   3. Hardcoded 8K conservative default           — unknown models
    let input_tokens = estimate_tokens(&body.prompt)
        + body
            .messages
            .as_ref()
            .map(|m| {
                m.iter()
                    .map(|msg| {
                        estimate_tokens(msg.get("content").and_then(|v| v.as_str()).unwrap_or(""))
                    })
                    .sum::<u32>()
            })
            .unwrap_or(0);

    let max_output_tokens: u32 = if let Some(explicit) = body.max_tokens {
        explicit
    } else {
        let ctx_window = known_context_window(&model_id).unwrap_or(8_192);
        compute_max_output_tokens(ctx_window, input_tokens)
    };

    let settings = cairn_domain::providers::ProviderBindingSettings {
        max_output_tokens: Some(max_output_tokens),
        ..Default::default()
    };
    let start = std::time::Instant::now();

    match provider.generate(&model_id, messages, &settings, &[]).await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "text":       resp.text,
                    "model":      resp.model_id,
                    "tokens_in":  resp.input_tokens,
                    "tokens_out": resp.output_tokens,
                    "latency_ms": latency_ms,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let (status, msg) = match &e {
                cairn_domain::providers::ProviderAdapterError::TimedOut => {
                    (StatusCode::GATEWAY_TIMEOUT, e.to_string())
                }
                cairn_domain::providers::ProviderAdapterError::RateLimited => {
                    (StatusCode::TOO_MANY_REQUESTS, e.to_string())
                }
                cairn_domain::providers::ProviderAdapterError::TransportFailure(_) => {
                    (StatusCode::BAD_GATEWAY, e.to_string())
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
            (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
        }
    }
}

// ── Ollama embed handler ──────────────────────────────────────────────────────

/// `POST /v1/memory/embed` — embed a batch of texts using the local Ollama daemon.
///
/// Body: `{ "texts": ["text a", "text b"], "model": "nomic-embed-text" }`
///
/// Returns `{ "embeddings": [[...], [...]], "model": "...", "token_count": N }`.
///
/// Returns 503 when OLLAMA_HOST is not configured, 400 when `texts` is empty,
/// 502 when the daemon is unreachable.
#[derive(serde::Deserialize)]
pub(crate) struct OllamaEmbedRequest {
    texts: Vec<String>,
    model: Option<String>,
}

pub(crate) async fn ollama_embed_handler(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<cairn_api::auth::AuthPrincipal>,
    auth_tenant: Option<axum::Extension<cairn_domain::TenantId>>,
    Query(scope): Query<PlaygroundScopeQuery>,
    Json(body): Json<OllamaEmbedRequest>,
) -> impl IntoResponse {
    if body.texts.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "texts must not be empty" })),
        )
            .into_response();
    }

    let default_ollama_embed = state
        .runtime
        .runtime_config
        .default_ollama_embed_model()
        .await;
    let default_compat_embed = state.runtime.runtime_config.default_embed_model().await;
    let model_id = body
        .model
        .as_deref()
        .unwrap_or_else(|| {
            if state.ollama.is_some() {
                &default_ollama_embed
            } else {
                &default_compat_embed
            }
        })
        .to_owned();

    let is_admin = cairn_app::is_admin_principal(&principal);
    let tenant_id =
        match resolve_playground_tenant(&scope, &principal, auth_tenant.as_ref(), is_admin) {
            Ok(t) => t,
            Err(resp) => return resp,
        };
    let embedder: Arc<dyn cairn_domain::providers::EmbeddingProvider> = match state
        .runtime
        .provider_registry
        .resolve_embedding_for_model(&tenant_id, &model_id)
        .await
    {
        Ok(Some(embedder)) => embedder,
        Ok(None) => {
            let model_id_lower = model_id.to_ascii_lowercase();
            let is_brain_model = model_id_lower == "openrouter/free"
                || model_id_lower.contains("gemma-3-27b")
                || model_id_lower.contains("qwen3-coder")
                || model_id_lower.contains("gemma-4")
                || model_id_lower.contains("gemma4")
                || model_id_lower.contains("cyankiwi")
                || model_id_lower.contains("brain");

            if let Some(ref ollama) = state.ollama {
                Arc::new(OllamaEmbeddingProvider::new(ollama.host()))
            } else if is_brain_model {
                if let Some(ref brain) = state.openai_compat_brain {
                    brain.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
                } else if let Some(ref worker) = state.openai_compat_worker {
                    worker.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
                } else if let Some(ref or_) = state.openai_compat_openrouter {
                    or_.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
                } else {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(serde_json::json!({
                            "error": "No embedding provider configured — set OLLAMA_HOST, CAIRN_WORKER_URL, CAIRN_BRAIN_URL, or OPENROUTER_API_KEY"
                        })),
                    )
                        .into_response();
                }
            } else if let Some(ref worker) = state.openai_compat_worker {
                worker.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
            } else if let Some(ref brain) = state.openai_compat_brain {
                brain.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
            } else if let Some(ref or_) = state.openai_compat_openrouter {
                or_.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
            } else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": "No embedding provider configured — set OLLAMA_HOST, CAIRN_WORKER_URL, CAIRN_BRAIN_URL, or OPENROUTER_API_KEY"
                    })),
                )
                    .into_response();
            }
        }
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    let start = std::time::Instant::now();
    match embedder.embed(&model_id, body.texts).await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "embeddings":   resp.embeddings,
                    "model":        resp.model_id,
                    "token_count":  resp.token_count,
                    "latency_ms":   latency_ms,
                })),
            )
                .into_response()
        }
        Err(e) => {
            use cairn_domain::providers::ProviderAdapterError;
            let (status, msg) = match &e {
                ProviderAdapterError::TimedOut => (StatusCode::GATEWAY_TIMEOUT, e.to_string()),
                ProviderAdapterError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, e.to_string()),
                ProviderAdapterError::TransportFailure(_) => {
                    (StatusCode::BAD_GATEWAY, e.to_string())
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
            (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
        }
    }
}

/// `POST /v1/chat/stream` — stream tokens from any configured LLM provider via SSE.
///
/// Routes to the first available provider: Bedrock → Ollama → OpenAI-compat brain → worker → OpenRouter.
/// Body: `{ "model": "qwen3:8b", "prompt": "...", "messages": [...] }`
/// Emits SSE events:
///   - `event: token`  data: `{"text": "word "}`
///   - `event: done`   data: `{"latency_ms": N, "model": "..."}`
///   - `event: error`  data: `{"error": "..."}`
///
/// Clients read via `fetch()` + `ReadableStream` — no EventSource needed.
pub(crate) fn stream_generation_provider_as_sse(
    provider: Arc<dyn cairn_domain::providers::GenerationProvider>,
    model_id: String,
    messages: Vec<serde_json::Value>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(8);

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let settings = cairn_domain::providers::ProviderBindingSettings::default();
        match provider.generate(&model_id, messages, &settings, &[]).await {
            Ok(resp) => {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("token")
                        .data(serde_json::json!({"text": resp.text}).to_string())))
                    .await;
                let _ = tx
                    .send(Ok(Event::default().event("done").data(
                        serde_json::json!({
                            "latency_ms": start.elapsed().as_millis() as u64,
                            "model": resp.model_id,
                            "tokens_in": resp.input_tokens,
                            "tokens_out": resp.output_tokens,
                        })
                        .to_string(),
                    )))
                    .await;
            }
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::json!({"error": err.to_string()}).to_string(),
                    )))
                    .await;
            }
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

pub(crate) fn stream_chat_provider_as_sse(
    provider: Arc<dyn cairn_providers::chat::ChatProvider>,
    model_id: String,
    messages: Vec<cairn_providers::chat::ChatMessage>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut stream = match provider.chat_stream(&messages, None).await {
            Ok(stream) => stream,
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::json!({"error": err.to_string()}).to_string(),
                    )))
                    .await;
                return;
            }
        };

        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            match chunk {
                Ok(text) if !text.is_empty() => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("token")
                            .data(serde_json::json!({"text": text}).to_string())))
                        .await;
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(
                            serde_json::json!({"error": err.to_string()}).to_string(),
                        )))
                        .await;
                    return;
                }
            }
        }

        let _ = tx
            .send(Ok(Event::default().event("done").data(
                serde_json::json!({
                    "latency_ms": start.elapsed().as_millis() as u64,
                    "model": model_id,
                })
                .to_string(),
            )))
            .await;
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

pub(crate) async fn chat_stream_handler(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<cairn_api::auth::AuthPrincipal>,
    auth_tenant: Option<axum::Extension<cairn_domain::TenantId>>,
    Query(scope): Query<PlaygroundScopeQuery>,
    Json(body): Json<OllamaGenerateRequest>,
) -> impl IntoResponse {
    let default_stream = state.runtime.runtime_config.default_stream_model().await;
    let model_id = body.model.as_deref().unwrap_or(&default_stream).to_owned();
    let is_admin = cairn_app::is_admin_principal(&principal);
    let tenant_id =
        match resolve_playground_tenant(&scope, &principal, auth_tenant.as_ref(), is_admin) {
            Ok(t) => t,
            Err(resp) => return resp,
        };
    let messages: Vec<serde_json::Value> = body
        .messages
        .unwrap_or_else(|| vec![serde_json::json!({"role": "user", "content": body.prompt})]);
    let chat_messages = cairn_runtime::json_messages_to_chat_messages(&messages);
    let has_active_connections = match state
        .runtime
        .provider_registry
        .has_active_connections(&tenant_id)
        .await
    {
        Ok(has_connections) => has_connections,
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    // Bedrock models contain a '.' (e.g. minimax.minimax-m2.5). Connection-backed
    // Bedrock routes resolve through the registry first; otherwise we keep the
    // existing single-shot static fallback and wrap it as SSE.
    let is_bedrock_model = model_id.contains('.');
    if is_bedrock_model && has_active_connections {
        match state
            .runtime
            .provider_registry
            .resolve_generation_for_model(
                &tenant_id,
                &model_id,
                cairn_runtime::ProviderResolutionPurpose::Stream,
            )
            .await
        {
            Ok(Some(provider)) => {
                return stream_generation_provider_as_sse(
                    provider,
                    model_id.clone(),
                    messages.clone(),
                );
            }
            Ok(None) => {}
            Err(err) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    axum::Json(serde_json::json!({ "error": err.to_string() })),
                )
                    .into_response();
            }
        }
    } else if has_active_connections {
        match state
            .runtime
            .provider_registry
            .resolve_chat_for_model(
                &tenant_id,
                &model_id,
                cairn_runtime::ProviderResolutionPurpose::Stream,
            )
            .await
        {
            Ok(Some(provider)) => {
                return stream_chat_provider_as_sse(
                    provider,
                    model_id.clone(),
                    chat_messages.clone(),
                );
            }
            Ok(None) => {}
            Err(err) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    axum::Json(serde_json::json!({ "error": err.to_string() })),
                )
                    .into_response();
            }
        }
    }

    // Issue #156: if the tenant has active connections but the registry
    // returned `Ok(None)` above (no connection serves this model), DO NOT
    // fall through to the global static fallbacks — that would silently
    // route the request to whatever env-configured provider happens to be
    // present and mask the real problem. Emit the actionable 422 now.
    if has_active_connections {
        let summaries = match state
            .runtime
            .provider_registry
            .active_connection_summaries(&tenant_id)
            .await
        {
            Ok(summaries) => summaries,
            Err(err) => {
                // Surface the store/runtime failure instead of
                // silently returning an empty `active_connections`
                // list in the 422 body — that would make the error
                // message claim "no connection serves this model"
                // when the real cause is the store itself.
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({
                        "error": format!(
                            "Failed to load active connection summaries for tenant '{}': {err}",
                            tenant_id.as_str(),
                        ),
                        "tenant_id": tenant_id.as_str(),
                        "requested_model": model_id,
                    })),
                )
                    .into_response();
            }
        };
        let conn_list: Vec<serde_json::Value> = summaries
            .iter()
            .map(|(id, models)| {
                serde_json::json!({
                    "connection_id": id,
                    "supported_models": models,
                })
            })
            .collect();
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({
                "error": format!(
                    "No registered connection for tenant '{}' supports model '{}'. \
                     Register with POST /v1/providers/connections with supported_models \
                     including '{}', or call GET /v1/providers/connections/:id/discover-models \
                     to refresh.",
                    tenant_id.as_str(),
                    model_id,
                    model_id,
                ),
                "tenant_id": tenant_id.as_str(),
                "requested_model": model_id,
                "active_connections": conn_list,
            })),
        )
            .into_response();
    }

    if is_bedrock_model {
        if let Some(ref bedrock) = state.bedrock {
            return stream_generation_provider_as_sse(
                bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>,
                model_id,
                messages,
            );
        }
        // Fall through to OpenAI-compat if no Bedrock provider.
    }

    // Static provider resolution: Ollama first, then OpenAI-compat brain, then worker.
    // All three expose /v1/chat/completions (OpenAI wire format) so the same
    // streaming logic applies — only URL construction differs.
    let (stream_url, stream_api_key): (String, String) = if let Some(ref o) = state.ollama {
        (format!("{}/v1/chat/completions", o.host()), String::new())
    } else if let Some(ref brain) = state.openai_compat_brain {
        (
            format!(
                "{}/chat/completions",
                brain.base_url.as_str().trim_end_matches('/')
            ),
            brain.api_key.as_str().to_owned(),
        )
    } else if let Some(ref worker) = state.openai_compat_worker {
        (
            format!(
                "{}/chat/completions",
                worker.base_url.as_str().trim_end_matches('/')
            ),
            worker.api_key.as_str().to_owned(),
        )
    } else if let Some(ref or_) = state.openai_compat_openrouter {
        (
            format!(
                "{}/chat/completions",
                or_.base_url.as_str().trim_end_matches('/')
            ),
            or_.api_key.as_str().to_owned(),
        )
    } else {
        // Reached only when the tenant has NO active connections AND no
        // env-var static fallback is configured — the genuine "nothing
        // wired up" case. When active connections exist but don't serve
        // the model, control returned above with the actionable 422.
        return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({
                    "error": "No LLM provider configured — set OLLAMA_HOST, CAIRN_BRAIN_URL, CAIRN_WORKER_URL, or OPENROUTER_API_KEY"
                })),
            ).into_response();
    };
    let disable_thinking = state
        .runtime
        .runtime_config
        .supports_thinking_mode(&model_id)
        .await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .unwrap_or_default();

        let mut req_body = serde_json::json!({
            "model":    model_id,
            "messages": messages,
            "stream":   true,
        });
        if disable_thinking {
            req_body["options"] = serde_json::json!({ "think": false });
        }

        let mut req = client.post(&stream_url).json(&req_body);
        if !stream_api_key.is_empty() {
            req = req.bearer_auth(&stream_api_key);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::json!({"error": err.to_string()}).to_string(),
                    )))
                    .await;
                return;
            }
        };

        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            let _ = tx
                .send(Ok(Event::default()
                    .event("error")
                    .data(serde_json::json!({"error": msg}).to_string())))
                .await;
            return;
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(err) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(
                            serde_json::json!({"error": err.to_string()}).to_string(),
                        )))
                        .await;
                    return;
                }
            };

            buf.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_owned();
                buf = buf[nl + 1..].to_owned();

                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                if data == "[DONE]" {
                    break;
                }

                let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else {
                    continue;
                };

                if let Some(text) = parsed
                    .get("choices")
                    .and_then(|choices| choices.get(0))
                    .and_then(|choice| choice.get("delta"))
                    .and_then(|delta| delta.get("content"))
                    .and_then(|content| content.as_str())
                {
                    if !text.is_empty() {
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("token")
                                .data(serde_json::json!({"text": text}).to_string())))
                            .await;
                    }
                }
            }
        }

        let _ = tx
            .send(Ok(Event::default().event("done").data(
                serde_json::json!({
                    "latency_ms": start.elapsed().as_millis() as u64,
                    "model": model_id,
                })
                .to_string(),
            )))
            .await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ── Ollama model management handlers ─────────────────────────────────────────

#[derive(serde::Deserialize)]
pub(crate) struct OllamaModelNameRequest {
    /// Model name, e.g. `"qwen3:8b"` or `"nomic-embed-text"`.
    model: String,
}

/// `POST /v1/providers/ollama/pull` — pull (download) a model into Ollama.
///
/// Body: `{ "model": "qwen3:8b" }`
///
/// Proxies to `POST OLLAMA_HOST/api/pull` with `stream: false`.
/// Returns `200 { "status": "success" }` on completion, `4xx`/`5xx` on error.
pub(crate) async fn ollama_pull_handler(
    State(state): State<AppState>,
    Json(body): Json<OllamaModelNameRequest>,
) -> impl IntoResponse {
    let Some(provider) = &state.ollama else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "Ollama not configured"})),
        )
            .into_response();
    };
    let url = format!("{}/api/pull", provider.host());
    let client = reqwest::Client::builder()
        // Pulling large models can take many minutes — long timeout.
        .timeout(std::time::Duration::from_secs(3600))
        .build()
        .unwrap_or_default();

    match client
        .post(&url)
        .json(&serde_json::json!({"name": body.model, "stream": false}))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({"status": "success", "model": body.model})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    axum::Json(serde_json::json!({"error": body_text})),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// `POST /v1/providers/ollama/delete` — delete a model from the local Ollama registry.
///
/// Body: `{ "model": "qwen3:8b" }`
///
/// Proxies to `DELETE OLLAMA_HOST/api/delete`.
/// Returns `200` on success, `404` when the model is not found.
pub(crate) async fn ollama_delete_model_handler(
    State(state): State<AppState>,
    Json(body): Json<OllamaModelNameRequest>,
) -> impl IntoResponse {
    let Some(provider) = &state.ollama else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "Ollama not configured"})),
        )
            .into_response();
    };
    let url = format!("{}/api/delete", provider.host());
    let client = reqwest::Client::new();

    match client
        .delete(&url)
        .json(&serde_json::json!({"name": body.model}))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({"status": "deleted", "model": body.model})),
                )
                    .into_response()
            } else {
                let msg = resp.text().await.unwrap_or_default();
                let code = if msg.contains("not found") {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::BAD_GATEWAY
                };
                (code, axum::Json(serde_json::json!({"error": msg}))).into_response()
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Ollama model info handler ─────────────────────────────────────────────────

/// `GET /v1/providers/ollama/models/:name/info` — detailed info for one model.
///
/// Calls `POST OLLAMA_HOST/api/show` + `GET OLLAMA_HOST/api/tags` and returns
/// the fields most useful for an operator dashboard:
///
/// ```json
/// {
///   "name": "qwen3:8b",
///   "family": "qwen3",
///   "format": "gguf",
///   "parameter_size": "8.2B",
///   "parameter_count": 8190735360,
///   "quantization_level": "Q4_K_M",
///   "context_length": 40960,
///   "size_bytes": 5234519167,
///   "size_human": "4.9 GB"
/// }
/// ```
pub(crate) async fn ollama_model_info_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Some(provider) = &state.ollama else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "Ollama not configured"})),
        )
            .into_response();
    };

    let host = provider.host().to_owned();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    // ── Call /api/show ────────────────────────────────────────────────────────
    let show_resp = match client
        .post(format!("{host}/api/show"))
        .json(&serde_json::json!({"name": name}))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            let msg = r.text().await.unwrap_or_default();
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": msg})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    let show: serde_json::Value = match show_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    // ── Extract fields from `details` + `model_info` ─────────────────────────
    let details = &show["details"];
    let model_info = &show["model_info"];

    let family = details["family"].as_str().unwrap_or("unknown");
    let format = details["format"].as_str().unwrap_or("unknown");
    let parameter_size = details["parameter_size"].as_str().unwrap_or("unknown");
    let quantization_level = details["quantization_level"].as_str().unwrap_or("unknown");

    // Derive architecture key (e.g. "qwen3", "llama") for model_info lookups.
    let arch = family;
    let parameter_count = model_info
        .get("general.parameter_count")
        .and_then(|v| v.as_u64());
    let context_length = model_info
        .get(format!("{arch}.context_length"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            model_info
                .get("llama.context_length")
                .and_then(|v| v.as_u64())
        });
    let embedding_length = model_info
        .get(format!("{arch}.embedding_length"))
        .and_then(|v| v.as_u64());

    // ── Get disk size from /api/tags ──────────────────────────────────────────
    let (size_bytes, size_human) = match client.get(format!("{host}/api/tags")).send().await {
        Ok(r) if r.status().is_success() => {
            if let Ok(tags) = r.json::<serde_json::Value>().await {
                let size = tags["models"]
                    .as_array()
                    .and_then(|arr| arr.iter().find(|m| m["name"].as_str() == Some(&name)))
                    .and_then(|m| m["size"].as_u64())
                    .unwrap_or(0);
                let human = if size >= 1_073_741_824 {
                    format!("{:.1} GB", size as f64 / 1_073_741_824.0)
                } else if size >= 1_048_576 {
                    format!("{:.0} MB", size as f64 / 1_048_576.0)
                } else {
                    format!("{size} B")
                };
                (size, human)
            } else {
                (0, "unknown".to_owned())
            }
        }
        _ => (0, "unknown".to_owned()),
    };

    (StatusCode::OK, axum::Json(serde_json::json!({
        "name":               name,
        "family":             family,
        "format":             format,
        "parameter_size":     parameter_size,
        "parameter_count":    parameter_count,
        "quantization_level": quantization_level,
        "context_length":     context_length,
        "embedding_length":   embedding_length,
        "size_bytes":         if size_bytes > 0 { serde_json::Value::Number(size_bytes.into()) } else { serde_json::Value::Null },
        "size_human":         size_human,
    }))).into_response()
}

// System info handler → bin_handlers.rs
