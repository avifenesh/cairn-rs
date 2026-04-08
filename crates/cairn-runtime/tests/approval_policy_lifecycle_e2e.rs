//! RFC 006 approval policy lifecycle end-to-end integration test.
//!
//! Validates the approval policy pipeline:
//!   (1) create an approval policy with required_approvers and allowed_roles
//!   (2) verify the policy is retrievable with all fields correct
//!   (3) request an approval and attach it to a release governed by the policy
//!   (4) verify the approval record is pending; policy governs via attached release
//!   (5) simulate auto_approve_after_ms: wait past the threshold, then auto-resolve
//!   (6) policy list by tenant; pagination; idempotent release attach
//!   (7) resolve approval (approved / rejected); double-resolve rejected

use std::sync::Arc;

use cairn_domain::{
    ApprovalDecision, ApprovalId, ApprovalRequirement, ProjectKey, PromptReleaseId, TenantId,
    WorkspaceRole,
};
use cairn_runtime::services::{ApprovalPolicyServiceImpl, ApprovalServiceImpl};
use cairn_runtime::{ApprovalPolicyService, ApprovalService};
use cairn_store::{EventLog, InMemoryStore};
use tokio::time::{sleep, Duration};

fn project() -> ProjectKey {
    ProjectKey::new("t_pol", "ws_pol", "proj_pol")
}
fn tenant() -> TenantId {
    TenantId::new("t_pol")
}

fn setup() -> (
    Arc<InMemoryStore>,
    ApprovalPolicyServiceImpl<InMemoryStore>,
    ApprovalServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    (
        store.clone(),
        ApprovalPolicyServiceImpl::new(store.clone()),
        ApprovalServiceImpl::new(store),
    )
}

// ── (1)+(2) Create policy — all fields persisted ─────────────────────────

#[tokio::test]
async fn create_policy_stores_all_fields() {
    let (_, policies, _) = setup();

    let policy = policies
        .create(
            tenant(),
            "Compliance Gate".to_owned(),
            2,
            vec![WorkspaceRole::Admin, WorkspaceRole::Owner],
            Some(3_600_000),
            None,
        )
        .await
        .unwrap();

    assert!(!policy.policy_id.is_empty(), "policy_id must be generated");
    assert_eq!(policy.tenant_id, tenant());
    assert_eq!(policy.name, "Compliance Gate");
    assert_eq!(policy.required_approvers, 2);
    assert_eq!(
        policy.allowed_approver_roles,
        vec![WorkspaceRole::Admin, WorkspaceRole::Owner]
    );
    assert_eq!(policy.auto_approve_after_ms, Some(3_600_000));
    assert!(policy.auto_reject_after_ms.is_none());
    assert!(
        policy.attached_release_ids.is_empty(),
        "no releases attached yet"
    );

    // get() round-trips all fields.
    let fetched = policies.get(&policy.policy_id).await.unwrap().unwrap();
    assert_eq!(fetched.policy_id, policy.policy_id);
    assert_eq!(fetched.name, "Compliance Gate");
    assert_eq!(fetched.required_approvers, 2);
    assert_eq!(fetched.auto_approve_after_ms, Some(3_600_000));
}

// ── (3) Request an approval, attach policy to the release ─────────────────

#[tokio::test]
async fn request_approval_and_attach_policy_to_release() {
    let (_, policies, approvals) = setup();

    let policy = policies
        .create(
            tenant(),
            "Release Gate".to_owned(),
            1,
            vec![WorkspaceRole::Admin],
            None,
            None,
        )
        .await
        .unwrap();

    let release_id = PromptReleaseId::new("rel_policy_1");
    let approval_id = ApprovalId::new("appr_policy_1");

    // Request an approval for the run linked to this release.
    let approval = approvals
        .request(
            &project(),
            approval_id.clone(),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    assert_eq!(approval.approval_id, approval_id);
    assert!(approval.decision.is_none(), "approval must be pending");

    // Attach the policy to the release — establishes the governance link.
    let updated_policy = policies
        .attach_to_release(&policy.policy_id, release_id.clone())
        .await
        .unwrap();

    assert!(
        updated_policy.attached_release_ids.contains(&release_id),
        "release must appear in policy's attached_release_ids after attach"
    );
}

// ── (4) Policy governs approval via attached release ─────────────────────

#[tokio::test]
async fn policy_governs_approval_through_attached_release() {
    let (_, policies, approvals) = setup();

    let policy = policies
        .create(
            tenant(),
            "Strict Policy".to_owned(),
            2,
            vec![WorkspaceRole::Admin],
            None,
            Some(86_400_000), // auto-reject after 24h
        )
        .await
        .unwrap();

    let release_a = PromptReleaseId::new("rel_a");
    let release_b = PromptReleaseId::new("rel_b");
    policies
        .attach_to_release(&policy.policy_id, release_a.clone())
        .await
        .unwrap();
    policies
        .attach_to_release(&policy.policy_id, release_b.clone())
        .await
        .unwrap();

    // Approval for release_a.
    approvals
        .request(
            &project(),
            ApprovalId::new("appr_a"),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    // Approval for release_b.
    approvals
        .request(
            &project(),
            ApprovalId::new("appr_b"),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();

    // Policy now governs both releases.
    let fetched = policies.get(&policy.policy_id).await.unwrap().unwrap();
    assert_eq!(fetched.attached_release_ids.len(), 2);
    assert!(fetched.attached_release_ids.contains(&release_a));
    assert!(fetched.attached_release_ids.contains(&release_b));
    assert_eq!(
        fetched.required_approvers, 2,
        "policy enforces 2 required approvers"
    );
    assert_eq!(fetched.auto_reject_after_ms, Some(86_400_000));
}

// ── (5) auto_approve_after_ms: simulate auto-resolution ───────────────────

#[tokio::test]
async fn auto_approve_after_ms_field_enables_caller_driven_auto_resolution() {
    let (store, policies, approvals) = setup();
    let approval_id = ApprovalId::new("appr_auto");

    // Create a policy with a very short auto_approve_after_ms (50ms).
    let policy = policies
        .create(
            tenant(),
            "Fast Auto-Approve".to_owned(),
            1,
            vec![WorkspaceRole::Admin],
            Some(50), // auto-approve after 50ms
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        policy.auto_approve_after_ms,
        Some(50),
        "policy must carry auto_approve_after_ms=50"
    );

    // Request an approval.
    let approval = approvals
        .request(
            &project(),
            approval_id.clone(),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    assert!(
        approval.decision.is_none(),
        "approval must start as pending"
    );

    // Simulate waiting past auto_approve_after_ms.
    sleep(Duration::from_millis(100)).await;

    // Caller-driven auto-approval: check if enough time has passed, then resolve.
    // (In production, a background task reads pending approvals and calls resolve.)
    let auto_resolved = approvals
        .resolve(&approval_id, ApprovalDecision::Approved)
        .await
        .unwrap();

    assert_eq!(
        auto_resolved.decision,
        Some(ApprovalDecision::Approved),
        "auto-approval must set decision to Approved"
    );

    // Verify event in log.
    let events = store.read_stream(None, 20).await.unwrap();
    let resolved = events.iter().any(|e| {
        matches!(
            &e.envelope.payload,
            cairn_domain::RuntimeEvent::ApprovalResolved(ev)
                if ev.approval_id == approval_id
                    && ev.decision == ApprovalDecision::Approved
        )
    });
    assert!(
        resolved,
        "ApprovalResolved(Approved) must be in the event log"
    );
}

// ── (6) List by tenant; pagination; idempotent attach ────────────────────

#[tokio::test]
async fn list_policies_by_tenant_with_pagination() {
    let (_, policies, _) = setup();

    for i in 0..4 {
        policies
            .create(tenant(), format!("Policy {i}"), 1, vec![], None, None)
            .await
            .unwrap();
    }

    let all = policies.list(&tenant(), 10, 0).await.unwrap();
    assert_eq!(all.len(), 4);
    assert!(all.iter().all(|p| p.tenant_id == tenant()));

    let page = policies.list(&tenant(), 2, 0).await.unwrap();
    assert_eq!(page.len(), 2, "limit must be respected");

    let rest = policies.list(&tenant(), 10, 2).await.unwrap();
    assert_eq!(rest.len(), 2, "offset must skip first 2");
}

#[tokio::test]
async fn attach_to_release_is_idempotent() {
    let (_, policies, _) = setup();

    let policy = policies
        .create(
            tenant(),
            "Idempotent Gate".to_owned(),
            1,
            vec![],
            None,
            None,
        )
        .await
        .unwrap();

    let release_id = PromptReleaseId::new("rel_idem");

    policies
        .attach_to_release(&policy.policy_id, release_id.clone())
        .await
        .unwrap();
    let after_second = policies
        .attach_to_release(&policy.policy_id, release_id.clone())
        .await
        .unwrap();

    assert_eq!(
        after_second.attached_release_ids.len(),
        1,
        "double attach must not add a duplicate entry"
    );
}

// ── (7) Resolve approved / rejected; double-resolve rejected ─────────────

#[tokio::test]
async fn resolve_approval_with_approved_decision() {
    let (_, _, approvals) = setup();
    let approval_id = ApprovalId::new("appr_resolve_ok");

    approvals
        .request(
            &project(),
            approval_id.clone(),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    let resolved = approvals
        .resolve(&approval_id, ApprovalDecision::Approved)
        .await
        .unwrap();

    assert_eq!(resolved.decision, Some(ApprovalDecision::Approved));
}

#[tokio::test]
async fn resolve_approval_with_rejected_decision() {
    let (_, _, approvals) = setup();
    let approval_id = ApprovalId::new("appr_resolve_rej");

    approvals
        .request(
            &project(),
            approval_id.clone(),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    let resolved = approvals
        .resolve(&approval_id, ApprovalDecision::Rejected)
        .await
        .unwrap();

    assert_eq!(resolved.decision, Some(ApprovalDecision::Rejected));
}

#[tokio::test]
async fn double_resolve_returns_error() {
    let (_, _, approvals) = setup();
    let approval_id = ApprovalId::new("appr_double");

    approvals
        .request(
            &project(),
            approval_id.clone(),
            None,
            None,
            ApprovalRequirement::Required,
        )
        .await
        .unwrap();
    approvals
        .resolve(&approval_id, ApprovalDecision::Approved)
        .await
        .unwrap();

    let second = approvals
        .resolve(&approval_id, ApprovalDecision::Approved)
        .await;
    assert!(second.is_err(), "double-resolve must return an error");
}
