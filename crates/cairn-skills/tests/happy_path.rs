//! Integration tests for `cairn-skills` — the `skill` builtin adapter
//! over published `harness-skill` 0.1.0.
//!
//! Coverage:
//! - `activation_succeeds_and_returns_body` — a valid SKILL.md under
//!   `<cwd>/.cairn/skills/<name>/` is discovered, activated, and the
//!   body is returned in the `kind: "ok"` result.
//! - `second_activation_returns_already_loaded` — session dedupe via
//!   `ActivatedSet` fires on the second call within the same session.
//! - `different_sessions_do_not_share_dedupe` — session isolation: A's
//!   activation does not satisfy B's dedupe.
//! - `unknown_skill_returns_not_found_with_suggestions` — fuzzy-match
//!   siblings flow through the `not_found` variant.
//! - `invalid_name_returns_error` — parameter-parse rejection surfaces
//!   as `ToolError::HarnessError { code: InvalidParam }`.
//!
//! Exhaustive SKILL.md parsing, fence, and trust-policy semantics live
//! in upstream `harness-skill` tests; this suite guards the cairn adapter
//! layer only (schema, session wiring, result → ToolResult mapping,
//! per-session ActivatedSet cache).

use std::fs;

use cairn_domain::ProjectKey;
use cairn_harness_tools::HarnessBuiltin;
use cairn_skills::{__clear_activated_sets_for_tests, HarnessSkill};
use cairn_tools::builtins::{ToolContext, ToolError, ToolHandler};
use harness_core::ToolErrorCode;
use serde_json::json;
use tempfile::TempDir;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn ctx_for(dir: &TempDir, session_id: &str) -> ToolContext {
    let mut c = ToolContext::default();
    c.working_dir = dir.path().to_path_buf();
    c.session_id = Some(session_id.to_owned());
    c
}

/// Plant a minimal conformant SKILL.md under `<dir>/.cairn/skills/<name>/`.
/// Returns the skill directory path.
fn plant_skill(dir: &TempDir, name: &str, description: &str, body: &str) {
    let skill_dir = dir.path().join(".cairn").join("skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    let content = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
    fs::write(skill_dir.join("SKILL.md"), content).unwrap();
}

#[tokio::test]
async fn activation_succeeds_and_returns_body() {
    __clear_activated_sets_for_tests();
    let dir = TempDir::new().unwrap();
    plant_skill(
        &dir,
        "run-analysis",
        "Summarizes a completed agent run. Use after a run finishes.",
        "When analyzing a run, list each step and flag anomalies.",
    );

    let tool = HarnessBuiltin::<HarnessSkill>::new();
    let res = tool
        .execute_with_context(
            &project(),
            json!({ "name": "run-analysis" }),
            &ctx_for(&dir, "sess-1"),
        )
        .await
        .expect("activation should succeed");

    assert_eq!(res.output["kind"], "ok");
    assert_eq!(res.output["name"], "run-analysis");
    assert!(
        res.output["body"]
            .as_str()
            .unwrap_or("")
            .contains("flag anomalies"),
        "body should include the SKILL.md prose; got {:?}",
        res.output["body"]
    );
    assert!(res.output["bytes"].as_u64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn second_activation_returns_already_loaded() {
    __clear_activated_sets_for_tests();
    let dir = TempDir::new().unwrap();
    plant_skill(&dir, "code-review", "Reviews a diff.", "Body text.");

    let tool = HarnessBuiltin::<HarnessSkill>::new();
    let ctx = ctx_for(&dir, "sess-dedupe");

    // First call — expect ok.
    let first = tool
        .execute_with_context(&project(), json!({ "name": "code-review" }), &ctx)
        .await
        .expect("first activation");
    assert_eq!(first.output["kind"], "ok");

    // Second call in the same session — expect already_loaded.
    let second = tool
        .execute_with_context(&project(), json!({ "name": "code-review" }), &ctx)
        .await
        .expect("second activation");
    assert_eq!(
        second.output["kind"], "already_loaded",
        "dedupe should fire within the same session"
    );
    assert_eq!(second.output["name"], "code-review");
}

#[tokio::test]
async fn different_sessions_do_not_share_dedupe() {
    __clear_activated_sets_for_tests();
    let dir = TempDir::new().unwrap();
    plant_skill(&dir, "triage", "Triage incoming issues.", "Body.");

    let tool = HarnessBuiltin::<HarnessSkill>::new();

    // Session A activates.
    let a = tool
        .execute_with_context(
            &project(),
            json!({ "name": "triage" }),
            &ctx_for(&dir, "sess-A"),
        )
        .await
        .expect("session A activation");
    assert_eq!(a.output["kind"], "ok");

    // Session B, same project — dedupe set must NOT be shared.
    let b = tool
        .execute_with_context(
            &project(),
            json!({ "name": "triage" }),
            &ctx_for(&dir, "sess-B"),
        )
        .await
        .expect("session B activation");
    assert_eq!(
        b.output["kind"], "ok",
        "session B should see a fresh activation, not already_loaded"
    );
}

#[tokio::test]
async fn unknown_skill_returns_not_found_with_suggestions() {
    __clear_activated_sets_for_tests();
    let dir = TempDir::new().unwrap();
    // Plant two skills so the fuzzy-suggester has material to work with.
    plant_skill(&dir, "run-analysis", "A.", "body");
    plant_skill(&dir, "run-summary", "B.", "body");

    let tool = HarnessBuiltin::<HarnessSkill>::new();
    let res = tool
        .execute_with_context(
            &project(),
            // Typo: close to both installed skills.
            json!({ "name": "run-analyzis" }),
            &ctx_for(&dir, "sess-nf"),
        )
        .await
        .expect("not-found should be an ok ToolResult, not an error");

    assert_eq!(res.output["kind"], "not_found");
    assert_eq!(res.output["name"], "run-analyzis");
    let siblings = res.output["siblings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !siblings.is_empty(),
        "fuzzy-suggester should return at least one neighbour"
    );
}

#[tokio::test]
async fn invalid_name_returns_error() {
    __clear_activated_sets_for_tests();
    let dir = TempDir::new().unwrap();
    let tool = HarnessBuiltin::<HarnessSkill>::new();

    // Upper-case / underscore — rejected by harness-skill's name validator.
    let err = tool
        .execute_with_context(
            &project(),
            json!({ "name": "Bad_Name" }),
            &ctx_for(&dir, "sess-inv"),
        )
        .await
        .expect_err("invalid name must surface as an error");

    match err {
        ToolError::HarnessError { code, .. } => {
            assert_eq!(code, ToolErrorCode::InvalidParam);
        }
        other => panic!("expected HarnessError(InvalidParam), got {other:?}"),
    }
}
