//! Shared test-only helpers for cairn-fabric unit tests.
//!
//! Gated on `#[cfg(test)]` so the symbols are only compiled into the
//! crate's unit-test binary. Integration tests in `tests/integration/`
//! cannot see `#[cfg(test)]` items (they compile against the library
//! crate without `cfg(test)`), so those files keep local helpers —
//! typically one that accepts `&TestHarness` so it threads the
//! cluster-wide `partition_config()` through.

use flowfabric::core::partition::PartitionConfig;
use flowfabric::core::types::{ExecutionId, LaneId};
use uuid::Uuid;

/// Mint a deterministic-but-distinct `ExecutionId` for tests.
///
/// Uses `ExecutionId::deterministic_solo` with a UUID v5 derived from
/// `seed`. Distinct seeds produce distinct ExecutionIds, so FF's dedup
/// slot does NOT fire between them — which is the invariant every
/// spend-without-dedup test in the crate relies on. Tests that need a
/// pinned ExecutionId for dedup coverage just pass the same seed twice.
pub(crate) fn test_eid(seed: &str) -> ExecutionId {
    let uuid = Uuid::new_v5(&Uuid::NAMESPACE_DNS, seed.as_bytes());
    ExecutionId::deterministic_solo(&LaneId::new("test"), &PartitionConfig::default(), uuid)
}
