use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    response::Response,
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppBootstrap;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::{
    ApprovalId, ApprovalRequirement, EventEnvelope, EventId, EventSource, OperatorId, ProjectKey,
    ProviderBindingId, ProviderCallCompleted, ProviderCallId, ProviderConnectionId,
    ProviderModelId, RouteAttemptId, RouteDecisionId, RunId, RuntimeEvent, ToolInvocationId,
    ToolInvocationProgressUpdated,
};
use cairn_graph::projections::{EdgeKind, GraphEdge, GraphProjection};
use cairn_runtime::ApprovalService;
use cairn_store::EventLog;
use std::{fs, path::PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{sleep, timeout, Duration},
};
use tower::ServiceExt;

fn valid_bundle_body() -> serde_json::Value {
    serde_json::json!({
        "bundle_schema_version": "1",
        "bundle_type": "curated_knowledge_pack_bundle",
        "bundle_id": "bundle_http_curated",
        "bundle_name": "HTTP Curated Pack",
        "created_at": 1_710_000_000u64,
        "created_by": "operator_http",
        "source_deployment_id": null,
        "source_scope": {
            "tenant_id": "acme",
            "workspace_id": "eng",
            "project_id": "support"
        },
        "artifact_count": 2,
        "artifacts": [
            {
                "artifact_kind": "knowledge_document",
                "artifact_logical_id": "doc_http_bundle_1",
                "artifact_display_name": "Install Guide",
                "origin_scope": {
                    "tenant_id": "acme",
                    "workspace_id": "eng",
                    "project_id": "support"
                },
                "origin_artifact_id": null,
                "content_hash": "hash_install",
                "source_bundle_id": "bundle_http_curated",
                "origin_timestamp": 1_710_000_001u64,
                "metadata": {},
                "payload": {
                    "knowledge_pack_logical_id": "bundle_http_curated",
                    "document_name": "Install Guide",
                    "source_type": "text_plain",
                    "content": {
                        "kind": "inline_text",
                        "text": "Install Cairn with cargo install cairn-cli."
                    },
                    "metadata": {},
                    "chunk_hints": [],
                    "retrieval_hints": ["install"]
                },
                "lineage": null,
                "tags": ["install"]
            },
            {
                "artifact_kind": "knowledge_document",
                "artifact_logical_id": "doc_http_bundle_2",
                "artifact_display_name": "Reset Password",
                "origin_scope": {
                    "tenant_id": "acme",
                    "workspace_id": "eng",
                    "project_id": "support"
                },
                "origin_artifact_id": null,
                "content_hash": "hash_reset",
                "source_bundle_id": "bundle_http_curated",
                "origin_timestamp": 1_710_000_002u64,
                "metadata": {},
                "payload": {
                    "knowledge_pack_logical_id": "bundle_http_curated",
                    "document_name": "Reset Password",
                    "source_type": "text_plain",
                    "content": {
                        "kind": "inline_text",
                        "text": "Reset the password from the account portal."
                    },
                    "metadata": {},
                    "chunk_hints": [],
                    "retrieval_hints": ["support"]
                },
                "lineage": null,
                "tags": ["support"]
            }
        ],
        "provenance": {
            "description": "HTTP bundle import test",
            "source_system": "integration_test",
            "export_reason": "round_trip"
        }
    })
}

#[tokio::test]
async fn health_endpoint_returns_200() {
    let bootstrap = AppBootstrap;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = BootstrapConfig::default();
    let server =
        tokio::spawn(async move { bootstrap.serve_with_listener(listener, &config).await });

    let response = request_health(addr).await;

    server.abort();
    let _ = server.await;

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("\"status\":\"healthy\""));
    assert!(response.contains("\"store_ok\":true"));
    assert!(response.contains("\"plugin_registry_count\""));
}

#[tokio::test]
async fn plain_http_server_works_without_tls() {
    let bootstrap = AppBootstrap;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = BootstrapConfig::default();
    let server =
        tokio::spawn(async move { bootstrap.serve_with_listener(listener, &config).await });

    let response = request_health(addr).await;

    server.abort();
    let _ = server.await;

    assert!(response.starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn tls_settings_route_reports_disabled_by_default() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "tls-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let response = send_empty_request(&app, "GET", "/v1/settings/tls", "tls-token").await;
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["enabled"], false);
    assert!(json["cert_subject"].is_null());
    assert!(json["expires_at"].is_null());
}

#[tokio::test]
async fn readiness_metrics_and_version_routes_round_trip() {
    let app = AppBootstrap::router(BootstrapConfig::default())
        .await
        .unwrap();

    let ready_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ready_response.status(), StatusCode::OK);
    let ready_json = response_json(ready_response).await;
    assert_eq!(ready_json["ready"], true);

    let _health_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let metrics_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics_response.status(), StatusCode::OK);
    let metrics_body = to_bytes(metrics_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics_text = String::from_utf8(metrics_body.to_vec()).unwrap();
    assert!(metrics_text.contains("http_requests_total"));
    assert!(metrics_text.contains("active_runs_total"));
    assert!(metrics_text.contains("active_tasks_total"));

    let version_response = app
        .oneshot(
            Request::builder()
                .uri("/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(version_response.status(), StatusCode::OK);
    let version_json = response_json(version_response).await;
    assert_eq!(version_json["version"], env!("CARGO_PKG_VERSION"));
    assert!(version_json.get("git_sha").is_some());
    assert!(version_json.get("build_date").is_some());
}

#[tokio::test]
async fn dashboard_and_activity_routes_report_overview() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "dashboard-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "dashboard-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_dashboard_http"
        }),
    )
    .await;
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "dashboard-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_dashboard_http",
            "run_id": "run_dashboard_http"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let dashboard_response =
        send_empty_request(&app, "GET", "/v1/dashboard", "dashboard-token").await;
    assert_eq!(dashboard_response.status(), StatusCode::OK);
    let dashboard_json = response_json(dashboard_response).await;
    assert!(dashboard_json.get("active_runs").is_some());
    assert!(dashboard_json.get("active_tasks").is_some());
    assert!(dashboard_json.get("pending_approvals").is_some());
    assert!(dashboard_json.get("failed_runs_24h").is_some());
    assert!(dashboard_json.get("degraded_components").is_some());
    assert!(dashboard_json.get("recent_critical_events").is_some());
    assert!(dashboard_json.get("active_providers").is_some());
    assert!(dashboard_json.get("active_plugins").is_some());
    assert!(dashboard_json.get("memory_doc_count").is_some());
    assert!(dashboard_json.get("eval_runs_today").is_some());
    assert!(dashboard_json.get("system_healthy").is_some());
    assert!(dashboard_json["active_runs"].as_u64().unwrap() > 0);

    let activity_response =
        send_empty_request(&app, "GET", "/v1/dashboard/activity", "dashboard-token").await;
    assert_eq!(activity_response.status(), StatusCode::OK);
    let activity_json = response_json(activity_response).await;
    assert!(activity_json.is_array());
}

#[tokio::test]
async fn eval_and_graph_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_http"),
        },
    );

    for (eval_run_id, task_success_rate) in [("eval_http_1", 0.71), ("eval_http_2", 0.93)] {
        let create_response = send_json_request(
            &app,
            "POST",
            "/v1/evals/runs",
            "test-token",
            serde_json::json!({
                "tenant_id": "tenant_acme",
                "workspace_id": "ws_main",
                "project_id": "project_alpha",
                "eval_run_id": eval_run_id,
                "subject_kind": "prompt_release",
                "evaluator_type": "regression",
                "prompt_asset_id": "asset_eval_http",
                "prompt_version_id": format!("ver_{eval_run_id}"),
                "prompt_release_id": format!("rel_{eval_run_id}")
            }),
        )
        .await;
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let start_response = send_empty_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/start"),
            "test-token",
        )
        .await;
        assert_eq!(start_response.status(), StatusCode::OK);

        let score_response = send_json_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/score"),
            "test-token",
            serde_json::json!({
                "metrics": {
                    "task_success_rate": task_success_rate,
                    "latency_p50_ms": 120
                }
            }),
        )
        .await;
        assert_eq!(score_response.status(), StatusCode::OK);

        let complete_response = send_json_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/complete"),
            "test-token",
            serde_json::json!({
                "metrics": {
                    "task_success_rate": task_success_rate,
                    "latency_p50_ms": 120,
                    "cost_per_run": 0.004
                },
                "cost": 0.25
            }),
        )
        .await;
        assert_eq!(complete_response.status(), StatusCode::OK);
    }

    let list_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/runs?tenant_id=tenant_acme&workspace_id=ws_main&project_id=project_alpha",
        "test-token",
    )
    .await;
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_json = response_json(list_response).await;
    assert!(list_json["items"].as_array().unwrap().len() >= 2);

    let scorecard_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/scorecard/asset_eval_http?tenant_id=tenant_acme&workspace_id=ws_main&project_id=project_alpha",
        "test-token",
    )
    .await;
    assert_eq!(scorecard_response.status(), StatusCode::OK);
    let scorecard_json = response_json(scorecard_response).await;
    assert_eq!(scorecard_json["entries"][0]["eval_run_id"], "eval_http_2");
    assert_eq!(scorecard_json["entries"][1]["eval_run_id"], "eval_http_1");

    let compare_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/compare?run_ids=eval_http_1,eval_http_2",
        "test-token",
    )
    .await;
    assert_eq!(compare_response.status(), StatusCode::OK);
    let compare_json = response_json(compare_response).await;
    assert_eq!(compare_json["run_ids"][0], "eval_http_1");
    assert_eq!(compare_json["run_ids"][1], "eval_http_2");
}

#[tokio::test]
async fn eval_trend_and_winner_routes_return_best_run() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "eval-trend-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_http"),
        },
    );

    for (eval_run_id, prompt_version_id, prompt_release_id, task_success_rate) in [
        ("eval_trend_http_1", "ver_trend_1", "rel_trend_1", 0.7),
        ("eval_trend_http_2", "ver_trend_2", "rel_trend_2", 0.8),
        ("eval_trend_http_3", "ver_trend_3", "rel_trend_3", 0.9),
    ] {
        let create_response = send_json_request(
            &app,
            "POST",
            "/v1/evals/runs",
            "eval-trend-token",
            serde_json::json!({
                "tenant_id": "tenant_acme",
                "workspace_id": "ws_main",
                "project_id": "project_alpha",
                "eval_run_id": eval_run_id,
                "subject_kind": "prompt_release",
                "evaluator_type": "trend",
                "prompt_asset_id": "asset_eval_trend_http",
                "prompt_version_id": prompt_version_id,
                "prompt_release_id": prompt_release_id
            }),
        )
        .await;
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let start_response = send_empty_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/start"),
            "eval-trend-token",
        )
        .await;
        assert_eq!(start_response.status(), StatusCode::OK);

        let complete_response = send_json_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/complete"),
            "eval-trend-token",
            serde_json::json!({
                "metrics": {
                    "task_success_rate": task_success_rate
                },
                "cost": null
            }),
        )
        .await;
        assert_eq!(complete_response.status(), StatusCode::OK);
    }

    let trend_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/assets/asset_eval_trend_http/trend?tenant_id=tenant_acme&workspace_id=ws_main&project_id=project_alpha&metric=task_success_rate&days=30",
        "eval-trend-token",
    )
    .await;
    assert_eq!(trend_response.status(), StatusCode::OK);
    let trend_json = response_json(trend_response).await;
    let points = trend_json.as_array().unwrap();
    assert_eq!(points.len(), 3);
    assert_eq!(points[0]["eval_run_id"], "eval_trend_http_1");
    assert_eq!(points[1]["eval_run_id"], "eval_trend_http_2");
    assert_eq!(points[2]["eval_run_id"], "eval_trend_http_3");

    let winner_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/assets/asset_eval_trend_http/winner?tenant_id=tenant_acme&workspace_id=ws_main&project_id=project_alpha",
        "eval-trend-token",
    )
    .await;
    assert_eq!(winner_response.status(), StatusCode::OK);
    let winner_json = response_json(winner_response).await;
    assert_eq!(winner_json["eval_run_id"], "eval_trend_http_3");
    assert_eq!(winner_json["task_success_rate"], 0.9);
}

#[tokio::test]
async fn eval_matrix_routes_return_real_rows() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "eval-matrix-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    for (eval_run_id, prompt_version_id, prompt_release_id, task_success_rate) in [
        ("eval_matrix_http_1", "ver_matrix_1", "rel_matrix_1", 0.61),
        ("eval_matrix_http_2", "ver_matrix_2", "rel_matrix_2", 0.94),
    ] {
        let create_response = send_json_request(
            &app,
            "POST",
            "/v1/evals/runs",
            "eval-matrix-token",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "ws_main",
                "project_id": "project_alpha",
                "eval_run_id": eval_run_id,
                "subject_kind": "prompt_release",
                "evaluator_type": "matrix",
                "prompt_asset_id": "asset_eval_matrix_http",
                "prompt_version_id": prompt_version_id,
                "prompt_release_id": prompt_release_id
            }),
        )
        .await;
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let start_response = send_empty_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/start"),
            "eval-matrix-token",
        )
        .await;
        assert_eq!(start_response.status(), StatusCode::OK);

        let complete_response = send_json_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/complete"),
            "eval-matrix-token",
            serde_json::json!({
                "metrics": {
                    "task_success_rate": task_success_rate,
                    "latency_p50_ms": 111
                },
                "cost": null
            }),
        )
        .await;
        assert_eq!(complete_response.status(), StatusCode::OK);
    }

    let prompt_matrix_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/matrices/prompt-comparison?tenant_id=default_tenant&asset_id=asset_eval_matrix_http",
        "eval-matrix-token",
    )
    .await;
    assert_eq!(prompt_matrix_response.status(), StatusCode::OK);
    let prompt_matrix_json = response_json(prompt_matrix_response).await;
    let prompt_rows = prompt_matrix_json["rows"].as_array().unwrap();
    assert_eq!(prompt_rows.len(), 2);
    assert_eq!(prompt_rows[0]["eval_run_id"], "eval_matrix_http_1");
    assert_eq!(prompt_rows[0]["metrics"]["task_success_rate"], 0.61);
    assert_eq!(prompt_rows[1]["eval_run_id"], "eval_matrix_http_2");
    assert_eq!(prompt_rows[1]["metrics"]["task_success_rate"], 0.94);

    let create_policy_response = send_json_request(
        &app,
        "POST",
        "/v1/policies",
        "eval-matrix-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "name": "Matrix Guardrail",
            "rules": [{
                "subject_type": "tool",
                "subject_id": "git.status",
                "action": "invoke",
                "effect": "deny",
                "conditions": []
            }]
        }),
    )
    .await;
    assert_eq!(create_policy_response.status(), StatusCode::CREATED);

    let evaluate_policy_response = send_json_request(
        &app,
        "POST",
        "/v1/policies/evaluate",
        "eval-matrix-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "subject_type": "tool",
            "subject_id": "git.status",
            "action": "invoke"
        }),
    )
    .await;
    assert_eq!(evaluate_policy_response.status(), StatusCode::OK);

    let permission_matrix_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/matrices/permissions?tenant_id=default_tenant",
        "eval-matrix-token",
    )
    .await;
    assert_eq!(permission_matrix_response.status(), StatusCode::OK);
    let permission_matrix_json = response_json(permission_matrix_response).await;
    let permission_rows = permission_matrix_json["rows"].as_array().unwrap();
    assert!(!permission_rows.is_empty());
    assert_eq!(permission_rows[0]["capability"], "invoke");
    assert_eq!(permission_rows[0]["metrics"]["policy_pass_rate"], 0.0);
}

#[tokio::test]
async fn eval_report_and_export_routes_cover_improving_runs() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "eval-report-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_http"),
        },
    );

    for (eval_run_id, prompt_version_id, prompt_release_id, task_success_rate) in [
        ("eval_report_http_1", "ver_report_1", "rel_report_1", 0.6),
        ("eval_report_http_2", "ver_report_2", "rel_report_2", 0.7),
        ("eval_report_http_3", "ver_report_3", "rel_report_3", 0.8),
        ("eval_report_http_4", "ver_report_4", "rel_report_4", 0.9),
    ] {
        let create_response = send_json_request(
            &app,
            "POST",
            "/v1/evals/runs",
            "eval-report-token",
            serde_json::json!({
                "tenant_id": "tenant_acme",
                "workspace_id": "ws_main",
                "project_id": "project_alpha",
                "eval_run_id": eval_run_id,
                "subject_kind": "prompt_release",
                "evaluator_type": "report",
                "prompt_asset_id": "asset_eval_report_http",
                "prompt_version_id": prompt_version_id,
                "prompt_release_id": prompt_release_id
            }),
        )
        .await;
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let start_response = send_empty_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/start"),
            "eval-report-token",
        )
        .await;
        assert_eq!(start_response.status(), StatusCode::OK);

        let complete_response = send_json_request(
            &app,
            "POST",
            &format!("/v1/evals/runs/{eval_run_id}/complete"),
            "eval-report-token",
            serde_json::json!({
                "metrics": {
                    "task_success_rate": task_success_rate,
                    "latency_p50_ms": 120,
                    "cost_per_run": 0.01
                },
                "cost": null
            }),
        )
        .await;
        assert_eq!(complete_response.status(), StatusCode::OK);
    }

    let report_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/assets/asset_eval_report_http/report?tenant_id=tenant_acme&workspace_id=ws_main&project_id=project_alpha",
        "eval-report-token",
    )
    .await;
    assert_eq!(report_response.status(), StatusCode::OK);
    let report_json = response_json(report_response).await;
    assert_eq!(report_json["asset_id"], "asset_eval_report_http");
    assert_eq!(report_json["total_runs"], 4);
    assert_eq!(report_json["best_run_id"], "eval_report_http_4");
    assert_eq!(report_json["worst_run_id"], "eval_report_http_1");
    assert_eq!(report_json["trend_direction"], "improving");

    let export_response = send_empty_request(
        &app,
        "GET",
        "/v1/evals/assets/asset_eval_report_http/export?tenant_id=tenant_acme&workspace_id=ws_main&project_id=project_alpha&format=csv",
        "eval-report-token",
    )
    .await;
    assert_eq!(export_response.status(), StatusCode::OK);
    let headers = export_response.headers().clone();
    assert_eq!(headers.get("content-type").unwrap(), "text/csv");
    let body = to_bytes(export_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csv = String::from_utf8(body.to_vec()).unwrap();
    assert!(csv.starts_with(
        "eval_run_id,prompt_release_id,task_success_rate,latency_p50_ms,cost_per_run,completed_at"
    ));
    assert!(csv.contains("eval_report_http_4,rel_report_4,0.9,120,0.01,"));
}

#[tokio::test]
async fn run_triage_diagnose_and_release_lease_flow() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "run-triage-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "run-triage-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_run_triage_http"
        }),
    )
    .await;
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "run-triage-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_run_triage_http",
            "run_id": "run_triage_http"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let task_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks",
        "run-triage-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "task_id": "task_triage_http",
            "parent_run_id": "run_triage_http"
        }),
    )
    .await;
    assert_eq!(task_response.status(), StatusCode::CREATED);

    let claim_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks/task_triage_http/claim",
        "run-triage-token",
        serde_json::json!({
            "worker_id": "worker_triage_http",
            "lease_duration_ms": 1
        }),
    )
    .await;
    assert_eq!(claim_response.status(), StatusCode::OK);

    sleep(Duration::from_millis(10)).await;

    let stalled_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/stalled?minutes=0",
        "run-triage-token",
    )
    .await;
    assert_eq!(stalled_response.status(), StatusCode::OK);
    let stalled_json = response_json(stalled_response).await;
    assert!(stalled_json["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["run_id"] == "run_triage_http"));

    let diagnose_response = send_empty_request(
        &app,
        "POST",
        "/v1/runs/run_triage_http/diagnose",
        "run-triage-token",
    )
    .await;
    assert_eq!(diagnose_response.status(), StatusCode::OK);
    let diagnose_json = response_json(diagnose_response).await;
    assert_eq!(diagnose_json["run_id"], "run_triage_http");
    assert_eq!(diagnose_json["suggested_action"], "release_leases");
    assert!(diagnose_json["stalled_tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|task_id| task_id == "task_triage_http"));

    let release_response = send_empty_request(
        &app,
        "POST",
        "/v1/tasks/task_triage_http/release-lease",
        "run-triage-token",
    )
    .await;
    assert_eq!(release_response.status(), StatusCode::OK);
    let release_json = response_json(release_response).await;
    assert_eq!(release_json["state"], "queued");
    assert!(release_json["lease_owner"].is_null());

    let task_get_response = send_empty_request(
        &app,
        "GET",
        "/v1/tasks/task_triage_http",
        "run-triage-token",
    )
    .await;
    assert_eq!(task_get_response.status(), StatusCode::OK);
    let task_get_json = response_json(task_get_response).await;
    assert_eq!(task_get_json["state"], "queued");
    assert!(task_get_json["lease_owner"].is_null());
}

#[tokio::test]
async fn onboarding_templates_and_settings_routes_return_200() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_http"),
        },
    );

    let templates_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/onboarding/templates")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(templates_response.status(), StatusCode::OK);
    let templates_body = to_bytes(templates_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let templates: Vec<serde_json::Value> = serde_json::from_slice(&templates_body).unwrap();
    assert!(!templates.is_empty());

    let settings_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/settings")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(settings_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn bundle_import_export_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "bundle-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let bundle = valid_bundle_body();

    let validate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/bundles/validate")
                .header("authorization", "Bearer bundle-token")
                .header("content-type", "application/json")
                .body(Body::from(bundle.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(validate_response.status(), StatusCode::OK);
    let validate_body = to_bytes(validate_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let validate_json: serde_json::Value = serde_json::from_slice(&validate_body).unwrap();
    assert_eq!(validate_json["errors"], serde_json::json!([]));

    let apply_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/bundles/apply")
                .header("authorization", "Bearer bundle-token")
                .header("content-type", "application/json")
                .body(Body::from(bundle.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(apply_response.status(), StatusCode::OK);
    let apply_body = to_bytes(apply_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let apply_json: serde_json::Value = serde_json::from_slice(&apply_body).unwrap();
    assert_eq!(apply_json["create_count"], serde_json::json!(2));

    let second_apply_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/bundles/apply")
                .header("authorization", "Bearer bundle-token")
                .header("content-type", "application/json")
                .body(Body::from(bundle.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_apply_response.status(), StatusCode::OK);
    let second_apply_body = to_bytes(second_apply_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let second_apply_json: serde_json::Value = serde_json::from_slice(&second_apply_body).unwrap();
    assert_eq!(second_apply_json["skip_count"], serde_json::json!(2));

    let export_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/bundles/export?project=acme/eng/support&source_ids=bundle_http_curated")
                .header("authorization", "Bearer bundle-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export_response.status(), StatusCode::OK);
    let export_body = to_bytes(export_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let export_json: serde_json::Value = serde_json::from_slice(&export_body).unwrap();
    assert_eq!(
        export_json["bundle_type"],
        serde_json::json!("curated_knowledge_pack_bundle")
    );
    assert_eq!(export_json["artifact_count"], serde_json::json!(2));
}

#[tokio::test]
async fn sessions_and_runs_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_http"),
        },
    );

    let create_session_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "tenant_http",
                        "workspace_id": "workspace_http",
                        "project_id": "project_http",
                        "session_id": "session_http_1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_session_response.status(), StatusCode::CREATED);

    let create_run_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "tenant_http",
                        "workspace_id": "workspace_http",
                        "project_id": "project_http",
                        "session_id": "session_http_1",
                        "run_id": "run_http_1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_run_response.status(), StatusCode::CREATED);

    let get_run_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/runs/run_http_1")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    if get_run_response.status() != StatusCode::OK {
        let s = get_run_response.status();
        let b = to_bytes(get_run_response.into_body(), usize::MAX)
            .await
            .unwrap();
        panic!(
            "GET /v1/runs/run_http_1 returned {} body={}",
            s,
            String::from_utf8_lossy(&b)
        );
    }

    let get_run_body = to_bytes(get_run_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&get_run_body).unwrap();
    assert_eq!(json["run"]["run_id"], "run_http_1");
    assert!(json["tasks"].as_array().is_some());
}

#[tokio::test]
async fn run_replay_routes_return_events_and_final_state() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "run-replay-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_replay_http"),
        },
    );

    let create_session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "run-replay-token",
        serde_json::json!({
            "tenant_id": "tenant_replay_http",
            "workspace_id": "workspace_replay_http",
            "project_id": "project_replay_http",
            "session_id": "session_replay_http_1"
        }),
    )
    .await;
    assert_eq!(create_session_response.status(), StatusCode::CREATED);

    let create_run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "run-replay-token",
        serde_json::json!({
            "tenant_id": "tenant_replay_http",
            "workspace_id": "workspace_replay_http",
            "project_id": "project_replay_http",
            "session_id": "session_replay_http_1",
            "run_id": "run_replay_http_1"
        }),
    )
    .await;
    assert_eq!(create_run_response.status(), StatusCode::CREATED);

    let task_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks",
        "run-replay-token",
        serde_json::json!({
            "tenant_id": "tenant_replay_http",
            "workspace_id": "workspace_replay_http",
            "project_id": "project_replay_http",
            "task_id": "task_replay_http_1",
            "parent_run_id": "run_replay_http_1"
        }),
    )
    .await;
    assert_eq!(task_response.status(), StatusCode::CREATED);

    let claim_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks/task_replay_http_1/claim",
        "run-replay-token",
        serde_json::json!({
            "worker_id": "worker_replay_http_1"
        }),
    )
    .await;
    assert_eq!(claim_response.status(), StatusCode::OK);

    let complete_response = send_empty_request(
        &app,
        "POST",
        "/v1/tasks/task_replay_http_1/complete",
        "run-replay-token",
    )
    .await;
    assert_eq!(complete_response.status(), StatusCode::OK);

    let events_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/run_replay_http_1/events?from=1&limit=50",
        "run-replay-token",
    )
    .await;
    assert_eq!(events_response.status(), StatusCode::OK);
    let events_json = response_json(events_response).await;
    let items = events_json.as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item["event_type"] == "run_created"));

    let replay_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/run_replay_http_1/replay",
        "run-replay-token",
    )
    .await;
    assert_eq!(replay_response.status(), StatusCode::OK);
    let replay_json = response_json(replay_response).await;
    assert_eq!(replay_json["final_run_state"], "completed");
    assert!(replay_json["events_replayed"].as_u64().unwrap() >= 3);
    assert!(replay_json["final_task_states"]
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["task_id"] == "task_replay_http_1" && task["state"] == "completed"));
}

#[tokio::test]
async fn pause_scheduled_run_resumes_through_http() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "pause-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_pause_http"),
        },
    );

    let create_session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "pause-token",
        serde_json::json!({
            "tenant_id": "tenant_pause_http",
            "workspace_id": "workspace_pause_http",
            "project_id": "project_pause_http",
            "session_id": "session_pause_http"
        }),
    )
    .await;
    assert_eq!(create_session_response.status(), StatusCode::CREATED);

    let create_run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "pause-token",
        serde_json::json!({
            "tenant_id": "tenant_pause_http",
            "workspace_id": "workspace_pause_http",
            "project_id": "project_pause_http",
            "session_id": "session_pause_http",
            "run_id": "run_pause_http"
        }),
    )
    .await;
    assert_eq!(create_run_response.status(), StatusCode::CREATED);

    let pause_response = send_json_request(
        &app,
        "POST",
        "/v1/runs/run_pause_http/pause",
        "pause-token",
        serde_json::json!({
            "reason_kind": "operator_pause",
            "actor": "op_pause_http",
            "detail": "waiting for review",
            "resume_after_ms": 50u64
        }),
    )
    .await;
    assert_eq!(pause_response.status(), StatusCode::OK);

    let paused_run_response =
        send_empty_request(&app, "GET", "/v1/runs/run_pause_http", "pause-token").await;
    assert_eq!(paused_run_response.status(), StatusCode::OK);
    let paused_run_json = response_json(paused_run_response).await;
    assert_eq!(paused_run_json["run"]["state"], "paused");
    assert_eq!(
        paused_run_json["run"]["pause_reason"]["kind"],
        "operator_pause"
    );
    assert_eq!(
        paused_run_json["run"]["pause_reason"]["actor"],
        "op_pause_http"
    );
    assert_eq!(
        paused_run_json["run"]["pause_reason"]["resume_after_ms"],
        50u64
    );

    sleep(Duration::from_millis(100)).await;

    let due_response = send_empty_request(&app, "GET", "/v1/runs/resume-due", "pause-token").await;
    assert_eq!(due_response.status(), StatusCode::OK);
    let due_json = response_json(due_response).await;
    assert_eq!(due_json["items"].as_array().unwrap().len(), 1);
    assert_eq!(due_json["items"][0]["run_id"], "run_pause_http");

    let process_response = send_json_request(
        &app,
        "POST",
        "/v1/runs/process-scheduled-resumes",
        "pause-token",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(process_response.status(), StatusCode::OK);
    let process_json = response_json(process_response).await;
    assert_eq!(process_json["resumedCount"], 1);

    let resumed_run_response =
        send_empty_request(&app, "GET", "/v1/runs/run_pause_http", "pause-token").await;
    assert_eq!(resumed_run_response.status(), StatusCode::OK);
    let resumed_run_json = response_json(resumed_run_response).await;
    assert_eq!(resumed_run_json["run"]["state"], "running");
}

#[tokio::test]
async fn intervention_force_fail_records_intervention() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "intervention-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_intervention_http"),
        },
    );

    let create_session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "intervention-token",
        serde_json::json!({
            "tenant_id": "tenant_intervention_http",
            "workspace_id": "workspace_intervention_http",
            "project_id": "project_intervention_http",
            "session_id": "session_intervention_http"
        }),
    )
    .await;
    assert_eq!(create_session_response.status(), StatusCode::CREATED);

    let create_run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "intervention-token",
        serde_json::json!({
            "tenant_id": "tenant_intervention_http",
            "workspace_id": "workspace_intervention_http",
            "project_id": "project_intervention_http",
            "session_id": "session_intervention_http",
            "run_id": "run_intervention_http"
        }),
    )
    .await;
    assert_eq!(create_run_response.status(), StatusCode::CREATED);

    let intervene_response = send_json_request(
        &app,
        "POST",
        "/v1/runs/run_intervention_http/intervene",
        "intervention-token",
        serde_json::json!({
            "action": "force_fail",
            "reason": "operator stop due to degraded execution"
        }),
    )
    .await;
    assert_eq!(intervene_response.status(), StatusCode::OK);
    let intervene_json = response_json(intervene_response).await;
    assert_eq!(intervene_json["run"]["state"], "failed");

    let interventions_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/run_intervention_http/interventions",
        "intervention-token",
    )
    .await;
    assert_eq!(interventions_response.status(), StatusCode::OK);
    let interventions_json = response_json(interventions_response).await;
    let items = interventions_json["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["run_id"], "run_intervention_http");
    assert_eq!(items[0]["action"], "force_fail");
    assert_eq!(
        items[0]["reason"],
        "operator stop due to degraded execution"
    );
}

#[tokio::test]
async fn tenant_isolation_hides_other_tenant_runs() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "tenant-a-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );
    tokens.register(
        "tenant-b-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_b_http"),
        },
    );

    let tenant_response = send_json_request(
        &app,
        "POST",
        "/v1/admin/tenants",
        "tenant-a-token",
        serde_json::json!({
            "tenant_id": "tenant_b_http",
            "name": "Tenant B HTTP"
        }),
    )
    .await;
    assert_eq!(tenant_response.status(), StatusCode::CREATED);

    let workspace_response = send_json_request(
        &app,
        "POST",
        "/v1/admin/tenants/tenant_b_http/workspaces",
        "tenant-a-token",
        serde_json::json!({
            "workspace_id": "workspace_b_http",
            "name": "Workspace B HTTP"
        }),
    )
    .await;
    assert_eq!(workspace_response.status(), StatusCode::CREATED);

    let project_response = send_json_request(
        &app,
        "POST",
        "/v1/admin/workspaces/workspace_b_http/projects",
        "tenant-a-token",
        serde_json::json!({
            "project_id": "project_b_http",
            "name": "Project B HTTP"
        }),
    )
    .await;
    assert_eq!(project_response.status(), StatusCode::CREATED);

    let create_session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "tenant-a-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_isolation_a"
        }),
    )
    .await;
    assert_eq!(create_session_response.status(), StatusCode::CREATED);

    let create_run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "tenant-a-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_isolation_a",
            "run_id": "run_isolation_a"
        }),
    )
    .await;
    assert_eq!(create_run_response.status(), StatusCode::CREATED);

    let other_tenant_list = send_empty_request(
        &app,
        "GET",
        "/v1/runs?tenant_id=tenant_b_http&workspace_id=workspace_b_http&project_id=project_b_http",
        "tenant-b-token",
    )
    .await;
    assert_eq!(other_tenant_list.status(), StatusCode::OK);
    let other_tenant_json = response_json(other_tenant_list).await;
    assert_eq!(other_tenant_json["items"], serde_json::json!([]));

    let mismatch_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        "tenant-b-token",
    )
    .await;
    assert_eq!(mismatch_response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn signals_feed_and_worker_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let signal_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/signals")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project",
                        "signal_id": "signal_http_1",
                        "source": "webhook",
                        "payload": { "kind": "deploy_pending" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(signal_response.status(), StatusCode::CREATED);

    let feed_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/feed?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(feed_response.status(), StatusCode::OK);

    let worker_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/workers/register")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "worker_id": "worker_http_1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(worker_response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn mailbox_and_recovery_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_mailbox_http"
        }),
    )
    .await;
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_mailbox_http",
            "run_id": "run_mailbox_http"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let append_response = send_json_request(
        &app,
        "POST",
        "/v1/mailbox",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "message_id": "mailbox_http_1",
            "run_id": "run_mailbox_http",
            "sender_id": "operator_http",
            "body": "Need operator review"
        }),
    )
    .await;
    assert_eq!(append_response.status(), StatusCode::CREATED);
    let append_json = response_json(append_response).await;
    assert_eq!(append_json["messageId"], "mailbox_http_1");
    assert_eq!(append_json["body"], "Need operator review");

    let list_response = send_empty_request(
        &app,
        "GET",
        "/v1/mailbox?run_id=run_mailbox_http",
        "test-token",
    )
    .await;
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_json = response_json(list_response).await;
    assert_eq!(list_json["items"][0]["messageId"], "mailbox_http_1");

    let delivered_response =
        send_empty_request(&app, "DELETE", "/v1/mailbox/mailbox_http_1", "test-token").await;
    assert_eq!(delivered_response.status(), StatusCode::OK);

    let recover_response = send_empty_request(
        &app,
        "POST",
        "/v1/runs/run_mailbox_http/recover",
        "test-token",
    )
    .await;
    // Fabric finalization: /v1/runs/:id/recover is a 202 deprecation stub.
    // FF's background scanners own recovery unconditionally; the endpoint
    // is kept for backwards-compatibility only. See CAIRN-FABRIC-FINALIZED.md §3.5.
    assert_eq!(recover_response.status(), StatusCode::ACCEPTED);

    let status_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/run_mailbox_http/recovery-status",
        "test-token",
    )
    .await;
    assert_eq!(status_response.status(), StatusCode::OK);
    let status_json = response_json(status_response).await;
    assert_eq!(status_json["runId"], "run_mailbox_http");
}

#[tokio::test]
async fn session_cost_route_accumulates_provider_call_totals() {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_cost_http"
        }),
    )
    .await;
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_cost_http",
            "run_id": "run_cost_http"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let project = ProjectKey::new("default_tenant", "default_workspace", "default_project");
    runtime
        .store
        .append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_http_provider_call_1"),
                EventSource::Runtime,
                RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                    project: project.clone(),
                    provider_call_id: ProviderCallId::new("pc_http_1"),
                    route_decision_id: RouteDecisionId::new("rd_http_1"),
                    route_attempt_id: RouteAttemptId::new("ra_http_1"),
                    provider_binding_id: ProviderBindingId::new("binding_http"),
                    provider_connection_id: ProviderConnectionId::new("conn_http"),
                    provider_model_id: ProviderModelId::new("model_http"),
                    run_id: Some(RunId::new("run_cost_http")),
                    session_id: None,
                    operation_kind: cairn_domain::providers::OperationKind::Generate,
                    status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                    latency_ms: Some(5),
                    input_tokens: Some(10),
                    output_tokens: Some(4),
                    cost_micros: Some(1_000),
                    error_class: None,
                    raw_error_message: None,
                    retry_count: 0,
                    task_id: None,
                    prompt_release_id: None,
                    fallback_position: 0,
                    started_at: 0,
                    finished_at: 0,
                    completed_at: 50,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_http_provider_call_2"),
                EventSource::Runtime,
                RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                    project,
                    provider_call_id: ProviderCallId::new("pc_http_2"),
                    route_decision_id: RouteDecisionId::new("rd_http_2"),
                    route_attempt_id: RouteAttemptId::new("ra_http_2"),
                    provider_binding_id: ProviderBindingId::new("binding_http"),
                    provider_connection_id: ProviderConnectionId::new("conn_http"),
                    provider_model_id: ProviderModelId::new("model_http"),
                    run_id: Some(RunId::new("run_cost_http")),
                    session_id: None,
                    operation_kind: cairn_domain::providers::OperationKind::Generate,
                    status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                    latency_ms: Some(6),
                    input_tokens: Some(6),
                    output_tokens: Some(3),
                    cost_micros: Some(2_000),
                    error_class: None,
                    raw_error_message: None,
                    retry_count: 0,
                    task_id: None,
                    prompt_release_id: None,
                    fallback_position: 0,
                    started_at: 0,
                    finished_at: 0,
                    completed_at: 60,
                }),
            ),
        ])
        .await
        .unwrap();

    let response = send_empty_request(
        &app,
        "GET",
        "/v1/sessions/session_cost_http/cost",
        "test-token",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["total_cost_micros"], 3000);
    assert_eq!(json["provider_calls"], 2);
    assert_eq!(json["run_breakdown"][0]["run_id"], "run_cost_http");
    assert_eq!(json["run_breakdown"][0]["total_cost_micros"], 3000);
}

#[tokio::test]
async fn run_cost_route_accumulates_provider_call_totals() {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "run_cost_http_session"
        }),
    )
    .await;
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "run_cost_http_session",
            "run_id": "run_cost_http_route"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let project = ProjectKey::new("default_tenant", "default_workspace", "default_project");
    runtime
        .store
        .append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_http_run_cost_1"),
                EventSource::Runtime,
                RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                    project: project.clone(),
                    provider_call_id: ProviderCallId::new("pc_run_http_1"),
                    route_decision_id: RouteDecisionId::new("rd_run_http_1"),
                    route_attempt_id: RouteAttemptId::new("ra_run_http_1"),
                    provider_binding_id: ProviderBindingId::new("binding_http"),
                    provider_connection_id: ProviderConnectionId::new("conn_http"),
                    provider_model_id: ProviderModelId::new("model_http"),
                    run_id: Some(RunId::new("run_cost_http_route")),
                    session_id: None,
                    operation_kind: cairn_domain::providers::OperationKind::Generate,
                    status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                    latency_ms: Some(5),
                    input_tokens: Some(10),
                    output_tokens: Some(4),
                    cost_micros: Some(1_000),
                    error_class: None,
                    raw_error_message: None,
                    retry_count: 0,
                    task_id: None,
                    prompt_release_id: None,
                    fallback_position: 0,
                    started_at: 0,
                    finished_at: 0,
                    completed_at: 50,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_http_run_cost_2"),
                EventSource::Runtime,
                RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                    project,
                    provider_call_id: ProviderCallId::new("pc_run_http_2"),
                    route_decision_id: RouteDecisionId::new("rd_run_http_2"),
                    route_attempt_id: RouteAttemptId::new("ra_run_http_2"),
                    provider_binding_id: ProviderBindingId::new("binding_http"),
                    provider_connection_id: ProviderConnectionId::new("conn_http"),
                    provider_model_id: ProviderModelId::new("model_http"),
                    run_id: Some(RunId::new("run_cost_http_route")),
                    session_id: None,
                    operation_kind: cairn_domain::providers::OperationKind::Generate,
                    status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                    latency_ms: Some(6),
                    input_tokens: Some(6),
                    output_tokens: Some(3),
                    cost_micros: Some(2_000),
                    error_class: None,
                    raw_error_message: None,
                    retry_count: 0,
                    task_id: None,
                    prompt_release_id: None,
                    fallback_position: 0,
                    started_at: 0,
                    finished_at: 0,
                    completed_at: 60,
                }),
            ),
        ])
        .await
        .unwrap();

    let response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/run_cost_http_route/cost",
        "test-token",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["run_id"], "run_cost_http_route");
    assert_eq!(json["total_cost_micros"], 3000);
    assert_eq!(json["provider_calls"], 2);
}

#[tokio::test]
async fn channel_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "channel-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let create_response = send_json_request(
        &app,
        "POST",
        "/v1/channels",
        "channel-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "name": "ops",
            "capacity": 8
        }),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let create_json = response_json(create_response).await;
    let channel_id = create_json["channel_id"].as_str().unwrap().to_owned();

    for body in ["one", "two", "three"] {
        let send_response = send_json_request(
            &app,
            "POST",
            &format!("/v1/channels/{channel_id}/send"),
            "channel-token",
            serde_json::json!({
                "sender_id": "alice",
                "body": body
            }),
        )
        .await;
        assert_eq!(send_response.status(), StatusCode::OK);
    }

    for expected_body in ["one", "two"] {
        let consume_response = send_json_request(
            &app,
            "POST",
            &format!("/v1/channels/{channel_id}/consume"),
            "channel-token",
            serde_json::json!({
                "consumer_id": "bob"
            }),
        )
        .await;
        assert_eq!(consume_response.status(), StatusCode::OK);
        let consume_json = response_json(consume_response).await;
        assert_eq!(consume_json["body"], expected_body);
    }

    let messages_response = send_empty_request(
        &app,
        "GET",
        &format!("/v1/channels/{channel_id}/messages?limit=10"),
        "channel-token",
    )
    .await;
    assert_eq!(messages_response.status(), StatusCode::OK);
    let messages_json = response_json(messages_response).await;
    let messages = messages_json.as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["consumed_at_ms"].is_null())
            .count(),
        1
    );
}

#[tokio::test]
async fn prompt_assets_and_approvals_routes_round_trip() {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_e2e_http"),
        },
    );

    let project = ProjectKey::new("default_tenant", "default_workspace", "default_project");
    runtime
        .approvals
        .request(
            &project,
            ApprovalId::new("approval_http_1"),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    let asset_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/prompts/assets")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "prompt_asset_id": "asset_http_1",
                        "name": "HTTP Prompt Asset",
                        "kind": "system"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(asset_response.status(), StatusCode::CREATED);

    let approvals_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/approvals")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approvals_response.status(), StatusCode::OK);

    let approve_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/approvals/approval_http_1/approve")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approve_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn audit_log_records_prompt_release_activation() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "audit-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/prompts/assets",
            "audit-token",
            serde_json::json!({
                "prompt_asset_id": "asset_audit_http",
                "name": "Audit Prompt Asset",
                "kind": "system"
            }),
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/prompts/assets/asset_audit_http/versions",
            "audit-token",
            serde_json::json!({
                "prompt_version_id": "version_audit_http",
                "content_hash": "hash_audit_http",
                "content": "audit activation content"
            }),
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/prompts/releases",
            "audit-token",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "prompt_release_id": "release_audit_http",
                "prompt_asset_id": "asset_audit_http",
                "prompt_version_id": "version_audit_http"
            }),
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    for to_state in ["proposed", "approved"] {
        assert_eq!(
            send_json_request(
                &app,
                "POST",
                "/v1/prompts/releases/release_audit_http/transition",
                "audit-token",
                serde_json::json!({ "to_state": to_state }),
            )
            .await
            .status(),
            StatusCode::OK
        );
    }

    let activate_response = send_empty_request(
        &app,
        "POST",
        "/v1/prompts/releases/release_audit_http/activate",
        "audit-token",
    )
    .await;
    assert_eq!(activate_response.status(), StatusCode::OK);

    let audit_response =
        send_empty_request(&app, "GET", "/v1/admin/audit-log?limit=20", "audit-token").await;
    assert_eq!(audit_response.status(), StatusCode::OK);
    let audit_json = response_json(audit_response).await;
    let items = audit_json["items"].as_array().unwrap();
    assert!(items.iter().any(|entry| {
        entry["action"] == "activate_prompt_release"
            && entry["resource_type"] == "prompt_release"
            && entry["resource_id"] == "release_audit_http"
            && entry["outcome"] == "success"
    }));

    let resource_response = send_empty_request(
        &app,
        "GET",
        "/v1/admin/audit-log/prompt_release/release_audit_http",
        "audit-token",
    )
    .await;
    assert_eq!(resource_response.status(), StatusCode::OK);
    let resource_json = response_json(resource_response).await;
    assert_eq!(resource_json["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn prompt_compare_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "prompt-compare-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let asset_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets",
        "prompt-compare-token",
        serde_json::json!({
            "prompt_asset_id": "asset_compare_1",
            "name": "Prompt Compare Asset",
            "kind": "system"
        }),
    )
    .await;
    assert_eq!(asset_response.status(), StatusCode::CREATED);

    let version_a_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets/asset_compare_1/versions",
        "prompt-compare-token",
        serde_json::json!({
            "prompt_version_id": "version_compare_a",
            "content_hash": "hash_compare_a",
            "content": "line one\nline shared\nline old"
        }),
    )
    .await;
    assert_eq!(version_a_response.status(), StatusCode::CREATED);

    let version_b_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets/asset_compare_1/versions",
        "prompt-compare-token",
        serde_json::json!({
            "prompt_version_id": "version_compare_b",
            "content_hash": "hash_compare_b",
            "content": "line one\nline shared\nline new"
        }),
    )
    .await;
    assert_eq!(version_b_response.status(), StatusCode::CREATED);

    let release_a_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/releases",
        "prompt-compare-token",
        serde_json::json!({
            "prompt_release_id": "release_compare_a",
            "prompt_asset_id": "asset_compare_1",
            "prompt_version_id": "version_compare_a"
        }),
    )
    .await;
    assert_eq!(release_a_response.status(), StatusCode::CREATED);

    let release_b_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/releases",
        "prompt-compare-token",
        serde_json::json!({
            "prompt_release_id": "release_compare_b",
            "prompt_asset_id": "asset_compare_1",
            "prompt_version_id": "version_compare_b"
        }),
    )
    .await;
    assert_eq!(release_b_response.status(), StatusCode::CREATED);

    let transition_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/releases/release_compare_a/transition",
        "prompt-compare-token",
        serde_json::json!({
            "to_state": "proposed"
        }),
    )
    .await;
    assert_eq!(transition_response.status(), StatusCode::OK);

    let history_response = send_empty_request(
        &app,
        "GET",
        "/v1/prompts/releases/release_compare_a/history",
        "prompt-compare-token",
    )
    .await;
    assert_eq!(history_response.status(), StatusCode::OK);
    let history_json = response_json(history_response).await;
    assert_eq!(history_json.as_array().unwrap().len(), 1);
    assert_eq!(history_json[0]["from_state"], "draft");
    assert_eq!(history_json[0]["to_state"], "proposed");

    let diff_response = send_empty_request(
        &app,
        "GET",
        "/v1/prompts/assets/asset_compare_1/versions/version_compare_a/diff?compare_to=version_compare_b",
        "prompt-compare-token",
    )
    .await;
    assert_eq!(diff_response.status(), StatusCode::OK);
    let diff_json = response_json(diff_response).await;
    assert!(diff_json["similarity_score"].as_f64().is_some());
    assert!(!diff_json["added_lines"].as_array().unwrap().is_empty());

    let compare_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/releases/compare",
        "prompt-compare-token",
        serde_json::json!({
            "release_ids": ["release_compare_a", "release_compare_b"]
        }),
    )
    .await;
    assert_eq!(compare_response.status(), StatusCode::OK);
    let compare_json = response_json(compare_response).await;
    let releases = compare_json["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 2);
    assert!(releases
        .iter()
        .any(|entry| entry["release_id"] == "release_compare_a"));
    assert!(releases
        .iter()
        .any(|entry| entry["release_id"] == "release_compare_b"));
}

#[tokio::test]
async fn prompt_template_routes_render_and_validate_required_vars() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "prompt-template-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let asset_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets",
        "prompt-template-token",
        serde_json::json!({
            "prompt_asset_id": "asset_template_1",
            "name": "Prompt Template Asset",
            "kind": "user_template"
        }),
    )
    .await;
    assert_eq!(asset_response.status(), StatusCode::CREATED);

    let version_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets/asset_template_1/versions",
        "prompt-template-token",
        serde_json::json!({
            "prompt_version_id": "version_template_1",
            "content_hash": "hash_template_1",
            "content": "Hello {{name}}, you are {{role}}",
            "template_vars": [
                {
                    "name": "name",
                    "required": true
                },
                {
                    "name": "role",
                    "required": true,
                    "default_value": "user"
                }
            ]
        }),
    )
    .await;
    assert_eq!(version_response.status(), StatusCode::CREATED);

    let vars_response = send_empty_request(
        &app,
        "GET",
        "/v1/prompts/assets/asset_template_1/versions/version_template_1/template-vars",
        "prompt-template-token",
    )
    .await;
    assert_eq!(vars_response.status(), StatusCode::OK);
    let vars_json = response_json(vars_response).await;
    assert_eq!(vars_json.as_array().unwrap().len(), 2);
    assert_eq!(vars_json[1]["default_value"], "user");

    let render_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets/asset_template_1/versions/version_template_1/render",
        "prompt-template-token",
        serde_json::json!({
            "vars": {
                "name": "Alice"
            }
        }),
    )
    .await;
    assert_eq!(render_response.status(), StatusCode::OK);
    let render_json = response_json(render_response).await;
    assert_eq!(render_json["content"], "Hello Alice, you are user");

    let missing_required_response = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets/asset_template_1/versions/version_template_1/render",
        "prompt-template-token",
        serde_json::json!({
            "vars": {}
        }),
    )
    .await;
    assert_eq!(
        missing_required_response.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn task_dependency_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "task-dependency-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    for task_id in ["task_dep_a", "task_dep_b"] {
        let response = send_json_request(
            &app,
            "POST",
            "/v1/tasks",
            "task-dependency-token",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "task_id": task_id
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let add_dep_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks/task_dep_b/dependencies",
        "task-dependency-token",
        serde_json::json!({
            "depends_on_task_id": "task_dep_a"
        }),
    )
    .await;
    assert_eq!(add_dep_response.status(), StatusCode::CREATED);

    let deps_before = send_empty_request(
        &app,
        "GET",
        "/v1/tasks/task_dep_b/dependencies",
        "task-dependency-token",
    )
    .await;
    assert_eq!(deps_before.status(), StatusCode::OK);
    let deps_before_json = response_json(deps_before).await;
    assert_eq!(deps_before_json.as_array().unwrap().len(), 1);
    assert!(deps_before_json[0]["resolved_at_ms"].is_null());

    let claim = send_json_request(
        &app,
        "POST",
        "/v1/tasks/task_dep_a/claim",
        "task-dependency-token",
        serde_json::json!({
            "worker_id": "worker-a",
            "lease_duration_ms": 60000
        }),
    )
    .await;
    assert_eq!(claim.status(), StatusCode::OK);

    let complete = send_empty_request(
        &app,
        "POST",
        "/v1/tasks/task_dep_a/complete",
        "task-dependency-token",
    )
    .await;
    assert_eq!(complete.status(), StatusCode::OK);

    let deps_after = send_empty_request(
        &app,
        "GET",
        "/v1/tasks/task_dep_b/dependencies",
        "task-dependency-token",
    )
    .await;
    assert_eq!(deps_after.status(), StatusCode::OK);
    let deps_after_json = response_json(deps_after).await;
    assert_eq!(deps_after_json.as_array().unwrap().len(), 1);
    assert!(deps_after_json[0]["resolved_at_ms"].as_u64().is_some());
}

#[tokio::test]
async fn auto_checkpoint_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "auto-checkpoint-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "auto-checkpoint-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_auto_cp_http"
        }),
    )
    .await;
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "auto-checkpoint-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_auto_cp_http",
            "run_id": "run_auto_cp_http"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let set_strategy_response = send_json_request(
        &app,
        "POST",
        "/v1/runs/run_auto_cp_http/checkpoint-strategy",
        "auto-checkpoint-token",
        serde_json::json!({
            "strategy_id": "strategy_auto_cp_http",
            "interval_ms": 60000,
            "max_checkpoints": 5,
            "trigger_on_task_complete": true
        }),
    )
    .await;
    assert_eq!(set_strategy_response.status(), StatusCode::OK);

    let get_strategy_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/run_auto_cp_http/checkpoint-strategy",
        "auto-checkpoint-token",
    )
    .await;
    assert_eq!(get_strategy_response.status(), StatusCode::OK);
    let strategy_json = response_json(get_strategy_response).await;
    assert_eq!(strategy_json["strategy_id"], "strategy_auto_cp_http");
    assert_eq!(strategy_json["trigger_on_task_complete"], true);

    let task_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks",
        "auto-checkpoint-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "task_id": "task_auto_cp_http",
            "parent_run_id": "run_auto_cp_http"
        }),
    )
    .await;
    assert_eq!(task_response.status(), StatusCode::CREATED);

    let claim_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks/task_auto_cp_http/claim",
        "auto-checkpoint-token",
        serde_json::json!({
            "worker_id": "worker-auto-cp",
            "lease_duration_ms": 60000
        }),
    )
    .await;
    assert_eq!(claim_response.status(), StatusCode::OK);

    let complete_response = send_empty_request(
        &app,
        "POST",
        "/v1/tasks/task_auto_cp_http/complete",
        "auto-checkpoint-token",
    )
    .await;
    assert_eq!(complete_response.status(), StatusCode::OK);

    let checkpoints_response = send_empty_request(
        &app,
        "GET",
        "/v1/checkpoints?run_id=run_auto_cp_http",
        "auto-checkpoint-token",
    )
    .await;
    assert_eq!(checkpoints_response.status(), StatusCode::OK);
    let checkpoints_json = response_json(checkpoints_response).await;
    let items = checkpoints_json["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["checkpoint_id"]
        .as_str()
        .unwrap()
        .starts_with("cp_auto_run_auto_cp_http_task_auto_cp_http_"));
}

#[tokio::test]
async fn protected_route_requires_bearer_token() {
    let app = AppBootstrap::router(BootstrapConfig::default())
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_accepts_registered_bearer_token() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "auth-ok-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/settings")
                .header("authorization", "Bearer auth-ok-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn nonexistent_run_returns_canonical_api_error() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "error-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let response = send_empty_request(&app, "GET", "/v1/runs/nonexistent-id", "error-token").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "not_found");
    assert!(json["message"].as_str().unwrap().contains("run"));
    assert!(json.get("request_id").is_some());
}

#[tokio::test]
async fn malformed_run_create_returns_validation_api_error() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "error-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs")
                .header("authorization", "Bearer error-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "validation_error");
    assert!(json["message"].as_str().unwrap().contains("session_id"));
}

#[tokio::test]
async fn unknown_path_returns_canonical_not_found_api_error() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "error-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let response = send_empty_request(&app, "GET", "/v1/unknown-path", "error-token").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "not_found");
    assert_eq!(json["message"], "route not found");
    assert!(json.get("request_id").is_some());
}

#[tokio::test]
async fn deep_search_and_graph_provenance_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "memory-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let ingest_response = send_json_request(
        &app,
        "POST",
        "/v1/memory/ingest",
        "memory-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "source_id": "source_deep_http",
            "document_id": "doc_deep_http",
            "content": "Cairn deep search helps operators inspect retrieval hops and provenance chains.",
            "source_type": "plain_text"
        }),
    )
    .await;
    assert_eq!(ingest_response.status(), StatusCode::OK);

    let deep_search_response = send_json_request(
        &app,
        "POST",
        "/v1/memory/deep-search",
        "memory-token",
        serde_json::json!({
            "project": {
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project"
            },
            "query_text": "deep search provenance",
            "max_hops": 2,
            "per_hop_limit": 3
        }),
    )
    .await;
    assert_eq!(deep_search_response.status(), StatusCode::OK);
    let deep_search_json = response_json(deep_search_response).await;
    assert!(deep_search_json["hops"].is_array());
    assert!(!deep_search_json["hops"].as_array().unwrap().is_empty());

    let provenance_response = send_empty_request(
        &app,
        "GET",
        "/v1/graph/provenance/doc_deep_http",
        "memory-token",
    )
    .await;
    assert_eq!(provenance_response.status(), StatusCode::OK);
    let provenance_json = response_json(provenance_response).await;
    assert!(provenance_json.is_array());
    assert!(provenance_json
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["node_id"] == "doc_deep_http"));
}

#[tokio::test]
async fn memory_graph_expansion_finds_related_documents_and_route() {
    let (app, _runtime, graph, tokens) =
        AppBootstrap::router_with_runtime_graph_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "memory-graph-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let seed_ingest = send_json_request(
        &app,
        "POST",
        "/v1/memory/ingest",
        "memory-graph-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "source_id": "source_graph_http",
            "document_id": "doc_graph_seed_http_1",
            "content": "alpha anchor evidence for graph-backed memory expansion",
            "source_type": "plain_text"
        }),
    )
    .await;
    assert_eq!(seed_ingest.status(), StatusCode::OK);

    let related_ingest = send_json_request(
        &app,
        "POST",
        "/v1/memory/ingest",
        "memory-graph-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "source_id": "source_graph_http",
            "document_id": "doc_graph_related_http_2",
            "content": "doc_graph_related_http_2 carries adjacent graph-only retrieval evidence",
            "source_type": "plain_text"
        }),
    )
    .await;
    assert_eq!(related_ingest.status(), StatusCode::OK);

    graph
        .add_edge(GraphEdge {
            source_node_id: "doc_graph_related_http_2".to_owned(),
            target_node_id: "doc_graph_seed_http_1".to_owned(),
            kind: EdgeKind::Spawned,
            created_at: 1,
            confidence: None,
        })
        .await
        .unwrap();

    let deep_search_response = send_json_request(
        &app,
        "POST",
        "/v1/memory/deep-search",
        "memory-graph-token",
        serde_json::json!({
            "project": {
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project"
            },
            "query_text": "alpha",
            "max_hops": 2,
            "per_hop_limit": 3
        }),
    )
    .await;
    assert_eq!(deep_search_response.status(), StatusCode::OK);
    let deep_search_json = response_json(deep_search_response).await;
    let merged_results = deep_search_json["merged_results"].as_array().unwrap();
    assert!(merged_results
        .iter()
        .any(|result| result["chunk"]["document_id"] == "doc_graph_seed_http_1"));
    assert!(merged_results
        .iter()
        .any(|result| result["chunk"]["document_id"] == "doc_graph_related_http_2"));

    let related_response = send_empty_request(
        &app,
        "GET",
        "/v1/memory/related/doc_graph_seed_http_1",
        "memory-graph-token",
    )
    .await;
    assert_eq!(related_response.status(), StatusCode::OK);
    let related_json = response_json(related_response).await;
    let related_items = related_json.as_array().unwrap();
    assert!(related_items.iter().any(|item| {
        item["id"] == "doc_graph_related_http_2"
            && item["source"]
                .as_str()
                .is_some_and(|relationship| relationship != "source_graph_http")
    }));
}

#[tokio::test]
async fn ingest_job_and_source_chunk_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "ingest-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let start_response = send_json_request(
        &app,
        "POST",
        "/v1/ingest/jobs",
        "ingest-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "job_id": "job_http_ingest_1",
            "source_id": "source_http_ingest_1",
            "document_id": "doc_http_ingest_1",
            "content": "Operators can inspect ongoing ingest jobs and source chunks.",
            "source_type": "plain_text"
        }),
    )
    .await;
    assert_eq!(start_response.status(), StatusCode::CREATED);
    let start_json = response_json(start_response).await;
    assert_eq!(start_json["id"], "job_http_ingest_1");

    let get_response = send_empty_request(
        &app,
        "GET",
        "/v1/ingest/jobs/job_http_ingest_1",
        "ingest-token",
    )
    .await;
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_json = response_json(get_response).await;
    assert_eq!(get_json["id"], "job_http_ingest_1");
    assert_eq!(get_json["state"], "processing");

    let complete_response = send_json_request(
        &app,
        "POST",
        "/v1/ingest/jobs/job_http_ingest_1/complete",
        "ingest-token",
        serde_json::json!({
            "success": true,
            "error_message": null
        }),
    )
    .await;
    assert_eq!(complete_response.status(), StatusCode::OK);
    let complete_json = response_json(complete_response).await;
    assert_eq!(complete_json["state"], "completed");

    let source_response = send_empty_request(
        &app,
        "GET",
        "/v1/sources/source_http_ingest_1?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        "ingest-token",
    )
    .await;
    assert_eq!(source_response.status(), StatusCode::OK);
    let source_json = response_json(source_response).await;
    assert_eq!(source_json["source_id"], "source_http_ingest_1");

    let chunks_response = send_empty_request(
        &app,
        "GET",
        "/v1/sources/source_http_ingest_1/chunks?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        "ingest-token",
    )
    .await;
    assert_eq!(chunks_response.status(), StatusCode::OK);
    let chunks_json = response_json(chunks_response).await;
    assert!(chunks_json["items"].is_array());
    assert!(!chunks_json["items"].as_array().unwrap().is_empty());
    assert!(chunks_json["items"][0]["text_preview"]
        .as_str()
        .unwrap()
        .contains("Operators"));
}

#[tokio::test]
async fn runtime_stream_emits_frame_after_run_creation() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "sse-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let create_session_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .header("authorization", "Bearer sse-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project",
                        "session_id": "session_sse_1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_session_response.status(), StatusCode::CREATED);

    let stream_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/streams/runtime")
                .header("authorization", "Bearer sse-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream_response.status(), StatusCode::OK);

    let stream_body = stream_response.into_body();
    let read_stream = tokio::spawn(async move {
        use http_body_util::BodyExt as _;
        let mut stream_body = stream_body;
        loop {
            let frame = stream_body.frame().await?;
            let Ok(frame) = frame else {
                return None;
            };
            if let Ok(data) = frame.into_data() {
                let text = String::from_utf8_lossy(data.as_ref()).to_string();
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
        }
    });

    let create_run_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs")
                .header("authorization", "Bearer sse-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project",
                        "session_id": "session_sse_1",
                        "run_id": "run_sse_1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_run_response.status(), StatusCode::CREATED);

    let frame_text = timeout(Duration::from_millis(500), read_stream)
        .await
        .expect("timed out waiting for runtime SSE frame")
        .expect("stream task panicked")
        .expect("stream ended before yielding a frame");
    assert!(frame_text.contains("run_sse_1"), "{frame_text}");
}

#[tokio::test]
async fn memory_and_provider_routes_round_trip() {
    let config = BootstrapConfig {
        mode: cairn_api::bootstrap::DeploymentMode::SelfHostedTeam,
        ..BootstrapConfig::default()
    };
    let (app, _runtime, tokens) = AppBootstrap::router_with_runtime_and_tokens(config)
        .await
        .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let ingest_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/memory/ingest")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project",
                        "source_id": "src_http_docs",
                        "document_id": "doc_http_1",
                        "content": "Provider routing and owned retrieval are core Cairn features.",
                        "source_type": "plain_text"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ingest_response.status(), StatusCode::OK);

    let search_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/memory/search?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project&query_text=owned%20retrieval&limit=5")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(search_response.status(), StatusCode::OK);

    let connection_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/providers/connections")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "provider_connection_id": "conn_http_openai",
                        "provider_family": "openai",
                        "adapter_type": "responses_api"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(connection_response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn memory_feedback_adjusts_source_quality_and_scores() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "memory-feedback-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let ingest_response = send_json_request(
        &app,
        "POST",
        "/v1/memory/ingest",
        "memory-feedback-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "source_id": "source_feedback_a",
            "document_id": "doc_feedback_a",
            "content": "feedback ranking source alpha carries reliable operator guidance",
            "source_type": "plain_text"
        }),
    )
    .await;
    assert_eq!(ingest_response.status(), StatusCode::OK);

    let baseline_response = send_empty_request(
        &app,
        "GET",
        "/v1/memory/search?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project&query_text=feedback%20ranking%20source&limit=5",
        "memory-feedback-token",
    )
    .await;
    assert_eq!(baseline_response.status(), StatusCode::OK);
    let baseline_json = response_json(baseline_response).await;
    let baseline_score = baseline_json["results"][0]["score"].as_f64().unwrap();
    let chunk_id = baseline_json["results"][0]["chunk"]["chunk_id"]
        .as_str()
        .unwrap()
        .to_owned();

    for _ in 0..3 {
        let feedback_response = send_json_request(
            &app,
            "POST",
            "/v1/memory/feedback",
            "memory-feedback-token",
            serde_json::json!({
                "chunk_id": chunk_id,
                "source_id": "source_feedback_a",
                "was_used": true,
                "rating": 5.0
            }),
        )
        .await;
        assert_eq!(feedback_response.status(), StatusCode::OK);
    }

    let quality_positive_response = send_empty_request(
        &app,
        "GET",
        "/v1/sources/source_feedback_a/quality",
        "memory-feedback-token",
    )
    .await;
    assert_eq!(quality_positive_response.status(), StatusCode::OK);
    let quality_positive_json = response_json(quality_positive_response).await;
    assert_eq!(quality_positive_json["chunk_count"], 1);
    assert_eq!(quality_positive_json["avg_rating"].as_f64().unwrap(), 5.0);

    let boosted_response = send_empty_request(
        &app,
        "GET",
        "/v1/memory/search?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project&query_text=feedback%20ranking%20source&limit=5",
        "memory-feedback-token",
    )
    .await;
    assert_eq!(boosted_response.status(), StatusCode::OK);
    let boosted_json = response_json(boosted_response).await;
    let boosted_score = boosted_json["results"][0]["score"].as_f64().unwrap();
    assert!(boosted_score > baseline_score);

    for _ in 0..3 {
        let feedback_response = send_json_request(
            &app,
            "POST",
            "/v1/memory/feedback",
            "memory-feedback-token",
            serde_json::json!({
                "chunk_id": chunk_id,
                "source_id": "source_feedback_a",
                "was_used": false,
                "rating": 1.0
            }),
        )
        .await;
        assert_eq!(feedback_response.status(), StatusCode::OK);
    }

    let quality_negative_response = send_empty_request(
        &app,
        "GET",
        "/v1/sources/source_feedback_a/quality",
        "memory-feedback-token",
    )
    .await;
    assert_eq!(quality_negative_response.status(), StatusCode::OK);
    let quality_negative_json = response_json(quality_negative_response).await;
    assert!(
        quality_negative_json["credibility_score"].as_f64().unwrap()
            < quality_positive_json["credibility_score"].as_f64().unwrap()
    );
    assert!(quality_negative_json["total_retrievals"].as_u64().unwrap() >= 2);

    let lowered_response = send_empty_request(
        &app,
        "GET",
        "/v1/memory/search?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project&query_text=feedback%20ranking%20source&limit=5",
        "memory-feedback-token",
    )
    .await;
    assert_eq!(lowered_response.status(), StatusCode::OK);
    let lowered_json = response_json(lowered_response).await;
    let lowered_score = lowered_json["results"][0]["score"].as_f64().unwrap();
    assert!(lowered_score < boosted_score);
}

#[tokio::test]
async fn admin_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let tenant_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/tenants")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "tenant_admin_http",
                        "name": "Admin Tenant"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tenant_response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn tool_invocation_checkpoint_and_plugin_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let session_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project",
                        "session_id": "session_http_tools"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project",
                        "session_id": "session_http_tools",
                        "run_id": "run_http_tools"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let tool_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tool-invocations")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "tenant_id": "default_tenant",
                        "workspace_id": "default_workspace",
                        "project_id": "default_project",
                        "invocation_id": "inv_http_1",
                        "session_id": "session_http_tools",
                        "run_id": "run_http_tools",
                        "target": {
                            "target_type": "builtin",
                            "tool_name": "git.status"
                        },
                        "execution_class": "supervised_process"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tool_response.status(), StatusCode::CREATED);

    let checkpoint_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs/run_http_tools/checkpoint")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "checkpoint_id": "cp_http_1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(checkpoint_response.status(), StatusCode::CREATED);

    let plugins_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/plugins")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(plugins_response.status(), StatusCode::OK);
    let plugins_body = to_bytes(plugins_response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&plugins_body).unwrap()["items"],
        serde_json::json!([])
    );

    let register_plugin_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plugins")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "id": "com.example.http-plugin",
                        "name": "HTTP Plugin",
                        "version": "0.1.0",
                        "command": ["echo", "ready"],
                        "capabilities": [{
                            "type": "tool_provider",
                            "tools": ["http.test"]
                        }],
                        "permissions": {
                            "permissions": ["fs_read"]
                        },
                        "limits": null,
                        "execution_class": "supervised_process"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(register_plugin_response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn tool_invocation_progress_route_returns_latest_progress() {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let tool_response = send_json_request(
        &app,
        "POST",
        "/v1/tool-invocations",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "invocation_id": "inv_http_progress",
            "target": {
                "target_type": "builtin",
                "tool_name": "git.status"
            },
            "execution_class": "supervised_process"
        }),
    )
    .await;
    assert_eq!(tool_response.status(), StatusCode::CREATED);

    runtime
        .store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_http_progress_1"),
            EventSource::Runtime,
            RuntimeEvent::ToolInvocationProgressUpdated(ToolInvocationProgressUpdated {
                invocation_id: ToolInvocationId::new("inv_http_progress"),
                progress_pct: 42,
                message: Some("warming cache".to_owned()),
                updated_at_ms: 1_710_000_000,
            }),
        )])
        .await
        .unwrap();

    let progress_response = send_empty_request(
        &app,
        "GET",
        "/v1/tool-invocations/inv_http_progress/progress",
        "test-token",
    )
    .await;
    assert_eq!(progress_response.status(), StatusCode::OK);
    let progress_json = response_json(progress_response).await;
    assert_eq!(progress_json["percent"], 42.5);
    assert_eq!(progress_json["message"], "warming cache");
}

fn write_eval_scorer_plugin_script() -> PathBuf {
    let path = std::env::temp_dir().join(format!("cairn_eval_plugin_{}.py", uuid::Uuid::new_v4()));
    let script = r#"
import json
import sys

for line in sys.stdin:
    if not line.strip():
        continue
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        response = {
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "protocolVersion": "1.0",
                "plugin": {
                    "id": "com.example.eval-scorer",
                    "name": "Eval Scorer",
                    "version": "0.1.0"
                },
                "capabilities": [{"type": "eval_scorer"}],
                "limits": None
            }
        }
    elif method == "eval.score":
        samples = request["params"].get("samples", [])
        sample = samples[0] if samples else {}
        expected = sample.get("expected")
        actual = request["params"].get("target", {}).get("actual")
        matched = expected == actual
        response = {
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "score": 1.0 if matched else 0.0,
                "passed": matched,
                "feedback": "exact match" if matched else "mismatch"
            }
        }
    elif method == "shutdown":
        response = {
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {}
        }
    else:
        response = {
            "jsonrpc": "2.0",
            "id": request["id"],
            "error": {"code": -32601, "message": f"unknown method: {method}"}
        }

    sys.stdout.write(json.dumps(response) + "\n")
    sys.stdout.flush()
"#;
    fs::write(&path, script).unwrap();
    path
}

#[tokio::test]
async fn plugin_eval_score_route_returns_exact_match_score() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let script_path = write_eval_scorer_plugin_script();
    let register_plugin_response = send_json_request(
        &app,
        "POST",
        "/v1/plugins",
        "test-token",
        serde_json::json!({
            "id": "com.example.eval-scorer",
            "name": "Eval Scorer",
            "version": "0.1.0",
            "command": ["python3", script_path.display().to_string()],
            "capabilities": [{
                "type": "eval_scorer"
            }],
            "permissions": {
                "permissions": []
            },
            "limits": null,
            "execution_class": "supervised_process"
        }),
    )
    .await;
    assert_eq!(register_plugin_response.status(), StatusCode::CREATED);

    let score_response = send_json_request(
        &app,
        "POST",
        "/v1/plugins/com.example.eval-scorer/eval-score",
        "test-token",
        serde_json::json!({
            "input": { "prompt": "hello" },
            "expected": { "answer": "world" },
            "actual": { "answer": "world" }
        }),
    )
    .await;
    assert_eq!(score_response.status(), StatusCode::OK);
    let score_json = response_json(score_response).await;
    assert_eq!(score_json["score"], serde_json::json!(1.0));
    assert_eq!(score_json["passed"], serde_json::json!(true));

    let _ = fs::remove_file(script_path);
}

#[tokio::test]
async fn full_workspace_operator_journey_over_http() {
    let config = BootstrapConfig {
        mode: cairn_api::bootstrap::DeploymentMode::SelfHostedTeam,
        ..BootstrapConfig::default()
    };
    let (app, _runtime, tokens) = AppBootstrap::router_with_runtime_and_tokens(config)
        .await
        .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_e2e_http"),
        },
    );

    let tenant_response = send_json_request(
        &app,
        "POST",
        "/v1/admin/tenants",
        "test-token",
        serde_json::json!({
            "tenant_id": "tenant_e2e_http",
            "name": "Tenant E2E HTTP"
        }),
    )
    .await;
    assert_eq!(tenant_response.status(), StatusCode::CREATED);

    let workspace_response = send_json_request(
        &app,
        "POST",
        "/v1/admin/tenants/tenant_e2e_http/workspaces",
        "test-token",
        serde_json::json!({
            "workspace_id": "workspace_e2e_http",
            "name": "Workspace E2E HTTP"
        }),
    )
    .await;
    assert_eq!(workspace_response.status(), StatusCode::CREATED);

    let project_response = send_json_request(
        &app,
        "POST",
        "/v1/admin/workspaces/workspace_e2e_http/projects",
        "test-token",
        serde_json::json!({
            "project_id": "project_e2e_http",
            "name": "Project E2E HTTP"
        }),
    )
    .await;
    assert_eq!(project_response.status(), StatusCode::CREATED);

    let connection_response = send_json_request(
        &app,
        "POST",
        "/v1/providers/connections",
        "test-token",
        serde_json::json!({
            "tenant_id": "tenant_e2e_http",
            "provider_connection_id": "conn_e2e_http",
            "provider_family": "openai",
            "adapter_type": "responses_api"
        }),
    )
    .await;
    assert_eq!(connection_response.status(), StatusCode::CREATED);

    let binding_response = send_json_request(
        &app,
        "POST",
        "/v1/providers/bindings",
        "test-token",
        serde_json::json!({
            "tenant_id": "tenant_e2e_http",
            "workspace_id": "workspace_e2e_http",
            "project_id": "project_e2e_http",
            "provider_connection_id": "conn_e2e_http",
            "operation_kind": "generate",
            "provider_model_id": "model_x"
        }),
    )
    .await;
    assert_eq!(binding_response.status(), StatusCode::CREATED);

    let route_policy_response = send_json_request(
        &app,
        "POST",
        "/v1/providers/policies",
        "test-token",
        serde_json::json!({
            "tenant_id": "tenant_e2e_http",
            "name": "Prefer model_x",
            "rules": [{
                "capability": "generate",
                "preferred_model_ids": ["model_x"],
                "fallback_model_ids": ["model_y"],
                "max_cost_micros": 1000,
                "require_provider_ids": []
            }]
        }),
    )
    .await;
    assert_eq!(route_policy_response.status(), StatusCode::CREATED);

    let session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "test-token",
        serde_json::json!({
            "tenant_id": "tenant_e2e_http",
            "workspace_id": "workspace_e2e_http",
            "project_id": "project_e2e_http",
            "session_id": "session_e2e_http_1"
        }),
    )
    .await;
    assert_eq!(session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "test-token",
        serde_json::json!({
            "tenant_id": "tenant_e2e_http",
            "workspace_id": "workspace_e2e_http",
            "project_id": "project_e2e_http",
            "session_id": "session_e2e_http_1",
            "run_id": "run_e2e_http_1"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let task_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks",
        "test-token",
        serde_json::json!({
            "tenant_id": "tenant_e2e_http",
            "workspace_id": "workspace_e2e_http",
            "project_id": "project_e2e_http",
            "task_id": "task_e2e_http_1",
            "parent_run_id": "run_e2e_http_1"
        }),
    )
    .await;
    assert_eq!(task_response.status(), StatusCode::CREATED);

    let claim_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks/task_e2e_http_1/claim",
        "test-token",
        serde_json::json!({
            "worker_id": "worker_e2e_http_1"
        }),
    )
    .await;
    assert_eq!(claim_response.status(), StatusCode::OK);

    let heartbeat_response = send_json_request(
        &app,
        "POST",
        "/v1/tasks/task_e2e_http_1/heartbeat",
        "test-token",
        serde_json::json!({
            "worker_id": "worker_e2e_http_1"
        }),
    )
    .await;
    assert_eq!(heartbeat_response.status(), StatusCode::OK);

    let complete_response = send_empty_request(
        &app,
        "POST",
        "/v1/tasks/task_e2e_http_1/complete",
        "test-token",
    )
    .await;
    assert_eq!(complete_response.status(), StatusCode::OK);

    let run_detail_response =
        send_empty_request(&app, "GET", "/v1/runs/run_e2e_http_1", "test-token").await;
    assert_eq!(run_detail_response.status(), StatusCode::OK);
    let run_detail_json = response_json(run_detail_response).await;
    assert_eq!(run_detail_json["run"]["run_id"], "run_e2e_http_1");
    assert_eq!(run_detail_json["run"]["state"], "completed");
    assert_eq!(run_detail_json["tasks"][0]["task_id"], "task_e2e_http_1");
    assert_eq!(run_detail_json["tasks"][0]["state"], "completed");
}

#[tokio::test]
async fn middleware_adds_request_id_limits_body_and_enables_local_cors() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let health_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health_response.status(), StatusCode::OK);
    assert!(health_response.headers().contains_key("x-request-id"));
    assert_eq!(
        health_response
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "*"
    );

    let oversized_body = format!("{{\"blob\":\"{}\"}}", "a".repeat(10 * 1024 * 1024 + 1));
    let too_large_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plugins")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(oversized_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(too_large_response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn openapi_json_route_returns_documented_paths() {
    let app = AppBootstrap::router(BootstrapConfig::default())
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let openapi = json["openapi"].as_str().unwrap_or_default();
    assert!(openapi.starts_with("3.0."));
    assert!(
        json["paths"]
            .as_object()
            .map(|paths| paths.len())
            .unwrap_or(0)
            >= 5
    );
}

#[tokio::test]
async fn run_subagent_spawn_and_children_routes_round_trip() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "subagent-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let parent_session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "subagent-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_parent_http"
        }),
    )
    .await;
    assert_eq!(parent_session_response.status(), StatusCode::CREATED);

    let child_session_response = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "subagent-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_child_http"
        }),
    )
    .await;
    assert_eq!(child_session_response.status(), StatusCode::CREATED);

    let run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "subagent-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_parent_http",
            "run_id": "run_parent_http"
        }),
    )
    .await;
    assert_eq!(run_response.status(), StatusCode::CREATED);

    let spawn_response = send_json_request(
        &app,
        "POST",
        "/v1/runs/run_parent_http/spawn",
        "subagent-token",
        serde_json::json!({
            "session_id": "sess_child_http",
            "parent_task_id": "task_parent_http",
            "child_task_id": "task_child_http",
            "child_run_id": "run_child_http"
        }),
    )
    .await;
    assert_eq!(spawn_response.status(), StatusCode::CREATED);
    let spawn_json = response_json(spawn_response).await;
    assert_eq!(spawn_json["parent_run_id"], "run_parent_http");
    assert_eq!(spawn_json["child_run_id"], "run_child_http");

    let child_run_response =
        send_empty_request(&app, "GET", "/v1/runs/run_child_http", "subagent-token").await;
    assert_eq!(child_run_response.status(), StatusCode::OK);
    let child_run_json = response_json(child_run_response).await;
    assert_eq!(child_run_json["run"]["run_id"], "run_child_http");
    assert_eq!(child_run_json["run"]["parent_run_id"], "run_parent_http");

    let children_response = send_empty_request(
        &app,
        "GET",
        "/v1/runs/run_parent_http/children",
        "subagent-token",
    )
    .await;
    assert_eq!(children_response.status(), StatusCode::OK);
    let children_json = response_json(children_response).await;
    let child_items = children_json["items"].as_array().unwrap();
    assert_eq!(child_items.len(), 1);
    assert_eq!(child_items[0]["run_id"], "run_child_http");

    let graph_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/graph/dependency-path/run_parent_http?max_depth=3")
                .header("authorization", "Bearer subagent-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(graph_response.status(), StatusCode::OK);
    let graph_json = response_json(graph_response).await;
    assert!(graph_json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["node_id"] == "run_child_http"));
    assert!(graph_json["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|edge| edge["source_node_id"] == "run_parent_http"
            && edge["target_node_id"] == "run_child_http"
            && edge["kind"] == "spawned"));
}

async fn send_json_request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: serde_json::Value,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn send_empty_request(app: &axum::Router, method: &str, uri: &str, token: &str) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn response_json(response: Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn request_health(addr: std::net::SocketAddr) -> String {
    for _ in 0..20 {
        match TcpStream::connect(addr).await {
            Ok(mut stream) => {
                stream
                    .write_all(
                        b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                return response;
            }
            Err(_) => sleep(Duration::from_millis(25)).await,
        }
    }

    panic!("server did not accept a /health connection in time");
}

// ── GAP-010: LLM Observability traces route ───────────────────────────────────

#[tokio::test]
async fn llm_traces_route_returns_traces_for_session() {
    let (app, runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    // Create session.
    let sess_resp = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_traces"
        }),
    )
    .await;
    assert_eq!(sess_resp.status(), StatusCode::CREATED);

    // Create a run so we have a run_id for the provider call.
    let run_resp = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_traces",
            "run_id": "run_traces"
        }),
    )
    .await;
    assert_eq!(run_resp.status(), StatusCode::CREATED);

    // Append a ProviderCallCompleted event — this is projected into LlmCallTrace.
    let project = ProjectKey::new("default_tenant", "default_workspace", "default_project");
    runtime
        .store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_traces_1"),
            EventSource::Runtime,
            RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
                project,
                provider_call_id: ProviderCallId::new("pc_traces_1"),
                route_decision_id: RouteDecisionId::new("rd_traces_1"),
                route_attempt_id: RouteAttemptId::new("ra_traces_1"),
                provider_binding_id: ProviderBindingId::new("binding_traces"),
                provider_connection_id: ProviderConnectionId::new("conn_traces"),
                provider_model_id: ProviderModelId::new("claude-sonnet-4-6"),
                session_id: Some(cairn_domain::SessionId::new("sess_traces")),
                run_id: Some(RunId::new("run_traces")),
                operation_kind: cairn_domain::providers::OperationKind::Generate,
                status: cairn_domain::providers::ProviderCallStatus::Succeeded,
                latency_ms: Some(350),
                input_tokens: Some(200),
                output_tokens: Some(80),
                cost_micros: Some(2_100),
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                task_id: None,
                prompt_release_id: None,
                fallback_position: 0,
                started_at: 0,
                finished_at: 0,
                completed_at: 9000,
            }),
        )])
        .await
        .unwrap();

    // GET /v1/sessions/sess_traces/llm-traces
    let response = send_empty_request(
        &app,
        "GET",
        "/v1/sessions/sess_traces/llm-traces",
        "test-token",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;

    // The traces array must contain one entry matching the appended call.
    let traces = json["traces"].as_array().expect("traces must be an array");
    assert_eq!(traces.len(), 1, "exactly one trace expected");

    let trace = &traces[0];
    assert_eq!(trace["trace_id"], "pc_traces_1");
    assert_eq!(trace["model_id"], "claude-sonnet-4-6");
    assert_eq!(trace["latency_ms"], 350);
    assert_eq!(trace["prompt_tokens"], 200);
    assert_eq!(trace["completion_tokens"], 80);
    assert_eq!(trace["cost_micros"], 2100);
}

#[tokio::test]
async fn llm_traces_route_returns_empty_for_session_with_no_calls() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    // Create a session but make no provider calls.
    let sess_resp = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        "test-token",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "sess_no_traces"
        }),
    )
    .await;
    assert_eq!(sess_resp.status(), StatusCode::CREATED);

    let response = send_empty_request(
        &app,
        "GET",
        "/v1/sessions/sess_no_traces/llm-traces",
        "test-token",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let traces = json["traces"].as_array().expect("traces must be array");
    assert!(
        traces.is_empty(),
        "no traces for session with no provider calls"
    );
}

#[tokio::test]
async fn llm_traces_route_returns_404_for_unknown_session() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );

    let response = send_empty_request(
        &app,
        "GET",
        "/v1/sessions/nonexistent_session/llm-traces",
        "test-token",
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn minimal_route_test() {
    let (app, _runtime, tokens) =
        AppBootstrap::router_with_runtime_and_tokens(BootstrapConfig::default())
            .await
            .unwrap();
    tokens.register(
        "test-token".to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("tenant_test"),
        },
    );

    // Test a simple GET to /health (no auth needed)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "health check should work");

    // Test GET /v1/runs/{id}
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/runs/some_run_id")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    eprintln!(
        "GET /v1/runs/some_run_id: {} body={}",
        status,
        String::from_utf8_lossy(&body)
    );
    // We expect 404 from the handler (run not found), NOT from the router
    assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "route should exist");
}
