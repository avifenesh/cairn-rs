//! Guardrail evaluation system end-to-end integration tests.
//!
//! Tests the full policy-evaluation arc:
//!   1. Create a guardrail policy with explicit allow and deny rules
//!   2. Evaluate a request that matches the allow rule — must be Allowed
//!   3. Evaluate a request that matches the deny rule — must be Denied
//!   4. Verify GuardrailPolicyEvaluated events carry correct fields
//!
//! Additional coverage:
//!   - Wildcard (subject_id=None) rules match any subject
//!   - Unmatched requests receive an implicit Allowed decision
//!   - Multiple policies evaluated in order; first match wins
//!   - Block / Redact effects also produce Denied decisions
//!   - No-tenant requests return NotFound

use std::sync::Arc;

use cairn_domain::policy::{
    GuardrailDecisionKind, GuardrailRule, GuardrailRuleEffect, GuardrailSubjectType,
};
use cairn_domain::{RuntimeEvent, TenantId};
use cairn_runtime::error::RuntimeError;
use cairn_runtime::guardrails::GuardrailService;
use cairn_runtime::services::{GuardrailServiceImpl, TenantServiceImpl};
use cairn_runtime::tenants::TenantService;
use cairn_store::{EventLog, InMemoryStore};

fn tenant() -> TenantId {
    TenantId::new("t_guardrail")
}

async fn setup() -> (Arc<InMemoryStore>, GuardrailServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    TenantServiceImpl::new(store.clone())
        .create(tenant(), "Guardrail Tenant".to_owned())
        .await
        .unwrap();
    let svc = GuardrailServiceImpl::new(store.clone());
    (store, svc)
}

// ── Test 1 + 2 + 3: create policy, evaluate allow, evaluate deny ──────────────

/// Create a policy with one explicit allow rule and one explicit deny rule,
/// then verify that evaluation honours both.
#[tokio::test]
async fn create_policy_with_allow_and_deny_rules() {
    let (_store, svc) = setup().await;

    // ── (1) Create a policy with two rules ────────────────────────────────
    let policy = svc
        .create_policy(
            tenant(),
            "tool-access-policy".to_owned(),
            vec![
                // Allow read-only file operations.
                GuardrailRule {
                    subject_type: GuardrailSubjectType::Tool,
                    subject_id: Some("fs.read".to_owned()),
                    action: "invoke".to_owned(),
                    effect: GuardrailRuleEffect::Allow,
                    conditions: vec![],
                },
                // Deny destructive file operations.
                GuardrailRule {
                    subject_type: GuardrailSubjectType::Tool,
                    subject_id: Some("fs.delete".to_owned()),
                    action: "invoke".to_owned(),
                    effect: GuardrailRuleEffect::Deny,
                    conditions: vec![],
                },
            ],
        )
        .await
        .unwrap();

    assert!(!policy.policy_id.is_empty(), "policy must have a non-empty ID");
    assert_eq!(policy.name, "tool-access-policy");
    assert_eq!(policy.rules.len(), 2, "policy must contain both rules");
    assert!(policy.enabled, "newly created policy must be enabled");

    // ── (2) Allowed request: fs.read matches the Allow rule ───────────────
    let allow_decision = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Tool,
            Some("fs.read".to_owned()),
            "invoke".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(
        allow_decision.decision,
        GuardrailDecisionKind::Allowed,
        "fs.read/invoke must be Allowed by the explicit allow rule"
    );
    assert_eq!(
        allow_decision.policy_id.as_deref(),
        Some(policy.policy_id.as_str()),
        "decision must reference the matched policy"
    );
    assert!(
        allow_decision.reason.is_some(),
        "allow decision must carry a reason string"
    );

    // ── (3) Denied request: fs.delete matches the Deny rule ──────────────
    let deny_decision = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Tool,
            Some("fs.delete".to_owned()),
            "invoke".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(
        deny_decision.decision,
        GuardrailDecisionKind::Denied,
        "fs.delete/invoke must be Denied by the explicit deny rule"
    );
    assert_eq!(
        deny_decision.policy_id.as_deref(),
        Some(policy.policy_id.as_str()),
        "denial must reference the matched policy"
    );
    let deny_reason = deny_decision.reason.as_deref().unwrap_or("");
    assert!(
        deny_reason.contains("deny") || deny_reason.contains("invoke"),
        "denial reason must mention the effect or action; got: '{deny_reason}'"
    );
}

// ── Test 4: verify GuardrailPolicyEvaluated event fields ──────────────────────

/// Every evaluate() call must emit a GuardrailPolicyEvaluated event.
/// The event must carry the correct tenant_id, subject_type, subject_id,
/// action, decision, and policy_id fields.
#[tokio::test]
async fn evaluate_emits_guardrail_policy_evaluated_event() {
    let (store, svc) = setup().await;

    svc.create_policy(
        tenant(),
        "event-check-policy".to_owned(),
        vec![GuardrailRule {
            subject_type: GuardrailSubjectType::Provider,
            subject_id: Some("openai".to_owned()),
            action: "call".to_owned(),
            effect: GuardrailRuleEffect::Deny,
            conditions: vec![],
        }],
    )
    .await
    .unwrap();

    let decision = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Provider,
            Some("openai".to_owned()),
            "call".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(decision.decision, GuardrailDecisionKind::Denied);

    // Read all events and find the GuardrailPolicyEvaluated event.
    let events = store.read_stream(None, 1_000).await.unwrap();

    let eval_event = events.iter().find_map(|e| {
        if let RuntimeEvent::GuardrailPolicyEvaluated(ev) = &e.envelope.payload {
            if ev.action == "call"
                && ev.subject_id.as_deref() == Some("openai")
                && ev.subject_type == GuardrailSubjectType::Provider
            {
                return Some(ev.clone());
            }
        }
        None
    });

    let ev = eval_event.expect("GuardrailPolicyEvaluated event must be emitted after evaluate()");

    assert_eq!(ev.tenant_id, tenant(), "event must carry the correct tenant_id");
    assert_eq!(ev.action, "call", "event must carry the correct action");
    assert_eq!(
        ev.subject_id.as_deref(),
        Some("openai"),
        "event must carry the correct subject_id"
    );
    assert_eq!(
        ev.subject_type,
        GuardrailSubjectType::Provider,
        "event must carry the correct subject_type"
    );
    assert_eq!(
        ev.decision,
        GuardrailDecisionKind::Denied,
        "event must reflect the Denied decision"
    );
    assert!(
        !ev.policy_id.is_empty(),
        "event must reference a non-empty policy_id"
    );
    assert!(
        ev.evaluated_at_ms > 0,
        "event must carry a non-zero evaluated_at_ms timestamp"
    );
}

// ── Unmatched request: implicit Allowed, policy_id = "implicit_allow" ─────────

/// When no policy rule matches, evaluate() must return Allowed and emit an
/// event with policy_id = "implicit_allow" (the default sentinel value).
#[tokio::test]
async fn unmatched_request_receives_implicit_allow() {
    let (store, svc) = setup().await;

    svc.create_policy(
        tenant(),
        "narrow-policy".to_owned(),
        vec![GuardrailRule {
            subject_type: GuardrailSubjectType::Tool,
            subject_id: Some("dangerous.tool".to_owned()),
            action: "invoke".to_owned(),
            effect: GuardrailRuleEffect::Deny,
            conditions: vec![],
        }],
    )
    .await
    .unwrap();

    // This action is not covered by any rule.
    let decision = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Tool,
            Some("safe.tool".to_owned()),
            "invoke".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(
        decision.decision,
        GuardrailDecisionKind::Allowed,
        "unmatched action must receive implicit Allowed"
    );
    assert!(
        decision.policy_id.is_none(),
        "implicit allow must not reference a specific policy"
    );

    // Event must use the "implicit_allow" sentinel policy_id.
    let events = store.read_stream(None, 1_000).await.unwrap();
    let ev = events.iter().find_map(|e| {
        if let RuntimeEvent::GuardrailPolicyEvaluated(ev) = &e.envelope.payload {
            if ev.subject_id.as_deref() == Some("safe.tool") {
                return Some(ev.clone());
            }
        }
        None
    });
    let ev = ev.expect("GuardrailPolicyEvaluated must be emitted for every evaluate()");
    assert_eq!(
        ev.policy_id, "implicit_allow",
        "implicit allow event must use 'implicit_allow' as policy_id sentinel"
    );
    assert_eq!(ev.decision, GuardrailDecisionKind::Allowed);
}

// ── Wildcard rule (subject_id = None) matches any subject ─────────────────────

/// A rule with subject_id=None must match any subject, acting as a
/// blanket allow/deny for the subject type + action combination.
#[tokio::test]
async fn wildcard_rule_matches_any_subject_id() {
    let (_store, svc) = setup().await;

    svc.create_policy(
        tenant(),
        "blanket-deny-policy".to_owned(),
        vec![GuardrailRule {
            subject_type: GuardrailSubjectType::Session,
            subject_id: None, // matches ALL sessions
            action: "terminate".to_owned(),
            effect: GuardrailRuleEffect::Deny,
            conditions: vec![],
        }],
    )
    .await
    .unwrap();

    // Any session ID must be blocked.
    for session_id in ["sess_001", "sess_002", "sess_any_value"] {
        let d = svc
            .evaluate(
                tenant(),
                GuardrailSubjectType::Session,
                Some(session_id.to_owned()),
                "terminate".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(
            d.decision,
            GuardrailDecisionKind::Denied,
            "wildcard deny rule must block session '{session_id}'"
        );
    }

    // A different action on the same subject type must still be allowed.
    let other_action = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Session,
            Some("sess_001".to_owned()),
            "inspect".to_owned(),
        )
        .await
        .unwrap();
    assert_eq!(
        other_action.decision,
        GuardrailDecisionKind::Allowed,
        "wildcard deny must not block unrelated actions"
    );
}

// ── Block and Redact effects both produce Denied ───────────────────────────────

/// Block and Redact effects must map to GuardrailDecisionKind::Denied,
/// not Warned or Allowed.
#[tokio::test]
async fn block_and_redact_effects_produce_denied() {
    let (_store, svc) = setup().await;

    svc.create_policy(
        tenant(),
        "multi-effect-policy".to_owned(),
        vec![
            GuardrailRule {
                subject_type: GuardrailSubjectType::Tool,
                subject_id: Some("tool.block".to_owned()),
                action: "invoke".to_owned(),
                effect: GuardrailRuleEffect::Block,
                conditions: vec![],
            },
            GuardrailRule {
                subject_type: GuardrailSubjectType::Tool,
                subject_id: Some("tool.redact".to_owned()),
                action: "invoke".to_owned(),
                effect: GuardrailRuleEffect::Redact,
                conditions: vec![],
            },
        ],
    )
    .await
    .unwrap();

    let block = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Tool,
            Some("tool.block".to_owned()),
            "invoke".to_owned(),
        )
        .await
        .unwrap();
    assert_eq!(
        block.decision,
        GuardrailDecisionKind::Denied,
        "Block effect must produce Denied decision"
    );

    let redact = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Tool,
            Some("tool.redact".to_owned()),
            "invoke".to_owned(),
        )
        .await
        .unwrap();
    assert_eq!(
        redact.decision,
        GuardrailDecisionKind::Denied,
        "Redact effect must produce Denied decision"
    );
}

// ── No-tenant request returns NotFound ────────────────────────────────────────

/// create_policy and evaluate must both return NotFound when called with
/// a tenant that has not been registered.
#[tokio::test]
async fn missing_tenant_returns_not_found() {
    let store = Arc::new(InMemoryStore::new());
    let svc = GuardrailServiceImpl::new(store);
    let ghost = TenantId::new("ghost_tenant");

    let create_err = svc
        .create_policy(ghost.clone(), "p".to_owned(), vec![])
        .await
        .unwrap_err();
    assert!(
        matches!(create_err, RuntimeError::NotFound { entity: "tenant", .. }),
        "create_policy for missing tenant must return NotFound; got: {create_err:?}"
    );

    let eval_err = svc
        .evaluate(
            ghost,
            GuardrailSubjectType::Tool,
            None,
            "invoke".to_owned(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(eval_err, RuntimeError::NotFound { entity: "tenant", .. }),
        "evaluate for missing tenant must return NotFound; got: {eval_err:?}"
    );
}

// ── Multiple policies: first match wins ───────────────────────────────────────

/// When two policies both match the same subject+action, the first
/// matching rule (across policies in list order) must take precedence.
#[tokio::test]
async fn first_matching_policy_wins() {
    let (_store, svc) = setup().await;
    use std::time::Duration;

    // Policy A: denies tool.contested
    svc.create_policy(
        tenant(),
        "policy-a-deny".to_owned(),
        vec![GuardrailRule {
            subject_type: GuardrailSubjectType::Tool,
            subject_id: Some("tool.contested".to_owned()),
            action: "invoke".to_owned(),
            effect: GuardrailRuleEffect::Deny,
            conditions: vec![],
        }],
    )
    .await
    .unwrap();

    // Ensure distinct policy IDs (they're timestamp-based).
    tokio::time::sleep(Duration::from_millis(2)).await;

    // Policy B: explicitly allows tool.contested
    svc.create_policy(
        tenant(),
        "policy-b-allow".to_owned(),
        vec![GuardrailRule {
            subject_type: GuardrailSubjectType::Tool,
            subject_id: Some("tool.contested".to_owned()),
            action: "invoke".to_owned(),
            effect: GuardrailRuleEffect::Allow,
            conditions: vec![],
        }],
    )
    .await
    .unwrap();

    // The first policy (deny) must win.
    let decision = svc
        .evaluate(
            tenant(),
            GuardrailSubjectType::Tool,
            Some("tool.contested".to_owned()),
            "invoke".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(
        decision.decision,
        GuardrailDecisionKind::Denied,
        "first matching policy must win; policy-a-deny must take precedence over policy-b-allow"
    );
}
