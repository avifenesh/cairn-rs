//! RFC 006 prompt approval workflow integration tests.

use std::sync::Arc;

use cairn_domain::{ApprovalDecision, ApprovalId, ProjectKey, PromptAssetId, PromptReleaseId, PromptVersionId, TenantId, WorkspaceRole};
use cairn_runtime::{PromptReleaseService, PromptReleaseServiceImpl};
use cairn_runtime::approval_policies::ApprovalPolicyService;
use cairn_runtime::approvals::ApprovalService;
use cairn_runtime::services::{ApprovalPolicyServiceImpl, ApprovalServiceImpl};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("tenant", "workspace", "project")
}
fn tenant() -> TenantId {
    TenantId::new("tenant")
}
fn asset() -> PromptAssetId {
    PromptAssetId::new("asset_approval_1")
}

async fn setup() -> (
    Arc<InMemoryStore>,
    PromptReleaseServiceImpl<InMemoryStore>,
    ApprovalPolicyServiceImpl<InMemoryStore>,
    ApprovalServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    let releases = PromptReleaseServiceImpl::new(store.clone());
    let policies = ApprovalPolicyServiceImpl::new(store.clone());
    let approvals = ApprovalServiceImpl::new(store.clone());
    (store, releases, policies, approvals)
}

/// Full happy path:
/// 1. Attach a policy (required_approvers=1) to a release.
/// 2. Try to activate → blocked.
/// 3. Request approval → get ApprovalRecord.
/// 4. Resolve as Approved (via ApprovalService).
/// 5. Activate → succeeds.
#[tokio::test]
async fn prompt_approval_flow_full_happy_path() {
    let (_store, releases, policies, approvals) = setup().await;

    // Create the policy.
    let policy = policies
        .create(
            tenant(),
            "Requires 1 approver".to_owned(),
            1,
            vec![WorkspaceRole::Admin],
            None,
            None,
        )
        .await
        .unwrap();

    // Create and advance a release to approved state.
    let release_id = PromptReleaseId::new("rel_approval_test");
    releases
        .create(&project(), release_id.clone(), asset(), PromptVersionId::new("v1"))
        .await
        .unwrap();
    releases.transition(&release_id, "approved").await.unwrap();

    // Attach the approval policy.
    releases
        .attach_approval_policy(&release_id, &policy.policy_id)
        .await
        .unwrap();

    // Try activate — must be blocked because no approval requested yet.
    let blocked = releases.activate(&release_id).await;
    assert!(
        blocked.is_err(),
        "activate must be blocked when policy is attached and no approval requested"
    );
    let err_msg = blocked.unwrap_err().to_string();
    assert!(
        err_msg.contains("approval") || err_msg.contains("Approval"),
        "error must mention approval, got: {err_msg}"
    );

    // Request approval.
    let approval_record = releases.request_approval(&release_id).await.unwrap();
    assert!(approval_record.decision.is_none(), "new approval must be pending");

    // Try activate again — still blocked (approval is still pending).
    let still_blocked = releases.activate(&release_id).await;
    assert!(still_blocked.is_err(), "activate must be blocked while approval is pending");

    // Resolve the approval as Approved.
    approvals
        .resolve(
            &approval_record.approval_id,
            ApprovalDecision::Approved,
        )
        .await
        .unwrap();

    // Now activate — must succeed.
    let activated = releases.activate(&release_id).await.unwrap();
    assert_eq!(activated.state, "active");
}

/// Activate is blocked until the approval is resolved.
#[tokio::test]
async fn prompt_approval_flow_activation_blocked_until_approved() {
    let (_store, releases, policies, approvals) = setup().await;

    let policy = policies
        .create(tenant(), "RequireApproval".to_owned(), 1, vec![], None, None)
        .await
        .unwrap();

    let release_id = PromptReleaseId::new("rel_block_test");
    releases
        .create(&project(), release_id.clone(), PromptAssetId::new("asset_block"), PromptVersionId::new("v1"))
        .await
        .unwrap();
    releases.transition(&release_id, "approved").await.unwrap();
    releases.attach_approval_policy(&release_id, &policy.policy_id).await.unwrap();

    // Request approval — get approval_id back.
    let approval = releases.request_approval(&release_id).await.unwrap();

    // Pending → blocked.
    assert!(releases.activate(&release_id).await.is_err());

    // Approve → unblocked.
    approvals.resolve(&approval.approval_id, ApprovalDecision::Approved).await.unwrap();
    let record = releases.activate(&release_id).await.unwrap();
    assert_eq!(record.state, "active");
}

/// A rejected approval prevents activation.
#[tokio::test]
async fn prompt_approval_flow_rejected_approval_prevents_activation() {
    let (_store, releases, policies, approvals) = setup().await;

    let policy = policies
        .create(tenant(), "RejectTest".to_owned(), 1, vec![], None, None)
        .await
        .unwrap();

    let release_id = PromptReleaseId::new("rel_reject_test");
    releases
        .create(&project(), release_id.clone(), PromptAssetId::new("asset_reject"), PromptVersionId::new("v1"))
        .await
        .unwrap();
    releases.transition(&release_id, "approved").await.unwrap();
    releases.attach_approval_policy(&release_id, &policy.policy_id).await.unwrap();

    let approval = releases.request_approval(&release_id).await.unwrap();
    approvals.resolve(&approval.approval_id, ApprovalDecision::Rejected).await.unwrap();

    let result = releases.activate(&release_id).await;
    assert!(result.is_err(), "rejected approval must block activation");
    assert!(
        result.unwrap_err().to_string().contains("reject"),
        "error should mention rejection"
    );
}

/// Without any policy attached, activation proceeds without approval checks.
#[tokio::test]
async fn prompt_approval_flow_no_policy_activates_without_approval() {
    let (_store, releases, _policies, _approvals) = setup().await;

    let release_id = PromptReleaseId::new("rel_no_policy");
    releases
        .create(&project(), release_id.clone(), PromptAssetId::new("asset_nopol"), PromptVersionId::new("v1"))
        .await
        .unwrap();
    releases.transition(&release_id, "approved").await.unwrap();

    // No policy attached — activate should succeed immediately.
    let record = releases.activate(&release_id).await.unwrap();
    assert_eq!(record.state, "active");
}

/// request_approval returns a valid ApprovalRecord linked to the release.
#[tokio::test]
async fn prompt_approval_flow_request_returns_pending_approval() {
    let (_store, releases, policies, _approvals) = setup().await;

    let policy = policies
        .create(tenant(), "PendingTest".to_owned(), 1, vec![], None, None)
        .await
        .unwrap();

    let release_id = PromptReleaseId::new("rel_pending_test");
    releases
        .create(&project(), release_id.clone(), PromptAssetId::new("asset_pending"), PromptVersionId::new("v1"))
        .await
        .unwrap();
    releases.attach_approval_policy(&release_id, &policy.policy_id).await.unwrap();

    let approval = releases.request_approval(&release_id).await.unwrap();

    assert!(approval.decision.is_none(), "new approval must be pending (no decision)");
    assert!(!approval.approval_id.as_str().is_empty(), "approval_id must be set");
}
