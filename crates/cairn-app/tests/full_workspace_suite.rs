#![cfg(feature = "in-memory-runtime")]

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    response::Response,
};
use cairn_api::auth::AuthPrincipal;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::AppBootstrap;
use cairn_domain::tenancy::TenantKey;
use cairn_domain::OperatorId;
use tower::ServiceExt;

const TOKEN: &str = "full-workspace-token";

fn three_doc_bundle(
    tenant_id: &str,
    workspace_id: &str,
    project_id: &str,
    bundle_id: &str,
) -> serde_json::Value {
    let scope = serde_json::json!({
        "tenant_id": tenant_id,
        "workspace_id": workspace_id,
        "project_id": project_id
    });

    let artifact = |logical_id: &str, name: &str, hash: &str, ts: u64, text: &str, tag: &str| {
        serde_json::json!({
            "artifact_kind": "knowledge_document",
            "artifact_logical_id": logical_id,
            "artifact_display_name": name,
            "origin_scope": scope,
            "origin_artifact_id": null,
            "content_hash": hash,
            "source_bundle_id": bundle_id,
            "origin_timestamp": ts,
            "metadata": {},
            "payload": {
                "knowledge_pack_logical_id": bundle_id,
                "document_name": name,
                "source_type": "text_plain",
                "content": {
                    "kind": "inline_text",
                    "text": text
                },
                "metadata": {},
                "chunk_hints": [],
                "retrieval_hints": [tag]
            },
            "lineage": null,
            "tags": [tag]
        })
    };

    serde_json::json!({
        "bundle_schema_version": "1",
        "bundle_type": "curated_knowledge_pack_bundle",
        "bundle_id": bundle_id,
        "bundle_name": "Full Workspace Bundle",
        "created_at": 1_710_000_100u64,
        "created_by": "full_workspace_suite",
        "source_deployment_id": null,
        "source_scope": scope,
        "artifact_count": 3,
        "artifacts": [
            artifact(
                "doc_full_1",
                "Install Guide",
                "hash_full_install",
                1_710_000_101u64,
                "Install Cairn with cargo install cairn-cli and run setup.",
                "install"
            ),
            artifact(
                "doc_full_2",
                "Password Reset",
                "hash_full_reset",
                1_710_000_102u64,
                "Reset the password from the operator portal using the magic reset phrase.",
                "support"
            ),
            artifact(
                "doc_full_3",
                "Graph Provenance",
                "hash_full_graph",
                1_710_000_103u64,
                "Graph provenance follows route decisions and tool results end to end.",
                "graph"
            )
        ],
        "provenance": {
            "description": "full workspace suite bundle",
            "source_system": "integration_test",
            "export_reason": "round_trip"
        }
    })
}

async fn app_with_token() -> axum::Router {
    let config = BootstrapConfig {
        mode: cairn_api::bootstrap::DeploymentMode::SelfHostedTeam,
        ..BootstrapConfig::default()
    };
    let (app, _runtime, tokens) = AppBootstrap::router_with_runtime_and_tokens(config)
        .await
        .unwrap();
    tokens.register(
        TOKEN.to_string(),
        AuthPrincipal::Operator {
            operator_id: OperatorId::new("test_op"),
            tenant: TenantKey::new("default_tenant"),
        },
    );
    app
}

async fn send_json_request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn send_empty_request(app: &axum::Router, method: &str, uri: &str) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {TOKEN}"))
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

#[tokio::test]
async fn full_workspace_complete_operator_setup() {
    let app = app_with_token().await;

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/admin/tenants",
            serde_json::json!({
                "tenant_id": "tenant_full_setup",
                "name": "Tenant Full Setup"
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
            "/v1/admin/tenants/tenant_full_setup/workspaces",
            serde_json::json!({
                "workspace_id": "workspace_full_setup",
                "name": "Workspace Full Setup"
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
            "/v1/admin/workspaces/workspace_full_setup/projects",
            serde_json::json!({
                "project_id": "project_full_setup",
                "name": "Project Full Setup"
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
            "/v1/admin/tenants/tenant_full_setup/operator-profiles",
            serde_json::json!({
                "display_name": "Alex Operator",
                "email": "alex@example.com",
                "role": "owner"
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
            "/v1/providers/connections",
            serde_json::json!({
                "tenant_id": "tenant_full_setup",
                "provider_connection_id": "conn_full_setup",
                "provider_family": "openai",
                "adapter_type": "responses_api"
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
            "/v1/providers/bindings",
            serde_json::json!({
                "tenant_id": "tenant_full_setup",
                "workspace_id": "workspace_full_setup",
                "project_id": "project_full_setup",
                "provider_connection_id": "conn_full_setup",
                "operation_kind": "generate",
                "provider_model_id": "gpt-4"
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
            "/v1/providers/policies",
            serde_json::json!({
                "tenant_id": "tenant_full_setup",
                "name": "Prefer GPT-4",
                "rules": [{
                    "capability": "generate",
                    "preferred_model_ids": ["gpt-4"],
                    "fallback_model_ids": ["gpt-4o-mini"],
                    "max_cost_micros": 5_000_000u64,
                    "require_provider_ids": []
                }]
            }),
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    let bindings = response_json(
        send_empty_request(
            &app,
            "GET",
            "/v1/providers/bindings?tenant_id=tenant_full_setup",
        )
        .await,
    )
    .await;
    assert_eq!(bindings["items"][0]["provider_model_id"], "gpt-4");
}

#[tokio::test]
async fn full_workspace_prompt_release_lifecycle() {
    let app = app_with_token().await;

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/prompts/assets",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "prompt_asset_id": "asset_full_prompt",
                "name": "Support Prompt",
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
            "/v1/prompts/assets/asset_full_prompt/versions",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "prompt_version_id": "ver_full_prompt",
                "content_hash": "hash_full_prompt_v1"
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
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "prompt_release_id": "rel_full_prompt",
                "prompt_asset_id": "asset_full_prompt",
                "prompt_version_id": "ver_full_prompt"
            }),
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    for state in ["proposed", "approved"] {
        assert_eq!(
            send_json_request(
                &app,
                "POST",
                "/v1/prompts/releases/rel_full_prompt/transition",
                serde_json::json!({ "to_state": state }),
            )
            .await
            .status(),
            StatusCode::OK
        );
    }

    assert_eq!(
        send_empty_request(
            &app,
            "POST",
            "/v1/prompts/releases/rel_full_prompt/activate"
        )
        .await
        .status(),
        StatusCode::OK
    );

    let releases = response_json(
        send_empty_request(
            &app,
            "GET",
            "/v1/prompts/releases?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        )
        .await,
    )
    .await;
    assert!(releases["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["prompt_release_id"] == "rel_full_prompt" && item["state"] == "active"));
}

#[tokio::test]
async fn full_workspace_run_with_tool_invocation() {
    let app = app_with_token().await;

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/sessions",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "session_id": "session_full_tool"
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
            "/v1/runs",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "session_id": "session_full_tool",
                "run_id": "run_full_tool"
            }),
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    // Claim the run after creation. On the Fabric runtime this flips
    // lifecycle_phase=active so downstream FCALLs (suspend / signal)
    // accept the execution; on the in-memory runtime it's a no-op
    // that returns the record. Pinning the route here means a
    // regression that breaks the handler wiring fails this
    // lifecycle test, not just the dedicated unit test.
    let claim_run_response = send_json_request(
        &app,
        "POST",
        "/v1/runs/run_full_tool/claim",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(claim_run_response.status(), StatusCode::OK);

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/tasks",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "task_id": "task_full_tool",
                "parent_run_id": "run_full_tool"
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
            "/v1/tasks/task_full_tool/claim",
            serde_json::json!({ "worker_id": "worker_full_tool" }),
        )
        .await
        .status(),
        StatusCode::OK
    );

    let create_invocation = send_json_request(
        &app,
        "POST",
        "/v1/tool-invocations",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "invocation_id": "inv_full_tool",
            "session_id": "session_full_tool",
            "run_id": "run_full_tool",
            "task_id": "task_full_tool",
            "target": {
                "target_type": "builtin",
                "tool_name": "git.status"
            },
            "execution_class": "supervised_process"
        }),
    )
    .await;
    assert_eq!(create_invocation.status(), StatusCode::CREATED);
    let create_invocation_json = response_json(create_invocation).await;
    assert_eq!(create_invocation_json["state"], "started");

    let invocation_get =
        response_json(send_empty_request(&app, "GET", "/v1/tool-invocations/inv_full_tool").await)
            .await;
    assert_eq!(invocation_get["state"], "started");

    let complete_invocation = response_json(
        send_empty_request(&app, "POST", "/v1/tool-invocations/inv_full_tool/complete").await,
    )
    .await;
    assert_eq!(complete_invocation["state"], "completed");

    assert_eq!(
        send_empty_request(&app, "POST", "/v1/tasks/task_full_tool/complete")
            .await
            .status(),
        StatusCode::OK
    );

    let run_detail =
        response_json(send_empty_request(&app, "GET", "/v1/runs/run_full_tool").await).await;
    assert_eq!(run_detail["run"]["state"], "completed");
}

#[tokio::test]
async fn full_workspace_eval_complete_flow() {
    let app = app_with_token().await;

    let _ = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "prompt_asset_id": "asset_full_eval",
            "name": "Eval Prompt",
            "kind": "system"
        }),
    )
    .await;
    let _ = send_json_request(
        &app,
        "POST",
        "/v1/prompts/assets/asset_full_eval/versions",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "prompt_version_id": "ver_full_eval",
            "content_hash": "hash_full_eval"
        }),
    )
    .await;
    let _ = send_json_request(
        &app,
        "POST",
        "/v1/prompts/releases",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "prompt_release_id": "rel_full_eval",
            "prompt_asset_id": "asset_full_eval",
            "prompt_version_id": "ver_full_eval"
        }),
    )
    .await;

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/evals/runs",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "workspace_id": "default_workspace",
                "project_id": "default_project",
                "eval_run_id": "eval_full_flow",
                "subject_kind": "prompt_release",
                "evaluator_type": "regression",
                "prompt_asset_id": "asset_full_eval",
                "prompt_version_id": "ver_full_eval",
                "prompt_release_id": "rel_full_eval"
            }),
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    assert_eq!(
        send_empty_request(&app, "POST", "/v1/evals/runs/eval_full_flow/start")
            .await
            .status(),
        StatusCode::OK
    );

    assert_eq!(
        send_json_request(
            &app,
            "POST",
            "/v1/evals/runs/eval_full_flow/complete",
            serde_json::json!({
                "metrics": {
                    "task_success_rate": 0.85,
                    "latency_p50_ms": 100
                },
                "cost": 0.12
            }),
        )
        .await
        .status(),
        StatusCode::OK
    );

    let scorecard = response_json(
        send_empty_request(
            &app,
            "GET",
            "/v1/evals/scorecard/asset_full_eval?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        )
        .await,
    )
    .await;
    assert!(scorecard["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["eval_run_id"] == "eval_full_flow"));
}

#[tokio::test]
async fn full_workspace_memory_import_export_round_trip() {
    let app = app_with_token().await;
    let bundle = three_doc_bundle(
        "default_tenant",
        "default_workspace",
        "default_project",
        "bundle_full_memory",
    );

    let apply_response = send_json_request(&app, "POST", "/v1/bundles/apply", bundle).await;
    assert_eq!(apply_response.status(), StatusCode::OK);

    let sources = response_json(
        send_empty_request(
            &app,
            "GET",
            "/v1/sources?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project",
        )
        .await,
    )
    .await;
    assert_eq!(sources.as_array().unwrap()[0]["document_count"], 3);

    let search = response_json(
        send_empty_request(
            &app,
            "GET",
            "/v1/memory/search?tenant_id=default_tenant&workspace_id=default_workspace&project_id=default_project&query_text=magic%20reset%20phrase&limit=5",
        )
        .await,
    )
    .await;
    assert!(!search["results"].as_array().unwrap().is_empty());

    let export = response_json(
        send_empty_request(
            &app,
            "GET",
            "/v1/bundles/export?project=default_tenant/default_workspace/default_project&source_ids=bundle_full_memory",
        )
        .await,
    )
    .await;
    assert_eq!(export["artifact_count"], 3);
}

#[tokio::test]
async fn full_workspace_approval_gate_flow() {
    let app = app_with_token().await;

    let _ = send_json_request(
        &app,
        "POST",
        "/v1/sessions",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_full_approval"
        }),
    )
    .await;
    let _ = send_json_request(
        &app,
        "POST",
        "/v1/runs",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "session_id": "session_full_approval",
            "run_id": "run_full_approval"
        }),
    )
    .await;

    let approval = send_json_request(
        &app,
        "POST",
        "/v1/approvals",
        serde_json::json!({
            "tenant_id": "default_tenant",
            "workspace_id": "default_workspace",
            "project_id": "default_project",
            "approval_id": "approval_full_approval",
            "run_id": "run_full_approval",
            "requirement": "required"
        }),
    )
    .await;
    assert_eq!(approval.status(), StatusCode::CREATED);

    let waiting_run =
        response_json(send_empty_request(&app, "GET", "/v1/runs/run_full_approval").await).await;
    assert_eq!(waiting_run["run"]["state"], "waiting_approval");

    assert_eq!(
        send_empty_request(&app, "POST", "/v1/approvals/approval_full_approval/approve")
            .await
            .status(),
        StatusCode::OK
    );

    let resumed_run =
        response_json(send_empty_request(&app, "GET", "/v1/runs/run_full_approval").await).await;
    assert_eq!(resumed_run["run"]["state"], "running");
}
