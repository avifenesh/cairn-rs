//! Issue #152 — sources CRUD + memory ingest HTTP surface.
//!
//! The UI's SourcesPage and MemoryPage drive these endpoints directly.
//! A regression in shape, status code, or scope handling would break the
//! operator workflow: registering a source, ingesting a document into it,
//! inspecting chunks, scheduling a refresh, and retiring the source.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn sources_crud_and_memory_ingest_roundtrip() {
    let h = LiveHarness::setup().await;
    let base = &h.base_url;
    let source_id = "docs/handbook-test";
    // Axum path segments must not contain `/`; mirror the UI's encodeURIComponent.
    let encoded_source_id = source_id.replace('/', "%2F");

    // 1. Create source.
    let res = h
        .client()
        .post(format!("{base}/v1/sources"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    h.tenant,
            "workspace_id": h.workspace,
            "project_id":   h.project,
            "source_id":    source_id,
            "name":         "Team Handbook",
            "description":  "Ingest-CRUD roundtrip",
        }))
        .send()
        .await
        .expect("create source reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "create source status, body: {}",
        res.text().await.unwrap_or_default(),
    );

    // 2. Ingest a document into the source.
    let res = h
        .client()
        .post(format!("{base}/v1/memory/ingest"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    h.tenant,
            "workspace_id": h.workspace,
            "project_id":   h.project,
            "source_id":    source_id,
            "document_id":  "onboarding.md",
            "content":      "Welcome to the team. This is the onboarding guide.",
        }))
        .send()
        .await
        .expect("ingest reaches server");
    let ingest_status = res.status().as_u16();
    let ingest_body = res.text().await.unwrap_or_default();
    assert!(
        (200..300).contains(&ingest_status),
        "ingest status {ingest_status}, body: {ingest_body}",
    );

    // 3. List sources — the new source must be present.
    let res = h
        .client()
        .get(format!(
            "{base}/v1/sources?tenant_id={}&workspace_id={}&project_id={}",
            h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("list sources reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("list json");
    let items = body.as_array().cloned().unwrap_or_default();
    assert!(
        items.iter().any(|s| s.get("source_id").and_then(|v| v.as_str()) == Some(source_id)),
        "expected list to contain new source, got {body}",
    );

    // 4. List chunks — the ingested document produced at least one chunk.
    let res = h
        .client()
        .get(format!(
            "{base}/v1/sources/{}/chunks?tenant_id={}&workspace_id={}&project_id={}",
            encoded_source_id, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("chunks reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("chunks json");
    let chunks = body.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    assert!(!chunks.is_empty(), "expected ingested chunks, got {body}");

    // 5. Update source metadata.
    let res = h
        .client()
        .put(format!("{base}/v1/sources/{}", encoded_source_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    h.tenant,
            "workspace_id": h.workspace,
            "project_id":   h.project,
            "name":         "Updated Handbook",
            "description":  "edited",
        }))
        .send()
        .await
        .expect("update reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("update json");
    assert_eq!(body.get("name").and_then(|v| v.as_str()), Some("Updated Handbook"));

    // 6. Set refresh schedule.
    let res = h
        .client()
        .post(format!(
            "{base}/v1/sources/{}/refresh-schedule?tenant_id={}&workspace_id={}&project_id={}",
            encoded_source_id, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "interval_ms": 3_600_000_u64 }))
        .send()
        .await
        .expect("schedule reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("schedule json");
    assert_eq!(body.get("interval_ms").and_then(|v| v.as_u64()), Some(3_600_000));

    // 7. Process-refresh — global endpoint, may process 0+ schedules.
    let res = h
        .client()
        .post(format!("{base}/v1/sources/process-refresh"))
        .bearer_auth(&h.admin_token)
        .json(&json!({}))
        .send()
        .await
        .expect("process-refresh reaches server");
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().await.expect("process-refresh json");
    assert!(body.get("processed_count").is_some(), "missing processed_count: {body}");

    // 8. Delete (deactivate) the source.
    let res = h
        .client()
        .delete(format!("{base}/v1/sources/{}", encoded_source_id))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("delete reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "delete status, body: {}",
        res.text().await.unwrap_or_default(),
    );
}
