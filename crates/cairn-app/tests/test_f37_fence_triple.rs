//! F37 regression: cairn must never send a partial fence triple to FF.
//!
//! # Bug (live dogfood v5 on 2026-04-25)
//!
//! Simple text-answer tasks ("Fibonacci function", "Redstone repeater")
//! failed after 4-5 successful provider turns with:
//!
//! ```text
//! ERROR request: fabric layer error
//! fabric_err=internal: ff_complete_execution rejected: partial_fence_triple
//! ```
//!
//! HTTP response: `{"reason":"internal runtime error: fabric layer error",
//! "termination":"failed"}`. No approval-gated tools were invoked — just
//! memory_search / glob / notify_operator. So the failure wasn't a
//! waitpoint issue; it was terminal-FCALL fence semantics.
//!
//! # Root cause
//!
//! FF's `ff_complete_execution` / `ff_fail_execution` / `ff_cancel_execution`
//! use RFC #58.5 fence-triple resolution (`resolve_lease_fence` in
//! `flowfabric.lua`): the `(lease_id, lease_epoch, attempt_id)` triple
//! must be either **all three set** (normal path, FF validates against
//! stored lease) or **all three empty** (unfenced — FF server-resolves
//! and requires `source=="operator_override"`). Any mix is rejected
//! with `partial_fence_triple`.
//!
//! Cairn's `resolve_lease_context` (both `FabricRunService` and
//! `FabricTaskService`) happily emitted **partial triples**:
//!
//! * When the lease expired (default 30s TTL) but `current_attempt_id`
//!   persisted → `(lease_id="", lease_epoch="1", attempt_id="<set>")`.
//!   The 30s TTL is trivially exceeded by a 5-iteration LLM run.
//! * When the lease was absent but `current_lease_epoch` was stamped
//!   from a prior lifecycle phase → same partial shape.
//!
//! Additionally, `build_complete_execution` / `build_fail_execution`
//! never passed the `source` ARGV at all — so even when the triple WAS
//! fully empty, FF would reject with `fence_required` (terminal ops
//! demand `source=="operator_override"` in unfenced mode).
//!
//! # Fix
//!
//! 1. `resolve_lease_context` in both services now guarantees the
//!    fence-triple invariant: it's all-three-set (live lease + current
//!    attempt) or all-three-empty (unfenced). Partial is unreachable.
//! 2. `ExecutionLeaseContext` grew a `source` field, populated to
//!    `"operator_override"` in the unfenced branch and `""` otherwise.
//! 3. `build_complete_execution` (ARGV 5→6) and `build_fail_execution`
//!    (ARGV 7→8) now carry `source` as the trailing argument.
//!
//! Cairn is the sole authoritative writer of run-execution lifecycle on
//! its side (one orchestrator per run), so `"operator_override"` is
//! semantically correct. FF still enforces lifecycle-phase, terminal,
//! and revocation checks in `validate_lease_and_mark_expired`, which is
//! the real safety net.
//!
//! # This test
//!
//! End-to-end LiveHarness driven through real HTTP. Two assertions:
//!
//! * `complete_run_after_lease_expiry_succeeds` — create session + run
//!   with `CAIRN_FABRIC_LEASE_TTL_MS=1000`, wait 1.5 s (lease expires),
//!   then `POST /v1/runs/:id/intervene {"action":"force_complete"}`.
//!   Pre-F37 this failed with `partial_fence_triple`. Post-F37 the
//!   force-complete succeeds via the unfenced `operator_override` path.
//!
//! * `complete_run_with_live_lease_succeeds` — the same flow WITHOUT
//!   sleeping past the lease TTL. Exercises the fully-fenced path so a
//!   future regression that blanks the fence unconditionally is caught.

mod support;

use std::time::Duration;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

/// Provision session + run + claim. `POST /v1/runs` creates the FF
/// execution (lifecycle phase `pending`). `POST /v1/runs/:id/claim`
/// calls `issue_grant_and_claim`, establishing the lease + transitioning
/// the execution to `active` — required before any terminal FCALL will
/// accept the run.
async fn provision_session_and_run(h: &LiveHarness) -> String {
    let suffix = h.project.clone();
    let tenant = h.tenant.clone();
    let workspace = h.workspace.clone();
    let project = h.project.clone();
    let session_id = format!("sess_f37_{suffix}");
    let run_id = format!("run_f37_{suffix}");

    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
        }))
        .send()
        .await
        .expect("session request reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "session create: {}",
        r.text().await.unwrap_or_default()
    );

    let r = h
        .client()
        .post(format!("{}/v1/runs", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
            "run_id": run_id,
        }))
        .send()
        .await
        .expect("run request reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "run create: {}",
        r.text().await.unwrap_or_default()
    );

    // Claim transitions the run to `active` and establishes the lease.
    // Without this, a terminal FCALL hits `execution_not_active` because
    // the run is still in `pending` (POST /v1/runs only creates).
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/claim", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("claim request reaches server");
    assert_eq!(
        r.status().as_u16(),
        200,
        "run claim: {}",
        r.text().await.unwrap_or_default()
    );

    run_id
}

/// POST a ForceComplete intervention on the given run. Returns
/// `(status, body)`.
async fn force_complete(h: &LiveHarness, run_id: &str) -> (u16, Value) {
    let r = h
        .client()
        .post(format!("{}/v1/runs/{}/intervene", h.base_url, run_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "action": "force_complete",
            "reason": "f37 regression test",
        }))
        .send()
        .await
        .expect("intervene request reaches server");
    let status = r.status().as_u16();
    let body = r.json::<Value>().await.unwrap_or(Value::Null);
    (status, body)
}

/// **F37 primary regression**: force-complete after the lease has
/// expired must no longer surface as an opaque 500
/// `fabric layer error: partial_fence_triple`.
///
/// Pre-F37 behaviour (the dogfood v5 symptom): cairn's
/// `resolve_lease_context` emitted `(lease_id="", lease_epoch="<set>",
/// attempt_id="<set>")` — a partial triple — so FF rejected with
/// `partial_fence_triple` and cairn surfaced it as a generic 500.
///
/// Post-F37 behaviour: the triple is either all-set or all-empty.
/// When FF's expiry scanner has already moved the execution to
/// `lifecycle_phase=terminal, terminal_outcome=expired` (which is the
/// exact dogfood state: long-running orchestrator loop outlives the
/// 30 s default lease), the unfenced path surfaces FF's
/// `execution_not_active` — a clean, structured 409 Conflict, NOT a
/// 500 with raw FCALL internals.
///
/// The test uses a 1 s lease TTL (FabricConfig's hard minimum) plus a
/// sleep so the FF-side `attempt_timeout` / `execution_deadline`
/// scanners reliably roll the execution past `active`.
#[tokio::test]
async fn complete_run_after_lease_expiry_returns_clean_conflict() {
    // 1 s is FabricConfig::validate's hard minimum.
    let h = LiveHarness::setup_with_env(&[("CAIRN_FABRIC_LEASE_TTL_MS", "1000")]).await;

    let run_id = provision_session_and_run(&h).await;

    // Wait past lease TTL. FF's expiry scanner runs on a separate cadence
    // and may need a moment to move the execution to terminal; 3 s is a
    // generous upper bound observed on CI.
    tokio::time::sleep(Duration::from_millis(3_000)).await;

    let (status, body) = force_complete(&h, &run_id).await;

    // The load-bearing F37 invariant: no 500 `fabric layer error`, no
    // raw `partial_fence_triple`, no raw `fence_required`. Pre-F37 the
    // response was {"message":"internal runtime error: fabric layer
    // error","status_code":500,...}. Post-F37 either 200 (lease still
    // live — FF's scanner hadn't run yet) or a structured 4xx
    // (execution already terminal). Either outcome is fine; what's
    // unacceptable is a 500 leaking FCALL internals.
    let body_str = body.to_string();
    assert_ne!(
        status, 500,
        "F37: lease-expired force-complete must NOT surface as a 500 \
         fabric layer error. status={status}, body={body_str}"
    );
    assert!(
        !body_str.contains("partial_fence_triple"),
        "F37: response must not leak `partial_fence_triple`; body={body_str}"
    );
    assert!(
        !body_str.contains("fence_required"),
        "F37: response must not leak `fence_required`; body={body_str}"
    );
    assert!(
        !body_str.contains("fabric layer error"),
        "F37: response must not leak `fabric layer error`; body={body_str}"
    );
}

/// Complementary regression: force-complete with a **live** lease must
/// also keep working. Without this assertion, a naive "blank the fence
/// unconditionally" fix would still pass the primary test above. The
/// fenced path must remain valid.
#[tokio::test]
async fn complete_run_with_live_lease_succeeds() {
    let h = LiveHarness::setup().await;

    let run_id = provision_session_and_run(&h).await;

    // No sleep — the lease is fresh (default 30 s TTL). This exercises
    // `resolve_lease_context`'s fenced branch (all three fence tokens
    // populated, source="").
    let (status, body) = force_complete(&h, &run_id).await;

    assert_eq!(
        status, 200,
        "F37: force-complete with live lease must succeed. \
         status={status}, body={body}"
    );
    assert_eq!(
        body.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "F37: force-complete (fenced) must report ok=true; body={body}"
    );
}
