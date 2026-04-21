//! Waitpoint HMAC rotation service.
//!
//! Surfaces `ff_rotate_waitpoint_hmac_secret` over every execution
//! partition. This is the cairn-side replacement for the operator's
//! previous restart-with-new-env-var dance: the FCALL installs the new
//! kid as `current_kid` on each partition and keeps the prior kid
//! accepted for `grace_ms` so in-flight waitpoints signed with the old
//! secret continue to verify.
//!
//! Partition fan-out is sequential. Each FCALL is O(small HSET) and the
//! default 256 partitions finish in <100ms on a local Valkey; there's
//! no scale case today that justifies parallelising at the cost of
//! harder error attribution. Operators that truly need parallelism
//! drive it caller-side by issuing the admin request across multiple
//! cairn-app instances.
//!
//! Idempotency: the FCALL converges on the same `(new_kid,
//! new_secret_hex)` via its `noop` outcome, so a failed fan-out is
//! safe to retry. Partial success (some partitions rotated, some
//! failed) is surfaced as `RotateOutcome::partial(..)` — operators see
//! the exact list of failed partition indices and can re-run.

use std::sync::Arc;

use ff_core::keys::IndexKeys;
use ff_core::partition::{Partition, PartitionFamily};

use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::fcall;
use crate::helpers::fcall_error_code;

/// Outcome of a rotation fan-out. Mirrors ff-server's
/// `RotateWaitpointSecretResult` shape (`rotated`, `failed`, `new_kid`)
/// plus the Lua-level details (noop, per-partition failures) cairn
/// wants to surface to operators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotateOutcome {
    /// Count of partitions that accepted a fresh rotation (`ok("rotated",
    /// ...)`). Zero on pure-replay calls where every partition was
    /// already on `(new_kid, new_secret_hex)`.
    pub rotated: u16,
    /// Count of partitions that replied `ok("noop", kid)` — exact
    /// replay of the same kid + secret. Idempotent retry path.
    pub noop: u16,
    /// Partition indices where the rotation failed. Operator should
    /// investigate (typically transport, or `rotation_conflict` if the
    /// kid was reused with a different secret). Re-running with the
    /// same `(new_kid, new_secret_hex)` once the underlying fault clears
    /// converges.
    pub failed: Vec<RotationFailure>,
    /// Echoed back for operator confirmation. Always equals the input.
    pub new_kid: String,
}

/// Per-partition failure detail for the rotation fan-out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotationFailure {
    pub partition_index: u16,
    /// FF typed error code when the Lua envelope returned one
    /// (`rotation_conflict`, `invalid_kid`, `invalid_secret_hex`,
    /// `invalid_grace_ms`). `None` when the call failed before FCALL
    /// reply (transport / timeout).
    pub code: Option<String>,
    /// Human-readable detail. For transport failures this is the
    /// underlying error string; for Lua rejections it repeats the code
    /// plus any details the FCALL envelope carried.
    pub detail: String,
}

/// Cairn-side rotation service.
pub struct FabricRotationService {
    runtime: Arc<FabricRuntime>,
}

impl FabricRotationService {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }

    /// Rotate the waitpoint HMAC signing kid across every execution
    /// partition. See module-level rustdoc for idempotency / partial-
    /// success semantics.
    ///
    /// Caller-facing validation (empty / `:`-containing `new_kid`, odd-
    /// length `new_secret_hex`, etc.) happens server-side via the
    /// FCALL; the service surfaces the typed error unchanged via
    /// `RotationFailure::code`. Callers can wrap this in an HTTP 400
    /// when the outcome shows every partition failed with the same
    /// input-validation code.
    pub async fn rotate_waitpoint_hmac(
        &self,
        new_kid: &str,
        new_secret_hex: &str,
        grace_ms: u64,
    ) -> RotateOutcome {
        let num_partitions = self.runtime.partition_config.num_flow_partitions;
        let mut rotated = 0u16;
        let mut noop = 0u16;
        let mut failed: Vec<RotationFailure> = Vec::new();

        for index in 0..num_partitions {
            let partition = Partition {
                family: PartitionFamily::Execution,
                index,
            };
            let idx = IndexKeys::new(&partition);
            let (keys, args) = fcall::rotation::build_rotate_waitpoint_hmac_secret(
                &idx,
                new_kid,
                new_secret_hex,
                grace_ms,
            );
            let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

            match self
                .runtime
                .fcall(
                    fcall::names::FF_ROTATE_WAITPOINT_HMAC_SECRET,
                    &key_refs,
                    &arg_refs,
                )
                .await
            {
                Ok(raw) => {
                    if let Some(code) = fcall_error_code(&raw) {
                        failed.push(RotationFailure {
                            partition_index: index,
                            code: Some(code.clone()),
                            detail: format!("partition {index} rejected: {code}"),
                        });
                    } else {
                        match classify_ok_variant(&raw) {
                            Ok(OkVariant::Rotated) => rotated += 1,
                            Ok(OkVariant::Noop) => noop += 1,
                            Err(e) => failed.push(RotationFailure {
                                partition_index: index,
                                code: None,
                                detail: format!(
                                    "partition {index} returned unparseable envelope: {e}"
                                ),
                            }),
                        }
                    }
                }
                Err(err) => failed.push(RotationFailure {
                    partition_index: index,
                    code: None,
                    detail: format!("partition {index} transport error: {err}"),
                }),
            }
        }

        RotateOutcome {
            rotated,
            noop,
            failed,
            new_kid: new_kid.to_owned(),
        }
    }
}

/// Success envelopes for `ff_rotate_waitpoint_hmac_secret`.
enum OkVariant {
    Rotated,
    Noop,
}

/// Inspect a success envelope (`fcall_error_code` already returned
/// `None`) and pick out whether this partition rotated or was a
/// replay.
///
/// FF's ok(...) envelope packs as `[Int(1), BulkString("OK"),
/// ...caller_args]`. For rotation the caller_args are either
/// `"rotated", previous_kid_or_empty, new_kid, gc_count`
/// or `"noop", kid`. Variant discriminator lives at index 2 (after
/// the leading status int and the fixed "OK" marker).
fn classify_ok_variant(raw: &ferriskey::Value) -> Result<OkVariant, FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => return Err(FabricError::Internal("rotate envelope not an array".into())),
    };
    let variant = match arr.get(2) {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        _ => {
            return Err(FabricError::Internal(
                "rotate envelope missing variant discriminator".into(),
            ))
        }
    };
    match variant.as_str() {
        "rotated" => Ok(OkVariant::Rotated),
        "noop" => Ok(OkVariant::Noop),
        other => Err(FabricError::Internal(format!(
            "unexpected rotation variant: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Envelope shape: [Int(1), "OK", variant, ...caller_args].
    #[test]
    fn classify_ok_variant_rotated() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(b"OK".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"rotated".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"kid_v1".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"kid_v2".to_vec().into())),
            Ok(ferriskey::Value::Int(0)),
        ]);
        assert!(matches!(classify_ok_variant(&raw), Ok(OkVariant::Rotated)));
    }

    #[test]
    fn classify_ok_variant_noop() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(b"OK".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"noop".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"kid_v2".to_vec().into())),
        ]);
        assert!(matches!(classify_ok_variant(&raw), Ok(OkVariant::Noop)));
    }

    #[test]
    fn classify_ok_variant_unknown_variant_errors() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(b"OK".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"stranger".to_vec().into())),
        ]);
        assert!(classify_ok_variant(&raw).is_err());
    }
}
