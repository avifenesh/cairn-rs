//! Shared claim helper for `FabricTaskService` and `FabricRunService`.
//!
//! Both services issue the same two-step FCALL sequence
//! (`ff_issue_claim_grant` → `ff_claim_execution`, with transparent
//! dispatch to `ff_claim_resumed_execution` for attempt-interrupted
//! executions). Phase D PR 2a moved the FCALL machinery behind
//! [`ControlPlaneBackend::issue_grant_and_claim`]; this module stays
//! as a thin orientation point so call sites keep their familiar
//! name and the docstring above the call continues to describe the
//! two-step protocol.
//!
//! **No FF-state-plane imports live here.** The `ff_core::keys` /
//! `ff_core::partition` machinery moved into the backend impl
//! (`engine/valkey_control_plane_impl.rs`). This file owns nothing
//! except the delegate.

use std::sync::Arc;

use ff_core::types::{ExecutionId, LaneId};

use crate::engine::{ClaimGrantOutcome, ControlPlaneBackend, IssueGrantAndClaimInput};
use crate::error::FabricError;

/// Re-export the typed claim outcome under the historical name so
/// call sites don't have to learn a new type alias. The service
/// intentionally does NOT consume the lease triple; the struct
/// exists for tests / debug logs and FF-contract continuity (see
/// [`ClaimGrantOutcome`] for the retention rationale).
pub type ClaimOutcome = ClaimGrantOutcome;

/// Execute the `ff_issue_claim_grant` + `ff_claim_execution` FCALL
/// pair via [`ControlPlaneBackend`].
///
/// Cancel-safety: a drop between the two FCALLs leaves only a grant,
/// which FF expires via its grant TTL. The returned `ClaimOutcome`
/// is intentionally dropped by both call sites — cairn does NOT
/// cache the lease triple. Every downstream terminal op re-reads
/// `current_lease_id` / `_epoch` / `_attempt_index` from FF's
/// `exec_core` on demand.
pub(crate) async fn issue_grant_and_claim(
    control_plane: &Arc<dyn ControlPlaneBackend>,
    eid: &ExecutionId,
    lane_id: &LaneId,
    lease_duration_ms: u64,
) -> Result<ClaimOutcome, FabricError> {
    control_plane
        .issue_grant_and_claim(IssueGrantAndClaimInput {
            execution_id: eid.clone(),
            lane_id: lane_id.clone(),
            lease_duration_ms,
        })
        .await
}
