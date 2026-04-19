//! Demo data seeding for local development mode.

#[allow(unused_imports)]
use crate::*;

// ── Demo data seeding ─────────────────────────────────────────────────────────

/// Populate the InMemory store with representative demo data so the dashboard
/// and all pages show meaningful content immediately after first start.
///
/// Only called in `DeploymentMode::Local`. Errors are logged but never fatal —
/// a partially-seeded store is better than no server at all.
pub(crate) async fn seed_demo_data(state: &AppState) {
    use cairn_domain::{
        policy::{ApprovalDecision, ApprovalRequirement},
        tenancy::ProjectKey,
        ApprovalId, AuditOutcome, FailureClass, PauseReason, PauseReasonKind, RunId, SessionId,
        TaskId, TenantId,
    };
    use cairn_runtime::{approvals::ApprovalService, audits::AuditService};

    let project = ProjectKey::new("default_tenant", "default_workspace", "demo_project");
    let tenant = TenantId::new("default_tenant");

    // ── 3 Sessions ────────────────────────────────────────────────────────────
    let s_ids: &[&str] = &["sess_alpha", "sess_beta", "sess_gamma"];
    for id in s_ids {
        if let Err(e) = state
            .runtime
            .sessions
            .create(&project, SessionId::new(*id))
            .await
        {
            tracing::warn!("seed: session {id}: {e}");
        }
    }

    // ── 5 Runs ────────────────────────────────────────────────────────────────
    // run_a: completed   (sess_alpha)
    // run_b: completed   (sess_alpha)
    // run_c: running     (sess_beta)
    // run_d: failed      (sess_beta)
    // run_e: paused      (sess_gamma)
    let run_defs: &[(&str, &str)] = &[
        ("run_a", "sess_alpha"),
        ("run_b", "sess_alpha"),
        ("run_c", "sess_beta"),
        ("run_d", "sess_beta"),
        ("run_e", "sess_gamma"),
    ];
    for (run, sess) in run_defs {
        if let Err(e) = state
            .runtime
            .runs
            .start(&project, &SessionId::new(*sess), RunId::new(*run), None)
            .await
        {
            tracing::warn!("seed: run {run}: {e}");
        }
    }
    let sess_alpha = SessionId::new("sess_alpha");
    let sess_beta = SessionId::new("sess_beta");
    let sess_gamma = SessionId::new("sess_gamma");
    let _ = state
        .runtime
        .runs
        .complete(&sess_alpha, &RunId::new("run_a"))
        .await;
    let _ = state
        .runtime
        .runs
        .complete(&sess_alpha, &RunId::new("run_b"))
        .await;
    let _ = state
        .runtime
        .runs
        .fail(
            &sess_beta,
            &RunId::new("run_d"),
            FailureClass::ExecutionError,
        )
        .await;
    let _ = state
        .runtime
        .runs
        .pause(
            &sess_gamma,
            &RunId::new("run_e"),
            PauseReason {
                kind: PauseReasonKind::OperatorPause,
                detail: Some("Demo pause".into()),
                resume_after_ms: None,
                actor: Some("demo_seed".into()),
            },
        )
        .await;

    // ── 12 Tasks ──────────────────────────────────────────────────────────────
    // Distribution: 3 queued, 2 claimed, 2 running, 4 completed, 1 failed, 1 cancelled (=13 total incl task_12)
    let task_defs: &[(&str, &str)] = &[
        ("task_01", "run_a"),
        ("task_02", "run_a"),
        ("task_03", "run_a"),
        ("task_04", "run_b"),
        ("task_05", "run_c"),
        ("task_06", "run_c"),
        ("task_07", "run_c"),
        ("task_08", "run_c"),
        ("task_09", "run_c"),
        ("task_10", "run_d"),
        ("task_11", "run_d"),
        ("task_12", "run_e"),
    ];
    // task_id → session_id map derived from task_defs's parent run and run_defs.
    let task_session = |tid: &str| -> SessionId {
        let parent_run = task_defs
            .iter()
            .find(|(t, _)| *t == tid)
            .map(|(_, r)| *r)
            .unwrap_or("run_a");
        let sess = run_defs
            .iter()
            .find(|(r, _)| *r == parent_run)
            .map(|(_, s)| *s)
            .unwrap_or("sess_alpha");
        SessionId::new(sess)
    };
    for (tid, rid) in task_defs {
        let sess = task_session(tid);
        if let Err(e) = state
            .runtime
            .tasks
            .submit(
                &project,
                Some(&sess),
                TaskId::new(*tid),
                Some(RunId::new(*rid)),
                None,
                0,
            )
            .await
        {
            tracing::warn!("seed: task {tid}: {e}");
        }
    }
    // Complete task_01–04
    for tid in &["task_01", "task_02", "task_03", "task_04"] {
        let sess = task_session(tid);
        let _ = state
            .runtime
            .tasks
            .claim(
                Some(&sess),
                &TaskId::new(*tid),
                "demo-worker".to_owned(),
                300_000,
            )
            .await;
        let _ = state
            .runtime
            .tasks
            .start(Some(&sess), &TaskId::new(*tid))
            .await;
        let _ = state
            .runtime
            .tasks
            .complete(Some(&sess), &TaskId::new(*tid))
            .await;
    }
    // Running: task_05, task_06
    for tid in &["task_05", "task_06"] {
        let sess = task_session(tid);
        let _ = state
            .runtime
            .tasks
            .claim(
                Some(&sess),
                &TaskId::new(*tid),
                "demo-worker".to_owned(),
                300_000,
            )
            .await;
        let _ = state
            .runtime
            .tasks
            .start(Some(&sess), &TaskId::new(*tid))
            .await;
    }
    // Claimed: task_07, task_08
    for tid in &["task_07", "task_08"] {
        let sess = task_session(tid);
        let _ = state
            .runtime
            .tasks
            .claim(
                Some(&sess),
                &TaskId::new(*tid),
                "demo-worker".to_owned(),
                300_000,
            )
            .await;
    }
    // task_09, task_12 remain queued
    // Fail task_10, cancel task_11
    let sess_10 = task_session("task_10");
    let _ = state
        .runtime
        .tasks
        .claim(
            Some(&sess_10),
            &TaskId::new("task_10"),
            "demo-worker".to_owned(),
            300_000,
        )
        .await;
    let _ = state
        .runtime
        .tasks
        .start(Some(&sess_10), &TaskId::new("task_10"))
        .await;
    let _ = state
        .runtime
        .tasks
        .fail(
            Some(&sess_10),
            &TaskId::new("task_10"),
            FailureClass::ExecutionError,
        )
        .await;
    let sess_11 = task_session("task_11");
    let _ = state
        .runtime
        .tasks
        .cancel(Some(&sess_11), &TaskId::new("task_11"))
        .await;

    // ── 3 Approvals ───────────────────────────────────────────────────────────
    // appr_01: pending (run_c)
    // appr_02: approved (run_a)
    // appr_03: rejected (run_d)
    let appr_defs: &[(&str, &str)] = &[
        ("appr_01", "run_c"),
        ("appr_02", "run_a"),
        ("appr_03", "run_d"),
    ];
    for (aid, rid) in appr_defs {
        if let Err(e) = state
            .runtime
            .approvals
            .request(
                &project,
                ApprovalId::new(*aid),
                Some(RunId::new(*rid)),
                None,
                ApprovalRequirement::Required,
            )
            .await
        {
            tracing::warn!("seed: approval {aid}: {e}");
        }
    }
    let _ = state
        .runtime
        .approvals
        .resolve(&ApprovalId::new("appr_02"), ApprovalDecision::Approved)
        .await;
    let _ = state
        .runtime
        .approvals
        .resolve(&ApprovalId::new("appr_03"), ApprovalDecision::Rejected)
        .await;

    // ── 10 Audit log entries ──────────────────────────────────────────────────
    let audit_entries: &[(&str, &str, &str, &str, AuditOutcome)] = &[
        (
            "operator",
            "create_session",
            "session",
            "sess_alpha",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "create_session",
            "session",
            "sess_beta",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "create_session",
            "session",
            "sess_gamma",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "start_run",
            "run",
            "run_a",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "start_run",
            "run",
            "run_c",
            AuditOutcome::Success,
        ),
        (
            "demo-worker",
            "complete_run",
            "run",
            "run_a",
            AuditOutcome::Success,
        ),
        (
            "demo-worker",
            "fail_run",
            "run",
            "run_d",
            AuditOutcome::Failure,
        ),
        (
            "operator",
            "pause_run",
            "run",
            "run_e",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "approve",
            "approval",
            "appr_02",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "reject",
            "approval",
            "appr_03",
            AuditOutcome::Success,
        ),
    ];
    for (actor, action, rtype, rid, outcome) in audit_entries {
        if let Err(e) = state
            .runtime
            .audits
            .record(
                tenant.clone(),
                (*actor).to_owned(),
                (*action).to_owned(),
                (*rtype).to_owned(),
                (*rid).to_owned(),
                *outcome,
                serde_json::json!({"source": "demo_seed"}),
            )
            .await
        {
            tracing::warn!("seed: audit {action}/{rid}: {e}");
        }
    }

    tracing::info!(
        "seed: demo data ready — {} sessions, {} runs, {} tasks, {} approvals, {} audit entries",
        s_ids.len(),
        run_defs.len(),
        task_defs.len(),
        appr_defs.len(),
        audit_entries.len(),
    );
}
