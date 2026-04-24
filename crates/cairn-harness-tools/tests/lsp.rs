//! Adapter-layer tests for `HarnessLsp`.
//!
//! Exhaustive LSP semantics live in `harness-lsp`'s own test suite — these
//! tests cover only the cairn-side contract: session-scoped client cache,
//! ToolHandler metadata, and a gated rust-analyzer smoke test.

use std::sync::Arc;

use cairn_domain::ProjectKey;
use cairn_harness_tools::{
    __clear_client_cache_for_tests, HarnessBuiltin, HarnessLsp,
};
use cairn_tools::builtins::{
    PermissionLevel, ToolCategory, ToolContext, ToolEffect, ToolHandler,
};
use serde_json::json;
use tempfile::TempDir;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn other_project() -> ProjectKey {
    ProjectKey::new("t2", "w2", "p2")
}

fn ctx_with_session(dir: &TempDir, session_id: &str) -> ToolContext {
    let mut c = ToolContext::default();
    c.working_dir = dir.path().to_path_buf();
    c.session_id = Some(session_id.to_owned());
    c
}

// ── Metadata / schema ────────────────────────────────────────────────────────

#[test]
fn lsp_handler_metadata_is_read_only_observational() {
    let tool = HarnessBuiltin::<HarnessLsp>::new();
    assert_eq!(tool.name(), "lsp");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(tool.category(), ToolCategory::FileSystem);
    assert!(matches!(tool.tool_effect(), ToolEffect::Observational));
    // Description mirrors harness-lsp's LSP_TOOL_DESCRIPTION and must mention
    // 1-indexed positions so the model uses the right convention.
    let desc = tool.description();
    assert!(desc.contains("1-INDEXED"), "description must flag 1-indexed positions: {desc}");
    assert!(desc.contains("server_starting"), "description must mention server_starting retry");
}

#[test]
fn lsp_schema_lists_all_six_operations() {
    let tool = HarnessBuiltin::<HarnessLsp>::new();
    let schema = tool.parameters_schema();
    let ops = schema
        .pointer("/properties/operation/enum")
        .and_then(|v| v.as_array())
        .expect("schema must expose operation enum");
    let names: Vec<&str> = ops.iter().filter_map(|v| v.as_str()).collect();
    for expected in [
        "hover",
        "definition",
        "references",
        "documentSymbol",
        "workspaceSymbol",
        "implementation",
    ] {
        assert!(
            names.contains(&expected),
            "schema missing operation {expected}; got {names:?}"
        );
    }
}

// ── Session cache ────────────────────────────────────────────────────────────

#[tokio::test]
async fn same_session_reuses_cached_lsp_client() {
    __clear_client_cache_for_tests().await;
    let dir = TempDir::new().unwrap();
    let ctx = ctx_with_session(&dir, "session-SHARE");

    let a = cairn_harness_tools::tools::lsp::client_for(&ctx, &project());
    let b = cairn_harness_tools::tools::lsp::client_for(&ctx, &project());
    assert!(
        Arc::ptr_eq(&a, &b),
        "two client_for calls in the same session must return the same Arc",
    );
}

#[tokio::test]
async fn different_sessions_get_isolated_lsp_clients() {
    __clear_client_cache_for_tests().await;
    let dir = TempDir::new().unwrap();
    let ctx_a = ctx_with_session(&dir, "session-A");
    let ctx_b = ctx_with_session(&dir, "session-B");

    let a = cairn_harness_tools::tools::lsp::client_for(&ctx_a, &project());
    let b = cairn_harness_tools::tools::lsp::client_for(&ctx_b, &project());
    assert!(
        !Arc::ptr_eq(&a, &b),
        "different session_ids in the same project must not share an LSP client",
    );
}

#[tokio::test]
async fn different_projects_get_isolated_lsp_clients() {
    __clear_client_cache_for_tests().await;
    let dir = TempDir::new().unwrap();
    let ctx = ctx_with_session(&dir, "same-session-id");

    let a = cairn_harness_tools::tools::lsp::client_for(&ctx, &project());
    let b = cairn_harness_tools::tools::lsp::client_for(&ctx, &other_project());
    assert!(
        !Arc::ptr_eq(&a, &b),
        "same session_id under different tenants/projects must not share a client",
    );
}

// ── Failure paths through ToolHandler ────────────────────────────────────────

#[tokio::test]
async fn lsp_call_without_manifest_reports_server_not_available() {
    // No .lsp.json in workspace → upstream must return ServerNotAvailable.
    // This proves the adapter correctly plumbs a session config that reaches
    // the manifest-loading path and surfaces its errors through ToolResult.
    __clear_client_cache_for_tests().await;
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.ts"), "const x = 1;\n").unwrap();

    let tool = HarnessBuiltin::<HarnessLsp>::new();
    let ctx = ctx_with_session(&dir, "session-no-manifest");
    let err = tool
        .execute_with_context(
            &project(),
            json!({
                "operation": "hover",
                "path": dir.path().join("a.ts").to_string_lossy(),
                "line": 1,
                "character": 7,
            }),
            &ctx,
        )
        .await
        .expect_err("expected error when no manifest is configured");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ServerNotAvailable") || msg.contains("server_not_available"),
        "expected ServerNotAvailable error, got: {msg}"
    );
}

#[tokio::test]
async fn lsp_rejects_zero_indexed_position() {
    // Guard: the tool must reject 0-indexed positions at the schema layer,
    // not silently accept them (LSP's convention is 0-based but the model
    // sees editor-style 1-based).
    __clear_client_cache_for_tests().await;
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.ts"), "const x = 1;\n").unwrap();

    let tool = HarnessBuiltin::<HarnessLsp>::new();
    let ctx = ctx_with_session(&dir, "session-zero");
    let err = tool
        .execute_with_context(
            &project(),
            json!({
                "operation": "hover",
                "path": dir.path().join("a.ts").to_string_lossy(),
                "line": 0,
                "character": 0,
            }),
            &ctx,
        )
        .await
        .expect_err("0-indexed position must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("1-indexed") || msg.contains("InvalidParam"),
        "expected 1-indexed rejection, got: {msg}"
    );
}

// ── Gated real-LSP integration test ──────────────────────────────────────────

/// Smoke test against a real `rust-analyzer` server. Requires:
///   * `rust-analyzer` on PATH (`rustup component add rust-analyzer`)
///   * seconds of indexing — ignored by default.
///
/// Run manually:
///   cargo test -p cairn-harness-tools --test lsp -- --ignored rust_analyzer
#[tokio::test]
#[ignore = "requires rust-analyzer on PATH; slow (indexes a cargo project)"]
async fn rust_analyzer_hover_round_trip() {
    use std::collections::HashMap;

    __clear_client_cache_for_tests().await;
    let dir = TempDir::new().unwrap();

    // Minimal cargo project.
    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "lsp_smoke"
version = "0.0.0"
edition = "2021"
[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let src_path = dir.path().join("src").join("lib.rs");
    std::fs::write(
        &src_path,
        "pub fn greet() -> &'static str { \"hi\" }\n",
    )
    .unwrap();

    // .lsp.json so harness-lsp finds a profile for `.rs`.
    let mut servers = HashMap::new();
    servers.insert(
        "rust".to_string(),
        serde_json::json!({
            "language": "rust",
            "extensions": [".rs"],
            "command": ["rust-analyzer"],
            "rootPatterns": ["Cargo.toml"],
        }),
    );
    std::fs::write(
        dir.path().join(".lsp.json"),
        serde_json::to_string(&serde_json::json!({ "servers": servers })).unwrap(),
    )
    .unwrap();

    let tool = HarnessBuiltin::<HarnessLsp>::new();
    let ctx = ctx_with_session(&dir, "session-ra");

    // Hover on `greet` (line 1, column 8 for "pub fn greet").
    // First call may return server_starting — retry a few times.
    for attempt in 0..30u32 {
        let res = tool
            .execute_with_context(
                &project(),
                json!({
                    "operation": "hover",
                    "path": src_path.to_string_lossy(),
                    "line": 1,
                    "character": 8,
                }),
                &ctx,
            )
            .await;
        match res {
            Ok(ok) if ok.output["kind"] == "hover" => {
                return;
            }
            Ok(ok) if ok.output["kind"] == "server_starting" => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }
            Ok(other) => {
                // no_results is also acceptable — the position may have
                // landed between tokens. The point of this test is that
                // the round-trip completed.
                if other.output["kind"] == "no_results" {
                    return;
                }
                panic!("unexpected ok result on attempt {attempt}: {:?}", other.output);
            }
            Err(e) => panic!("lsp call errored on attempt {attempt}: {e:?}"),
        }
    }
    panic!("rust-analyzer never became ready after 30 retries");
}
