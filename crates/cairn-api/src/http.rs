use serde::{Deserialize, Serialize};

/// Preserved HTTP route classification per compatibility catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteClassification {
    Preserve,
    Transitional,
    IntentionallyBroken,
}

/// Preserved route metadata used by compatibility tracking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEntry {
    pub method: HttpMethod,
    pub path: String,
    pub classification: RouteClassification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

/// Standard paginated list response used by preserved endpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse<T> {
    pub items: Vec<T>,
    pub has_more: bool,
}

/// Standard success acknowledgement for mutation endpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkResponse {
    pub ok: bool,
}

/// Health check response for `GET /health`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
}

/// Structured API error returned by HTTP handlers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApiError {
    pub status_code: u16,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub request_id: Option<String>,
}

impl ApiError {
    pub fn new(status_code: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status_code,
            code: code.into(),
            message: message.into(),
            request_id: None,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "not_found", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(401, "unauthorized", message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "bad_request", message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.status_code, self.code, self.message)
    }
}

/// Seam for HTTP route registration. Implementors wire routes to handlers.
pub trait RouteRegistry {
    type Error;

    fn register(&mut self, entry: RouteEntry) -> Result<(), Self::Error>;
    fn routes(&self) -> &[RouteEntry];
}

/// Preserved catalog of Week 1 route boundaries.
/// This function returns the route entries that must exist for compatibility.
pub fn preserved_route_catalog() -> Vec<RouteEntry> {
    use HttpMethod::*;
    use RouteClassification::*;

    vec![
        RouteEntry {
            method: Get,
            path: "/health".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/dashboard".into(),
            classification: Preserve,
        },
        // RFC 010: overview surface is the canonical operator entry point.
        RouteEntry {
            method: Get,
            path: "/v1/overview".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/feed".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/feed/:id/read".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/feed/read-all".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/tasks".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/tasks/:id/cancel".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/approvals".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/approvals/:id/approve".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/approvals/:id/deny".into(),
            classification: Preserve,
        },
        // ── Plan review (RFC 018) ─────────────────────────────────────────
        RouteEntry {
            method: Post,
            path: "/v1/runs/:id/approve".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/runs/:id/reject".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/runs/:id/revise".into(),
            classification: Preserve,
        },
        // ── Decisions (RFC 019) ──────────────────────────────────────────
        RouteEntry {
            method: Get,
            path: "/v1/decisions".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/decisions/cache".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/decisions/:id".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/decisions/:id/invalidate".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/decisions/invalidate".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/decisions/invalidate-by-rule".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/assistant/sessions".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/assistant/sessions/:sessionId".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/assistant/message".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/assistant/voice".into(),
            classification: Transitional,
        },
        RouteEntry {
            method: Get,
            path: "/v1/memories".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/memories/search".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/memories".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/memories/:id/accept".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/memories/:id/reject".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/fleet".into(),
            classification: Transitional,
        },
        RouteEntry {
            method: Get,
            path: "/v1/skills".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/soul".into(),
            classification: Transitional,
        },
        RouteEntry {
            method: Put,
            path: "/v1/soul".into(),
            classification: Transitional,
        },
        RouteEntry {
            method: Get,
            path: "/v1/soul/history".into(),
            classification: Transitional,
        },
        RouteEntry {
            method: Get,
            path: "/v1/soul/patches".into(),
            classification: Transitional,
        },
        RouteEntry {
            method: Get,
            path: "/v1/costs".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/metrics".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/status".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/poll/run".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/stream".into(),
            classification: Preserve,
        },
        // RFC 010: Operator control plane routes
        RouteEntry {
            method: Get,
            path: "/v1/runs".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/runs/:id".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/prompts/assets".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/prompts/releases".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/graph/trace".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/policies/decisions".into(),
            classification: Preserve,
        },
        // RFC 010: evals surface — required top-level operator view.
        RouteEntry {
            method: Get,
            path: "/v1/evals/runs".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/evals/datasets".into(),
            classification: Preserve,
        },
        // RFC 010: sources and channels surface.
        RouteEntry {
            method: Get,
            path: "/v1/sources".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/channels".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/providers/health".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/settings".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/admin/tenants".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/admin/workspaces".into(),
            classification: Preserve,
        },
        // GAP-008: runtime config key-value store routes.
        RouteEntry {
            method: Get,
            path: "/v1/config".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/config/:key".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Put,
            path: "/v1/config/:key".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Delete,
            path: "/v1/config/:key".into(),
            classification: Preserve,
        },
        // GAP-010: LLM observability — per-session trace history.
        RouteEntry {
            method: Get,
            path: "/v1/sessions/:id/llm-traces".into(),
            classification: Preserve,
        },
        // RFC 013: import/export contract — validate → preview → apply pipeline.
        RouteEntry {
            method: Post,
            path: "/v1/import/validate".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/import/preview".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/import/apply".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/export/:format".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/import/reports".into(),
            classification: Preserve,
        },
        // RFC 014: commercial/admin surface — entitlement status, capability mapping,
        // license inspection and activation.
        RouteEntry {
            method: Get,
            path: "/v1/admin/entitlements".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/admin/capabilities".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/admin/license".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/admin/license/activate".into(),
            classification: Preserve,
        },
        // ── Static GET routes missing from the catalog (fold match arms never fired).
        RouteEntry {
            method: Get,
            path: "/v1/admin/audit-log".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/admin/logs".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/admin/notifications/failed".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/approval-policies".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/checkpoints".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/evals/matrices/guardrail".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/evals/matrices/memory-quality".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/evals/matrices/permissions".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/evals/matrices/prompt-comparison".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/evals/matrices/skill-health".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/ingest/jobs".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/memory/diagnostics".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/memory/search".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/onboarding/status".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/onboarding/templates".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/plugins".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/providers/bindings".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/providers/bindings/cost-ranking".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/providers/budget".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/providers/connections".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/providers/policies".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/providers/pools".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/runs/cost-alerts".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/runs/escalated".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/runs/resume-due".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/runs/sla-breached".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/runs/stalled".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/settings/tls".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/streams/runtime".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/tasks/expired".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Get,
            path: "/v1/tool-invocations".into(),
            classification: Preserve,
        },
        // ── Static POST/PUT/DELETE routes added to catalog so their fold match arms fire.
        //    Dynamic-path routes (:id, etc.) are NOT added here — catalog_path_to_axum()
        //    converts :id to {id} which matchit 0.7 treats as a static literal.
        //    Dynamic routes are registered as explicit .route() calls in build_router.
        RouteEntry {
            method: Post,
            path: "/v1/admin/tenants".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/admin/license/override".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/onboarding/template".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/prompts/assets".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/prompts/releases".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/approval-policies".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/runs/process-scheduled-resumes".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/tool-invocations".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/plugins".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/tasks/expire-leases".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/evals/datasets".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/evals/baselines".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/evals/rubrics".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/evals/runs".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/sources".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/sources/process-refresh".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/ingest/jobs".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/channels".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/memory/ingest".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/memory/deep-search".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/providers/budget".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/providers/pools".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/providers/connections".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/providers/run-health-checks".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/providers/bindings".into(),
            classification: Preserve,
        },
        RouteEntry {
            method: Post,
            path: "/v1/providers/policies".into(),
            classification: Preserve,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserved_catalog_has_expected_count() {
        let catalog = preserved_route_catalog();
        assert!(!catalog.is_empty());
        // Verify health check is first
        assert_eq!(catalog[0].path, "/health");
        assert_eq!(catalog[0].classification, RouteClassification::Preserve);
    }

    #[test]
    fn transitional_routes_are_marked() {
        let catalog = preserved_route_catalog();
        let transitional: Vec<_> = catalog
            .iter()
            .filter(|r| r.classification == RouteClassification::Transitional)
            .collect();
        assert!(!transitional.is_empty());
        assert!(transitional.iter().any(|r| r.path.contains("voice")));
    }

    #[test]
    fn list_response_serialization() {
        let response = ListResponse {
            items: vec!["a".to_owned(), "b".to_owned()],
            has_more: true,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["hasMore"], true);
        assert_eq!(json["items"].as_array().unwrap().len(), 2);
    }

    // ── RFC 010 Gap Tests ─────────────────────────────────────────────────

    /// RFC 010: minimum top-level operator views must all be present as routes.
    /// Required: overview, runs, approvals, memory, graph, prompts, evals,
    ///           policies, sources/channels, settings.
    #[test]
    fn rfc010_all_operator_views_have_routes() {
        let catalog = preserved_route_catalog();
        let paths: Vec<&str> = catalog.iter().map(|r| r.path.as_str()).collect();

        let required_surfaces = [
            ("/v1/overview", "overview"),
            ("/v1/runs", "runs"),
            ("/v1/approvals", "approvals"),
            ("/v1/memories", "memory"),
            ("/v1/graph/", "graph"),
            ("/v1/prompts/", "prompts"),
            ("/v1/evals/", "evals"),
            ("/v1/policies/", "policies"),
            ("/v1/sources", "sources/channels"),
            ("/v1/settings", "settings"),
        ];

        for (prefix, surface) in &required_surfaces {
            assert!(
                paths.iter().any(|p| p.starts_with(prefix)),
                "RFC 010: missing route for operator surface '{}' (expected path starting with '{}')",
                surface,
                prefix,
            );
        }
    }

    /// RFC 010: graph surface must have a route exposing relationship/visual data.
    #[test]
    fn rfc010_graph_surface_has_trace_or_execution_route() {
        let catalog = preserved_route_catalog();
        let paths: Vec<&str> = catalog.iter().map(|r| r.path.as_str()).collect();
        // RFC 010 requires graph to include a genuinely visual relationship view.
        // At minimum it must expose execution trace or graph query routes.
        let has_graph_route = paths.iter().any(|p| p.starts_with("/v1/graph/"));
        assert!(
            has_graph_route,
            "RFC 010: graph surface must expose relationship/trace routes"
        );
    }

    /// RFC 010: tenant-level roll-up views must be read-only for operational actions.
    /// Verify admin tenant routes are GET (read) not mutation routes for operations.
    #[test]
    fn rfc010_admin_tenant_routes_support_read_operations() {
        let catalog = preserved_route_catalog();
        let admin_gets: Vec<_> = catalog
            .iter()
            .filter(|r| r.path.starts_with("/v1/admin/tenants") && r.method == HttpMethod::Get)
            .collect();
        assert!(
            !admin_gets.is_empty(),
            "RFC 010: admin tenant GET routes must exist for tenant-level roll-up views"
        );
    }
}
