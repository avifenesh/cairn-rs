//! Integration tests for RFC 022: Triggers — Binding Signals to Runs.

use cairn_domain::decisions::RunMode;
use cairn_domain::ids::{OperatorId, RunTemplateId, SignalId, TriggerId};
use cairn_domain::tenancy::ProjectKey;
use cairn_runtime::services::trigger_service::{
    auto_approve_decision, evaluate_conditions, substitute_variables, RateLimitConfig, RunTemplate,
    SignalPattern, SkipReason, TemplateBudget, Trigger, TriggerCondition, TriggerError,
    TriggerEvent, TriggerService, TriggerState,
};
use serde_json::json;

fn operator() -> OperatorId {
    OperatorId::new("test-op")
}

fn project(id: &str) -> ProjectKey {
    ProjectKey::new("acme", "eng", id)
}

fn make_template(id: &str, project: &ProjectKey) -> RunTemplate {
    RunTemplate {
        id: RunTemplateId::new(id),
        project: project.clone(),
        name: format!("Template {id}"),
        description: None,
        default_mode: RunMode::Direct,
        system_prompt: "You are responding to {{action}} on issue #{{issue.number}} in {{repository.full_name}}.\nThe issue title is: {{issue.title}}\nLabels: {{issue.labels[].name}}".into(),
        initial_user_message: None,
        plugin_allowlist: None,
        tool_allowlist: None,
        budget: TemplateBudget::default(),
        sandbox_hint: None,
        required_fields: vec!["issue.number".into()],
        created_by: operator(),
        created_at: 0,
        updated_at: 0,
    }
}

fn make_trigger(id: &str, template_id: &str, project: &ProjectKey) -> Trigger {
    Trigger {
        id: TriggerId::new(id),
        project: project.clone(),
        name: format!("Trigger {id}"),
        description: Some("Test trigger".into()),
        signal_pattern: SignalPattern {
            signal_type: "github.issue.labeled".into(),
            plugin_id: Some("github".into()),
        },
        conditions: vec![TriggerCondition::Contains {
            path: "issue.labels[].name".into(),
            value: json!("cairn-ready"),
        }],
        run_template_id: RunTemplateId::new(template_id),
        state: TriggerState::Enabled,
        rate_limit: RateLimitConfig::default(),
        max_chain_depth: 5,
        created_by: operator(),
        created_at: 0,
        updated_at: 0,
    }
}

fn github_payload() -> serde_json::Value {
    json!({
        "action": "labeled",
        "issue": {
            "number": 42,
            "title": "Fix login bug",
            "labels": [{"name": "bug"}, {"name": "cairn-ready"}],
            "body": "The login page crashes on mobile"
        },
        "label": {"name": "cairn-ready"},
        "repository": {"full_name": "org/dogfood"},
        "sender": {"login": "alice"}
    })
}

// ── RFC 022 Test 1: Create + enable + fire ──────────────────────────────────

#[test]
fn rfc022_test1_create_enable_fire() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");

    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-1"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );

    assert_eq!(events.len(), 1);
    if let TriggerEvent::TriggerFired {
        trigger_id,
        signal_type,
        chain_depth,
        ..
    } = &events[0]
    {
        assert_eq!(trigger_id.as_str(), "t1");
        assert_eq!(signal_type, "github.issue.labeled");
        assert_eq!(*chain_depth, 1);
    } else {
        panic!("expected TriggerFired, got {:?}", events[0]);
    }
}

// ── RFC 022 Test 2: Condition mismatch is silent ────────────────────────────

#[test]
fn rfc022_test2_condition_mismatch_skips() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    // Wrong label
    let payload = json!({
        "action": "labeled",
        "issue": {
            "number": 42,
            "labels": [{"name": "bug"}, {"name": "wontfix"}]
        }
    });

    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-2"),
        "github.issue.labeled",
        "github",
        &payload,
        None,
        &auto_approve_decision,
    );

    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        TriggerEvent::TriggerSkipped {
            reason: SkipReason::ConditionMismatch,
            ..
        }
    ));
}

// ── RFC 022 Test 3: Multiple triggers fan out ───────────────────────────────

#[test]
fn rfc022_test3_multiple_triggers_fan_out() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_template(make_template("tmpl-2", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();
    svc.create_trigger(make_trigger("t2", "tmpl-2", &p1))
        .unwrap();

    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-3"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );

    let fired = events
        .iter()
        .filter(|e| matches!(e, TriggerEvent::TriggerFired { .. }))
        .count();
    assert_eq!(fired, 2, "both triggers should fire");
}

// ── RFC 022 Test 4: Cross-project isolation ─────────────────────────────────

#[test]
fn rfc022_test4_cross_project_isolation() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    let p2 = project("p2");

    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    // Signal in p2 should not match p1's trigger
    let events = svc.evaluate_signal(
        &p2,
        &SignalId::new("sig-4"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );

    assert!(events.is_empty(), "p2 has no triggers");
}

// ── RFC 022 Test 6: Fire ledger dedup ───────────────────────────────────────

#[test]
fn rfc022_test6_fire_ledger_dedup() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    let signal_id = SignalId::new("sig-dup");

    // First fires normally
    let events1 = svc.evaluate_signal(
        &p1,
        &signal_id,
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );
    assert!(matches!(&events1[0], TriggerEvent::TriggerFired { .. }));

    // Second with same signal_id is deduped by fire ledger
    let events2 = svc.evaluate_signal(
        &p1,
        &signal_id,
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );
    assert!(matches!(
        &events2[0],
        TriggerEvent::TriggerSkipped {
            reason: SkipReason::AlreadyFired,
            ..
        }
    ));

    // Different signal_id with same payload fires normally
    let events3 = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-different"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );
    assert!(matches!(&events3[0], TriggerEvent::TriggerFired { .. }));
}

// ── RFC 022 Test 7: Rate limit drops excess ─────────────────────────────────

#[test]
fn rfc022_test7_rate_limit_drops_excess() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));

    let mut trigger = make_trigger("t1", "tmpl-1", &p1);
    trigger.rate_limit = RateLimitConfig {
        max_per_minute: 3,
        max_burst: 3,
    };
    svc.create_trigger(trigger).unwrap();

    let mut fired = 0;
    let mut rate_limited = 0;

    for i in 0..6 {
        let events = svc.evaluate_signal(
            &p1,
            &SignalId::new(format!("sig-rate-{i}")),
            "github.issue.labeled",
            "github",
            &github_payload(),
            None,
            &auto_approve_decision,
        );

        for e in &events {
            match e {
                TriggerEvent::TriggerFired { .. } => fired += 1,
                TriggerEvent::TriggerRateLimited { .. } => rate_limited += 1,
                _ => {}
            }
        }
    }

    assert_eq!(fired, 3, "only 3 should fire within the rate limit");
    assert_eq!(rate_limited, 3, "3 should be rate-limited");
}

// ── RFC 022 Test 9: Chain depth cap prevents loops ──────────────────────────

#[test]
fn rfc022_test9_chain_depth_prevents_loops() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));

    let mut trigger = make_trigger("t1", "tmpl-1", &p1);
    trigger.max_chain_depth = 3;
    svc.create_trigger(trigger).unwrap();

    // Depth 3 (source at 2) fires
    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-d3"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        Some(2),
        &auto_approve_decision,
    );
    assert!(matches!(
        &events[0],
        TriggerEvent::TriggerFired { chain_depth: 3, .. }
    ));

    // Depth 4 (source at 3) is too deep
    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-d4"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        Some(3),
        &auto_approve_decision,
    );
    assert!(matches!(
        &events[0],
        TriggerEvent::TriggerSkipped {
            reason: SkipReason::ChainTooDeep,
            ..
        }
    ));
}

// ── RFC 022 Test 10: Variable substitution ──────────────────────────────────

#[test]
fn rfc022_test10_variable_substitution() {
    let payload = github_payload();
    let template = "You are responding to {{action}} on issue #{{issue.number}} in {{repository.full_name}}.\nThe issue title is: {{issue.title}}\nLabels: {{issue.labels[].name}}";

    let result = substitute_variables(template, &payload, &[]).unwrap();

    assert!(result.contains("labeled"));
    assert!(result.contains("#42"));
    assert!(result.contains("org/dogfood"));
    assert!(result.contains("Fix login bug"));
    assert!(result.contains("bug, cairn-ready"));
}

// ── RFC 022 Test 11: Required fields ────────────────────────────────────────

#[test]
fn rfc022_test11_required_fields_skip() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    // Payload missing required "issue.number"
    let payload = json!({
        "action": "labeled",
        "issue": {
            "labels": [{"name": "cairn-ready"}]
        }
    });

    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-missing"),
        "github.issue.labeled",
        "github",
        &payload,
        None,
        &auto_approve_decision,
    );

    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        TriggerEvent::TriggerSkipped {
            reason: SkipReason::MissingRequiredField { field },
            ..
        } if field == "issue.number"
    ));
}

// ── RFC 022 Test 12: Template delete blocked by trigger ─────────────────────

#[test]
fn rfc022_test12_template_delete_blocked() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    // Delete blocked
    let result = svc.delete_template(&RunTemplateId::new("tmpl-1"), operator());
    assert!(matches!(result, Err(TriggerError::TemplateInUse { .. })));

    // Delete trigger first, then template succeeds
    svc.delete_trigger(&TriggerId::new("t1"), operator())
        .unwrap();
    let result = svc.delete_template(&RunTemplateId::new("tmpl-1"), operator());
    assert!(result.is_ok());
}

// ── RFC 022 Test 14: Run carries trigger origin ─────────────────────────────

#[test]
fn rfc022_test14_run_carries_trigger_origin() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-origin"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );

    if let TriggerEvent::TriggerFired {
        trigger_id,
        chain_depth,
        run_id,
        ..
    } = &events[0]
    {
        assert_eq!(trigger_id.as_str(), "t1");
        assert_eq!(*chain_depth, 1);
        assert!(!run_id.as_str().is_empty());
    } else {
        panic!("expected TriggerFired");
    }
}

// ── Trigger enable/disable lifecycle ────────────────────────────────────────

#[test]
fn trigger_enable_disable_resume_lifecycle() {
    let mut svc = TriggerService::new();
    let p1 = project("p1");
    svc.create_template(make_template("tmpl-1", &p1));
    svc.create_trigger(make_trigger("t1", "tmpl-1", &p1))
        .unwrap();

    // Disable
    svc.disable_trigger(
        &TriggerId::new("t1"),
        operator(),
        Some("maintenance".into()),
    )
    .unwrap();

    // Signal should not match disabled trigger
    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-disabled"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );
    assert!(events.is_empty(), "disabled trigger should not fire");

    // Re-enable
    svc.enable_trigger(&TriggerId::new("t1"), operator())
        .unwrap();

    // Now fires again
    let events = svc.evaluate_signal(
        &p1,
        &SignalId::new("sig-reenabled"),
        "github.issue.labeled",
        "github",
        &github_payload(),
        None,
        &auto_approve_decision,
    );
    assert!(matches!(&events[0], TriggerEvent::TriggerFired { .. }));
}
