//! Shared FCALL sequence for claiming an FF execution.
//!
//! Both `FabricTaskService::claim` and `FabricRunService::claim` follow the
//! same two-step protocol:
//!
//!   1. `ff_issue_claim_grant` — reserves the execution for this worker
//!      instance with a bounded TTL. Dropped grants expire automatically.
//!   2. `ff_claim_execution` — upgrades the grant into an active lease,
//!      allocates the next attempt slot, and flips `lifecycle_phase` to
//!      `active` so downstream FCALLs (`ff_suspend_execution`,
//!      `ff_complete_execution`, etc.) accept the execution.
//!
//! FF owns the wire format. This module owns nothing except the ordering of
//! two FCALLs; do not add retry policy, caching, or state here — every
//! field we consume is read back from Valkey by the caller, never cached.

use std::sync::Arc;

use ff_core::keys::{ExecKeyContext, IndexKeys};
use ff_core::types::{AttemptIndex, ExecutionId, LaneId, LeaseEpoch, LeaseId};

use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::helpers::{check_fcall_success, fcall_error_code};

/// FF's typed dispatch error: the execution's `attempt_state` is
/// `attempt_interrupted` (i.e. this is a resume-from-suspension claim), so
/// the caller must use `ff_claim_resumed_execution` instead of
/// `ff_claim_execution`. See `lua/execution.lua:382-384`.
const USE_CLAIM_RESUMED_EXECUTION: &str = "use_claim_resumed_execution";

/// Result of a successful claim.
///
/// Cairn does NOT consume the lease triple — every downstream terminal op
/// re-reads `current_lease_id` / `_epoch` / `_attempt_index` from FF's
/// `exec_core` on demand (see `FabricTaskService::resolve_active_lease`).
/// Keeping this struct carries the FCALL's typed response without a cairn
/// cache; the fields are read via tests / debug logs only.
#[derive(Clone, Debug)]
#[allow(dead_code)] // FF-authoritative; cairn re-reads from exec_core, never cached
pub struct ClaimOutcome {
    pub lease_id: LeaseId,
    pub lease_epoch: LeaseEpoch,
    pub attempt_index: AttemptIndex,
}

/// Execute the `ff_issue_claim_grant` + `ff_claim_execution` FCALL pair.
///
/// `lease_duration_ms` controls how long the lease remains valid before the
/// lease-expiry scanner reclaims it. `renew_before_ms` is derived as
/// `lease_duration_ms / 3` — matches `FabricTaskService::claim` and the
/// FF-recommended third-of-lease heartbeat cadence.
pub(crate) async fn issue_grant_and_claim(
    runtime: &Arc<FabricRuntime>,
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    eid: &ExecutionId,
    lane_id: &LaneId,
    lease_duration_ms: u64,
) -> Result<ClaimOutcome, FabricError> {
    // ── Step 1: issue claim grant ─────────────────────────────────────────
    let grant_ttl = runtime.config.grant_ttl_ms;
    let grant_keys: Vec<String> = vec![ctx.core(), ctx.claim_grant(), idx.lane_eligible(lane_id)];
    let grant_args: Vec<String> = vec![
        eid.to_string(),
        runtime.config.worker_id.to_string(),
        runtime.config.worker_instance_id.to_string(),
        lane_id.to_string(),
        String::new(),
        grant_ttl.to_string(),
        String::new(),
        String::new(),
    ];
    let grant_key_refs: Vec<&str> = grant_keys.iter().map(|s| s.as_str()).collect();
    let grant_arg_refs: Vec<&str> = grant_args.iter().map(|s| s.as_str()).collect();

    let raw_grant: ferriskey::Value = runtime
        .fcall(
            crate::fcall::names::FF_ISSUE_CLAIM_GRANT,
            &grant_key_refs,
            &grant_arg_refs,
        )
        .await
        .map_err(|e| FabricError::Internal(format!("ff_issue_claim_grant: {e}")))?;
    check_fcall_success(&raw_grant, crate::fcall::names::FF_ISSUE_CLAIM_GRANT)?;

    // ── Step 2: claim execution ────────────────────────────────────────────
    let total_str: Option<String> = runtime
        .client
        .hget(&ctx.core(), "total_attempt_count")
        .await
        .unwrap_or(None);
    let next_idx = total_str
        .as_deref()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let att_idx = AttemptIndex::new(next_idx);

    let lease_id = LeaseId::new();
    let attempt_id = ff_core::types::AttemptId::new();
    let renew_before_ms = lease_duration_ms / 3;

    let claim_keys: Vec<String> = vec![
        ctx.core(),
        ctx.claim_grant(),
        idx.lane_eligible(lane_id),
        idx.lease_expiry(),
        idx.worker_leases(&runtime.config.worker_instance_id),
        ctx.attempt_hash(att_idx),
        ctx.attempt_usage(att_idx),
        ctx.attempt_policy(att_idx),
        ctx.attempts(),
        ctx.lease_current(),
        ctx.lease_history(),
        idx.lane_active(lane_id),
        idx.attempt_timeout(),
        idx.execution_deadline(),
    ];
    let claim_args: Vec<String> = vec![
        eid.to_string(),
        runtime.config.worker_id.to_string(),
        runtime.config.worker_instance_id.to_string(),
        lane_id.to_string(),
        String::new(),
        lease_id.to_string(),
        lease_duration_ms.to_string(),
        renew_before_ms.to_string(),
        attempt_id.to_string(),
        "{}".to_owned(),
        String::new(),
        String::new(),
    ];
    let claim_key_refs: Vec<&str> = claim_keys.iter().map(|s| s.as_str()).collect();
    let claim_arg_refs: Vec<&str> = claim_args.iter().map(|s| s.as_str()).collect();

    let raw_claim: ferriskey::Value = runtime
        .fcall(
            crate::fcall::names::FF_CLAIM_EXECUTION,
            &claim_key_refs,
            &claim_arg_refs,
        )
        .await
        .map_err(|e| FabricError::Internal(format!("ff_claim_execution: {e}")))?;

    // Dispatch: if FF returns `use_claim_resumed_execution` the execution is
    // in `attempt_interrupted` (resumed from suspension) and must go through
    // `ff_claim_resumed_execution` — which resumes the SAME attempt instead
    // of allocating a new one. The grant is still live: FF's dispatch guard
    // (lua/execution.lua:382) runs BEFORE grant consumption (line 397), so
    // we do NOT re-issue it. NO state cached — the dispatch is driven
    // entirely by FF's typed error on each attempt.
    if fcall_error_code(&raw_claim).as_deref() == Some(USE_CLAIM_RESUMED_EXECUTION) {
        return claim_resumed_execution(runtime, ctx, idx, eid, lane_id, lease_duration_ms).await;
    }

    check_fcall_success(&raw_claim, crate::fcall::names::FF_CLAIM_EXECUTION)?;

    let lease_epoch = parse_claim_lease_epoch(&raw_claim)?;

    Ok(ClaimOutcome {
        lease_id,
        lease_epoch,
        attempt_index: att_idx,
    })
}

/// Execute `ff_claim_resumed_execution` after the fresh-claim path dispatched
/// us here. Resumes the EXISTING attempt (no new attempt_index) and rebinds
/// a fresh lease. KEYS / ARGV layout pinned to `lua/signal.lua:478-484`
/// (verified against ff-sdk `worker.rs:891-940` at FF @a098710).
async fn claim_resumed_execution(
    runtime: &Arc<FabricRuntime>,
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    eid: &ExecutionId,
    lane_id: &LaneId,
    lease_duration_ms: u64,
) -> Result<ClaimOutcome, FabricError> {
    // FF requires the existing attempt_hash to be KEYS[6]; we re-read the
    // attempt index from exec_core so the key points at the live attempt.
    // This is the only cairn-side read — all other fields are supplied by
    // FF's response.
    let att_idx_str: Option<String> = runtime
        .client
        .hget(&ctx.core(), "current_attempt_index")
        .await
        .unwrap_or(None);
    let att_idx = AttemptIndex::new(
        att_idx_str
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0),
    );

    let lease_id = LeaseId::new();

    // KEYS (11): see lua/signal.lua:478-481.
    let keys: Vec<String> = vec![
        ctx.core(),                                            // 1  exec_core
        ctx.claim_grant(),                                     // 2  claim_grant
        idx.lane_eligible(lane_id),                            // 3  eligible_zset
        idx.lease_expiry(),                                    // 4  lease_expiry_zset
        idx.worker_leases(&runtime.config.worker_instance_id), // 5  worker_leases
        ctx.attempt_hash(att_idx),                             // 6  existing_attempt_hash
        ctx.lease_current(),                                   // 7  lease_current
        ctx.lease_history(),                                   // 8  lease_history
        idx.lane_active(lane_id),                              // 9  active_index
        idx.attempt_timeout(),                                 // 10 attempt_timeout_zset
        idx.execution_deadline(),                              // 11 execution_deadline_zset
    ];

    // ARGV (8): see lua/signal.lua:482-484.
    let args: Vec<String> = vec![
        eid.to_string(),                               // 1 execution_id
        runtime.config.worker_id.to_string(),          // 2 worker_id
        runtime.config.worker_instance_id.to_string(), // 3 worker_instance_id
        lane_id.to_string(),                           // 4 lane
        String::new(),                                 // 5 capability_snapshot_hash
        lease_id.to_string(),                          // 6 lease_id
        lease_duration_ms.to_string(),                 // 7 lease_ttl_ms
        String::new(),                                 // 8 remaining_attempt_timeout_ms
    ];

    let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let raw: ferriskey::Value = runtime
        .fcall(
            crate::fcall::names::FF_CLAIM_RESUMED_EXECUTION,
            &key_refs,
            &arg_refs,
        )
        .await
        .map_err(|e| FabricError::Internal(format!("ff_claim_resumed_execution: {e}")))?;
    check_fcall_success(&raw, crate::fcall::names::FF_CLAIM_RESUMED_EXECUTION)?;

    let lease_epoch = parse_claim_lease_epoch(&raw)?;

    Ok(ClaimOutcome {
        lease_id,
        lease_epoch,
        attempt_index: att_idx,
    })
}

/// Parse the `lease_epoch` slot of `ff_claim_execution`'s reply:
/// `{1, "OK", <lease_id>, <lease_epoch>}`. Mirrors the private helper that
/// used to live inside `task_service.rs`; kept here so both services parse
/// the same wire shape.
fn parse_claim_lease_epoch(raw: &ferriskey::Value) -> Result<LeaseEpoch, FabricError> {
    if let ferriskey::Value::Array(arr) = raw {
        if let Some(Ok(ferriskey::Value::BulkString(b))) = arr.get(3) {
            if let Ok(n) = String::from_utf8_lossy(b).parse::<u64>() {
                return Ok(LeaseEpoch::new(n));
            }
        }
        if let Some(Ok(ferriskey::Value::Int(n))) = arr.get(3) {
            return Ok(LeaseEpoch::new(*n as u64));
        }
    }
    Err(FabricError::Internal(
        "ff_claim_execution: missing lease_epoch in response".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lease_epoch_from_bulk_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::BulkString(b"lease_abc".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"7".to_vec().into())),
        ]);
        let epoch = parse_claim_lease_epoch(&raw).unwrap();
        assert_eq!(epoch.0, 7);
    }

    #[test]
    fn parse_lease_epoch_from_int() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::BulkString(b"lease_abc".to_vec().into())),
            Ok(ferriskey::Value::Int(12)),
        ]);
        let epoch = parse_claim_lease_epoch(&raw).unwrap();
        assert_eq!(epoch.0, 12);
    }

    #[test]
    fn parse_lease_epoch_missing_slot_errors() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
        ]);
        assert!(parse_claim_lease_epoch(&raw).is_err());
    }

    #[test]
    fn parse_lease_epoch_rejects_non_array() {
        let raw = ferriskey::Value::SimpleString("OK".to_owned());
        assert!(parse_claim_lease_epoch(&raw).is_err());
    }

    #[test]
    fn parse_lease_epoch_rejects_non_numeric() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::BulkString(b"lease_abc".to_vec().into())),
            Ok(ferriskey::Value::BulkString(
                b"not-a-number".to_vec().into(),
            )),
        ]);
        assert!(parse_claim_lease_epoch(&raw).is_err());
    }

    /// Pins the Lua error code against the sentinel constant used by the
    /// dispatch branch. If FF renames `use_claim_resumed_execution` (or the
    /// ScriptError variant moves), `fcall_error_code` + this assertion fail
    /// together — making the silent "wrong code" regression impossible.
    #[test]
    fn use_claim_resumed_code_matches_ff_lua_sentinel() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(0)),
            Ok(ferriskey::Value::BulkString(
                b"use_claim_resumed_execution".to_vec().into(),
            )),
        ]);
        assert_eq!(
            fcall_error_code(&raw).as_deref(),
            Some(USE_CLAIM_RESUMED_EXECUTION),
            "FF's Lua error code must match the dispatch sentinel",
        );
    }

    /// Non-dispatch errors must NOT be mistaken for the resumed-claim
    /// trigger. Guards against a future refactor that checks
    /// `contains("resumed")` or similar loose matches.
    #[test]
    fn other_error_codes_do_not_trigger_dispatch() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(0)),
            Ok(ferriskey::Value::BulkString(
                b"lease_conflict".to_vec().into(),
            )),
        ]);
        assert_ne!(
            fcall_error_code(&raw).as_deref(),
            Some(USE_CLAIM_RESUMED_EXECUTION),
        );
    }

    /// Ok envelope — no error code, no dispatch.
    #[test]
    fn ok_envelope_returns_no_error_code() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::BulkString(b"lease_abc".to_vec().into())),
            Ok(ferriskey::Value::Int(1)),
        ]);
        assert!(fcall_error_code(&raw).is_none());
    }
}
