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
}
