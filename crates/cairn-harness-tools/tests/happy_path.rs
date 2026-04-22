//! Happy-path smoke tests: one `execute_with_context` per harness-backed tool.
//!
//! Each test builds a temp workspace, runs the tool with minimal valid args,
//! and asserts that the adapter returns a non-error `ToolResult`. Exhaustive
//! semantics live in the upstream harness crates' own tests — these smoke
//! tests protect against adapter-layer regressions only (schema shape,
//! session wiring, result→ToolResult mapping).

use std::sync::Arc;

use cairn_domain::ProjectKey;
use cairn_harness_tools::{
    HarnessBash, HarnessBuiltin, HarnessEdit, HarnessGlob, HarnessGrep, HarnessMultiEdit,
    HarnessRead, HarnessWrite,
};
use cairn_tools::builtins::{ToolContext, ToolHandler};
use serde_json::json;
use tempfile::TempDir;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn ctx_for(dir: &TempDir) -> ToolContext {
    let mut c = ToolContext::default();
    c.working_dir = dir.path().to_path_buf();
    c
}

#[tokio::test]
async fn bash_echo_succeeds() {
    let dir = TempDir::new().unwrap();
    let tool = HarnessBuiltin::<HarnessBash>::new();
    let res = tool
        .execute_with_context(
            &project(),
            json!({ "command": "echo hello" }),
            &ctx_for(&dir),
        )
        .await
        .expect("bash echo should succeed");
    // output shape: { kind: "ok", exit_code: 0, stdout: "...hello..." }
    assert_eq!(res.output["exit_code"], 0);
    assert!(res.output["stdout"]
        .as_str()
        .unwrap_or("")
        .contains("hello"));
}

#[tokio::test]
async fn bash_accepts_cairn_working_dir_alias() {
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();

    let tool = HarnessBuiltin::<HarnessBash>::new();
    // Send the cairn-legacy `working_dir` arg; adapter must remap to `cwd`.
    let res = tool
        .execute_with_context(
            &project(),
            json!({ "command": "pwd", "working_dir": sub.to_string_lossy() }),
            &ctx_for(&dir),
        )
        .await
        .expect("bash with working_dir alias should succeed");
    let stdout = res.output["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("/sub"),
        "expected pwd to report the alias-mapped cwd; got {stdout:?}"
    );
}

#[tokio::test]
async fn ledger_is_scoped_per_session() {
    // Cross-session read must NOT satisfy NOT_READ_THIS_SESSION for another session.
    use cairn_harness_tools::HarnessEdit;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("target.txt");
    std::fs::write(&path, "original\n").unwrap();

    let mut ctx_a = ToolContext::default();
    ctx_a.working_dir = dir.path().to_path_buf();
    ctx_a.session_id = Some("session-A".to_owned());

    let mut ctx_b = ToolContext::default();
    ctx_b.working_dir = dir.path().to_path_buf();
    ctx_b.session_id = Some("session-B".to_owned());

    // Read under session A.
    let _ = HarnessBuiltin::<HarnessRead>::new()
        .execute_with_context(
            &project(),
            json!({ "path": path.to_string_lossy() }),
            &ctx_a,
        )
        .await
        .expect("read in session A");

    // Edit from session B must fail with NOT_READ_THIS_SESSION.
    let err = HarnessBuiltin::<HarnessEdit>::new()
        .execute_with_context(
            &project(),
            json!({
                "path": path.to_string_lossy(),
                "old_string": "original",
                "new_string": "NEW",
            }),
            &ctx_b,
        )
        .await
        .expect_err("edit from session B must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("NotReadThisSession") || msg.contains("NOT_READ_THIS_SESSION"),
        "expected NOT_READ_THIS_SESSION, got {msg}"
    );
}

#[tokio::test]
async fn read_returns_text_for_existing_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("greeting.txt");
    std::fs::write(&file, "line 1\nline 2\nline 3\n").unwrap();

    let tool = HarnessBuiltin::<HarnessRead>::new();
    let res = tool
        .execute_with_context(
            &project(),
            json!({ "path": file.to_string_lossy() }),
            &ctx_for(&dir),
        )
        .await
        .expect("read should succeed");
    assert_eq!(res.output["kind"], "text");
    assert!(res.output["output"]
        .as_str()
        .unwrap_or("")
        .contains("line 2"));
}

#[tokio::test]
async fn glob_finds_created_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.rs"), "// hi").unwrap();
    std::fs::write(dir.path().join("b.rs"), "// hi").unwrap();

    let tool = HarnessBuiltin::<HarnessGlob>::new();
    let res = tool
        .execute_with_context(&project(), json!({ "pattern": "*.rs" }), &ctx_for(&dir))
        .await
        .expect("glob should succeed");
    assert_eq!(res.output["kind"], "paths");
    let paths = res.output["paths"].as_array().expect("paths array");
    assert!(
        paths.len() >= 2,
        "expected >= 2 matched paths, got {paths:?}"
    );
}

#[tokio::test]
async fn grep_finds_pattern_in_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("code.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();

    let tool = HarnessBuiltin::<HarnessGrep>::new();
    let res = tool
        .execute_with_context(
            &project(),
            json!({ "pattern": "alpha", "output_mode": "content" }),
            &ctx_for(&dir),
        )
        .await
        .expect("grep should succeed");
    // content mode returns kind=content with at least one match in output
    assert_eq!(res.output["kind"], "content");
    assert!(res.output["output"]
        .as_str()
        .unwrap_or("")
        .contains("alpha"));
}

#[tokio::test]
async fn write_creates_new_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("new.txt");

    let tool = HarnessBuiltin::<HarnessWrite>::new();
    let res = tool
        .execute_with_context(
            &project(),
            json!({ "path": path.to_string_lossy(), "content": "hello world\n" }),
            &ctx_for(&dir),
        )
        .await
        .expect("write should succeed");
    assert_eq!(res.output["kind"], "text");
    let contents = std::fs::read_to_string(&path).unwrap();
    assert_eq!(contents, "hello world\n");
}

#[tokio::test]
async fn edit_replaces_exact_string() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("doc.txt");
    std::fs::write(&path, "foo bar baz\n").unwrap();

    // harness-write requires a prior read before edit (NOT_READ_THIS_SESSION
    // enforcement). The adapter uses a process-global InMemoryLedger so we
    // must read through the adapter first.
    let read_tool = HarnessBuiltin::<HarnessRead>::new();
    let _ = read_tool
        .execute_with_context(
            &project(),
            json!({ "path": path.to_string_lossy() }),
            &ctx_for(&dir),
        )
        .await
        .expect("read-before-edit should succeed");

    let tool = HarnessBuiltin::<HarnessEdit>::new();
    let res = tool
        .execute_with_context(
            &project(),
            json!({
                "path": path.to_string_lossy(),
                "old_string": "bar",
                "new_string": "QUX",
            }),
            &ctx_for(&dir),
        )
        .await
        .expect("edit should succeed");
    assert_eq!(res.output["kind"], "text");
    let contents = std::fs::read_to_string(&path).unwrap();
    assert_eq!(contents, "foo QUX baz\n");
}

#[tokio::test]
async fn multi_edit_applies_sequence() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("doc.txt");
    std::fs::write(&path, "alpha beta gamma\n").unwrap();

    // Read-before-edit
    let read_tool = HarnessBuiltin::<HarnessRead>::new();
    let _ = read_tool
        .execute_with_context(
            &project(),
            json!({ "path": path.to_string_lossy() }),
            &ctx_for(&dir),
        )
        .await
        .expect("read-before-edit");

    let tool = HarnessBuiltin::<HarnessMultiEdit>::new();
    let res = tool
        .execute_with_context(
            &project(),
            json!({
                "path": path.to_string_lossy(),
                "edits": [
                    { "old_string": "alpha", "new_string": "A" },
                    { "old_string": "gamma", "new_string": "C" },
                ]
            }),
            &ctx_for(&dir),
        )
        .await
        .expect("multi_edit should succeed");
    assert_eq!(res.output["kind"], "text");
    let contents = std::fs::read_to_string(&path).unwrap();
    assert_eq!(contents, "A beta C\n");
}

// webfetch + bash_output + bash_kill are not exercised here because they
// require a test HTTP server / background job lifecycle plumbing that
// belongs in the upstream harness crates' own test suites. The adapter's
// session wiring is covered by the other 7 tests above.
#[test]
fn smoke_registry_descriptors_include_all_harness_tools() {
    use cairn_harness_tools::{HarnessBashKill, HarnessBashOutput, HarnessWebFetch};
    use cairn_tools::builtins::BuiltinToolRegistry;

    let reg = BuiltinToolRegistry::new()
        .register(Arc::new(HarnessBuiltin::<HarnessBash>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessBashOutput>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessBashKill>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessRead>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessGrep>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessGlob>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessWrite>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessEdit>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessMultiEdit>::new()))
        .register(Arc::new(HarnessBuiltin::<HarnessWebFetch>::new()));
    assert_eq!(reg.len(), 10, "expected all 10 harness tools registered");
    for name in [
        "bash",
        "bash_output",
        "bash_kill",
        "read",
        "grep",
        "glob",
        "write",
        "edit",
        "multiedit",
        "webfetch",
    ] {
        assert!(reg.get(name).is_some(), "missing tool: {name}");
    }
}
