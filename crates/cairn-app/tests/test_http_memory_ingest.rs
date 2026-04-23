//! `POST /v1/memory/ingest` validation (closes #238).
//!
//! Before: empty `source_id` silently minted a blank-id source;
//! `source_type: "structured_json"` accepted arbitrary non-JSON content
//! and only failed later at retrieval. Now both surface as 422 at
//! ingest time.

mod support;

use serde_json::json;
use support::live_fabric::LiveHarness;

fn ingest_url(h: &LiveHarness) -> String {
    format!("{}/v1/memory/ingest", h.base_url)
}

#[tokio::test]
async fn empty_source_id_returns_422() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .post(ingest_url(&h))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "source_id": "",
            "document_id": "doc1",
            "content": "hello world",
        }))
        .send()
        .await
        .expect("ingest empty source_id");
    assert_eq!(
        r.status().as_u16(),
        422,
        "empty source_id must 422: body={}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn malformed_structured_json_returns_422() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .post(ingest_url(&h))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "source_id": "src1",
            "document_id": "doc1",
            "content": "this is not JSON {{",
            "source_type": "structured_json",
        }))
        .send()
        .await
        .expect("ingest malformed JSON");
    assert_eq!(
        r.status().as_u16(),
        422,
        "malformed structured_json must 422: body={}",
        r.text().await.unwrap_or_default(),
    );
}

#[tokio::test]
async fn well_formed_structured_json_succeeds() {
    let h = LiveHarness::setup().await;
    let r = h
        .client()
        .post(ingest_url(&h))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": h.tenant,
            "workspace_id": h.workspace,
            "project_id": h.project,
            "source_id": "src1",
            "document_id": "doc1",
            "content": r#"{"title": "hello", "body": "world"}"#,
            "source_type": "structured_json",
        }))
        .send()
        .await
        .expect("ingest well-formed JSON");
    assert_eq!(
        r.status().as_u16(),
        200,
        "well-formed structured_json should 200: body={}",
        r.text().await.unwrap_or_default(),
    );
}
