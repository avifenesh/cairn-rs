//! HTTP tests for the skills catalog endpoints.
//!
//! Regression for issue #147 — `GET /v1/skills` was a hard-coded empty
//! stub. These tests exercise the real wiring into the
//! `cairn_domain::skills::SkillCatalog` that `AppState` owns.

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_domain::skills::{Skill, SkillStatus};
use cairn_domain::tenancy::TenantKey;
use cairn_domain::OperatorId;
use tower::ServiceExt;

const TOKEN: &str = "skills-test-token";

async fn app_with_token() -> (axum::Router, std::sync::Arc<cairn_app::AppState>) {
    let (app, state) = support::build_test_router_fake_fabric(BootstrapConfig::default()).await;
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );
    (app, state)
}

fn seed_skill(id: &str, enabled: bool, tags: &[&str]) -> Skill {
    Skill {
        skill_id: id.to_owned(),
        name: format!("{id} Skill"),
        description: format!("Capability for {id}"),
        version: "1.0.0".to_owned(),
        entry_point: format!("skills/{id}/main.md"),
        required_permissions: vec![],
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        enabled,
        status: if enabled {
            SkillStatus::Active
        } else {
            SkillStatus::Proposed
        },
    }
}

#[tokio::test]
async fn list_skills_returns_registered_entries() {
    let (app, state) = app_with_token().await;

    {
        let mut catalog = state.skill_catalog.write().await;
        catalog.register(seed_skill("coder", true, &["coding"]));
        catalog.register(seed_skill("writer", false, &["content"]));
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let items = json["items"].as_array().expect("items array");
    assert_eq!(
        items.len(),
        2,
        "both registered skills must surface: {json}"
    );
    // Catalog.list() sorts by skill_id.
    assert_eq!(items[0]["skill_id"], "coder");
    assert_eq!(items[0]["enabled"], true);
    assert_eq!(items[0]["version"], "1.0.0");
    assert_eq!(items[1]["skill_id"], "writer");
    assert_eq!(items[1]["enabled"], false);

    assert_eq!(json["summary"]["total"], 2);
    assert_eq!(json["summary"]["enabled"], 1);
    assert_eq!(json["summary"]["disabled"], 1);

    let active = json["currently_active"]
        .as_array()
        .expect("currently_active");
    assert_eq!(active.len(), 1, "only `coder` is Active");
    // Legacy camelCase alias must still be emitted for byte-for-byte
    // compatibility with the previous stub response shape.
    let active_camel = json["currentlyActive"]
        .as_array()
        .expect("currentlyActive alias");
    assert_eq!(
        active_camel, active,
        "camelCase alias must mirror snake_case"
    );
    assert_eq!(active[0], "coder");
}

#[tokio::test]
async fn list_skills_empty_on_fresh_state() {
    let (app, _state) = app_with_token().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 0);
    assert_eq!(json["summary"]["total"], 0);
    assert_eq!(json["currently_active"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_skills_filters_by_tag() {
    let (app, state) = app_with_token().await;
    {
        let mut catalog = state.skill_catalog.write().await;
        catalog.register(seed_skill("coder", true, &["coding", "review"]));
        catalog.register(seed_skill("writer", true, &["content"]));
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills?tag=coding")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["skill_id"], "coder");
}

/// Regression for a reviewer-flagged edge case: the domain
/// `SkillCatalog::disable()` only clears `enabled` and leaves
/// `status` as `Active`. The list handler must gate
/// `currently_active` on BOTH flags so a disabled skill doesn't
/// keep showing up under the UI's "Currently active" panel.
#[tokio::test]
async fn list_skills_excludes_disabled_from_currently_active() {
    let (app, state) = app_with_token().await;
    {
        let mut catalog = state.skill_catalog.write().await;
        catalog.register(seed_skill("coder", true, &["coding"]));
        // enabled=true + Active, then disable() — leaves status=Active,
        // clears enabled. Handler must exclude it.
        assert!(catalog.disable("coder"));
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["items"].as_array().unwrap().len(), 1);
    assert_eq!(json["summary"]["enabled"], 0);
    assert_eq!(json["summary"]["disabled"], 1);
    assert!(
        json["currently_active"].as_array().unwrap().is_empty(),
        "disabled skill must NOT appear under currently_active: {json}"
    );
    assert!(
        json["currentlyActive"].as_array().unwrap().is_empty(),
        "camelCase alias must mirror snake_case exclusion"
    );
}

#[tokio::test]
async fn get_skill_returns_detail_or_404() {
    let (app, state) = app_with_token().await;
    {
        let mut catalog = state.skill_catalog.write().await;
        catalog.register(seed_skill("coder", true, &["coding"]));
    }

    let hit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills/coder")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let hit_status = hit.status();
    let hit_body = to_bytes(hit.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        hit_status,
        StatusCode::OK,
        "GET /v1/skills/coder body: {}",
        String::from_utf8_lossy(&hit_body)
    );
    let json: serde_json::Value = serde_json::from_slice(&hit_body).unwrap();
    assert_eq!(json["skill_id"], "coder");
    assert_eq!(json["entry_point"], "skills/coder/main.md");
    assert_eq!(json["status"], "active");

    let miss = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills/unknown")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(miss.status(), StatusCode::NOT_FOUND);
    // 404 body uses the standard AppApiError envelope
    // (`{status_code, code, message, request_id}`), not a bespoke shape.
    let miss_body = to_bytes(miss.into_body(), usize::MAX).await.unwrap();
    let miss_json: serde_json::Value = serde_json::from_slice(&miss_body).unwrap();
    assert_eq!(miss_json["status_code"], 404);
    assert_eq!(miss_json["code"], "skill_not_found");
    assert!(
        miss_json["message"]
            .as_str()
            .unwrap_or("")
            .contains("unknown"),
        "error message should name the missing id: {miss_json}"
    );
}

/// Regression for issue #236: the binary serves an SPA "catch-all"
/// fallback that returns index.html for anything not matched by the
/// axum Router. The `/v1/skills/:id` handler was reported to fall into
/// that catch-all and return 200 text/html instead of a 404 JSON
/// envelope. This test reproduces the binary-side router composition
/// (catalog-routes + binary-routes + SPA fallback) in-process so
/// regressions can be caught without spawning a subprocess.
#[tokio::test]
async fn get_skill_404_wins_over_spa_fallback() {
    use axum::routing::get as axum_get;

    let bootstrap = BootstrapConfig::default();
    let (_, state) = support::build_test_router_fake_fabric(bootstrap).await;
    state.service_tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );
    {
        let mut catalog = state.skill_catalog.write().await;
        catalog.register(seed_skill("coder", true, &["coding"]));
    }

    // Binary-style composition: catalog routes + an empty "binary
    // routes" merge + a permissive fallback that mimics `serve_frontend`
    // (returns 200 text/html index.html for anything unmatched).
    async fn spa_fallback() -> impl axum::response::IntoResponse {
        (
            axum::http::StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/html; charset=utf-8",
            )],
            "<!doctype html><html><body>spa</body></html>",
        )
    }
    let catalog_routes =
        cairn_app::AppBootstrap::build_catalog_routes().with_state(state.clone());
    let binary_routes: axum::Router = axum::Router::new().with_state(());
    let merged = catalog_routes
        .merge(binary_routes)
        .fallback(axum_get(spa_fallback));
    let app = cairn_app::AppBootstrap::apply_middleware(merged, state.clone());

    // Known id — must hit the real handler and return JSON.
    let hit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills/coder")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hit.status(), StatusCode::OK);
    let ct_hit = hit
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    assert!(
        ct_hit.starts_with("application/json"),
        "known skill must return JSON, got {ct_hit:?}"
    );

    // Unknown id — must return 404 application/json, NOT the SPA
    // fallback's 200 text/html. This is the regression under test.
    let miss = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/skills/no-such-id")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = miss.status();
    let ct = miss
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = to_bytes(miss.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404 for unknown skill id, got {status} ({ct}) body: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        ct.starts_with("application/json"),
        "expected application/json content-type, got {ct:?}"
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "skill_not_found");
}
