//! `ff_rotate_waitpoint_hmac_secret` FCALL builder.
//!
//! FF 0.2 exposes the rotation FCALL on every execution partition. Each
//! call touches exactly one partition's `ff:sec:{p:N}:waitpoint_hmac`
//! hash; cairn fans out across all partitions (see
//! `FabricRotationService::rotate_waitpoint_hmac`). Atomicity is
//! per-partition — the FCALL IS the atomicity boundary, so no Rust-side
//! lock is required.
//!
//! Invariants enforced by the FCALL (see `flowfabric.lua`
//! `ff_rotate_waitpoint_hmac_secret`):
//!   * Empty / `:`-containing `new_kid` → `invalid_kid`.
//!   * Empty / odd-length / non-hex `new_secret_hex` → `invalid_secret_hex`.
//!   * Negative / non-integer `grace_ms` → `invalid_grace_ms`.
//!   * Same `new_kid` already installed with a DIFFERENT secret →
//!     `rotation_conflict` (operator must pick a fresh kid).
//!   * Same `new_kid` installed with the SAME secret → `ok("noop", kid)`.
//!
//! The function returns the previous kid (or empty) and a GC count for
//! expired kids removed during the same call.

use flowfabric::core::keys::IndexKeys;

/// Number of KEYS expected by `ff_rotate_waitpoint_hmac_secret`. Matches
/// the Lua contract (KEYS(1): `hmac_secrets`).
pub const ROTATE_WAITPOINT_HMAC_SECRET_KEYS: usize = 1;

/// Number of ARGS expected by `ff_rotate_waitpoint_hmac_secret`. Matches
/// the Lua contract (ARGV(3): `new_kid, new_secret_hex, grace_ms`).
pub const ROTATE_WAITPOINT_HMAC_SECRET_ARGS: usize = 3;

/// Build the KEYS + ARGV pair for `ff_rotate_waitpoint_hmac_secret` on a
/// single partition. Callers are responsible for iterating across every
/// partition and invoking the FCALL once per partition.
pub fn build_rotate_waitpoint_hmac_secret(
    idx: &IndexKeys,
    new_kid: &str,
    new_secret_hex: &str,
    grace_ms: u64,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![idx.waitpoint_hmac_secrets()];
    let args = vec![
        new_kid.to_owned(),
        new_secret_hex.to_owned(),
        grace_ms.to_string(),
    ];
    (keys, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowfabric::core::partition::{Partition, PartitionFamily};

    fn test_idx() -> IndexKeys {
        let partition = Partition {
            family: PartitionFamily::Execution,
            index: 7,
        };
        IndexKeys::new(&partition)
    }

    #[test]
    fn builds_expected_keys_and_args() {
        let idx = test_idx();
        let (keys, args) = build_rotate_waitpoint_hmac_secret(&idx, "kid_v2", "deadbeef", 60_000);
        assert_eq!(keys.len(), ROTATE_WAITPOINT_HMAC_SECRET_KEYS);
        assert_eq!(args.len(), ROTATE_WAITPOINT_HMAC_SECRET_ARGS);
        assert_eq!(keys[0], idx.waitpoint_hmac_secrets());
        assert_eq!(args[0], "kid_v2");
        assert_eq!(args[1], "deadbeef");
        assert_eq!(args[2], "60000");
    }

    #[test]
    fn grace_ms_zero_serializes() {
        let idx = test_idx();
        let (_, args) = build_rotate_waitpoint_hmac_secret(&idx, "kid", "aa", 0);
        assert_eq!(args[2], "0");
    }
}
