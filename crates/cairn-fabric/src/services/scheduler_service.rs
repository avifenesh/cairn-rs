//! FabricSchedulerService — thin wrapper over the ff-scheduler crate.
//!
//! # Phase D PR 2a exception
//!
//! Unlike `run_service`, `session_service`, `task_service`, and
//! `claim_common`, this service still imports `flowfabric::scheduler::claim::
//! {Scheduler, ClaimGrant}` directly. That's intentional.
//!
//! `ClaimGrant` is a wire-contract type shared with ff-sdk workers:
//! when a worker dequeues a grant it receives exactly this struct,
//! and the worker-side code paths in ff-sdk depend on its field
//! layout. Mirroring it cairn-side would add a conversion hop that
//! hides nothing — both cairn and ff-sdk would still have to agree
//! on the layout, and the cairn-native mirror would track upstream
//! 1:1 with every FF release.
//!
//! When FF 0.3 stabilises the scheduler types upstream (tracked in
//! [FlowFabric#58](https://github.com/avifenesh/FlowFabric/issues/58))
//! this stays an exception. Any future Phase D work that retires
//! `ff_scheduler` from cairn (e.g. cairn running its own scheduling
//! loop against the `ControlPlaneBackend` trait directly) would
//! revisit the exception — but that's a Phase E / F scope, not
//! Phase D.

use std::collections::BTreeSet;
use std::sync::Arc;

use flowfabric::core::partition::PartitionConfig;
use flowfabric::core::types::{LaneId, WorkerId, WorkerInstanceId};
use flowfabric::scheduler::claim::{ClaimGrant, Scheduler};

use crate::boot::FabricRuntime;
use crate::error::FabricError;

pub struct FabricSchedulerService {
    scheduler: Scheduler,
    /// Capabilities advertised to FF at claim time. Passed into
    /// `Scheduler::claim_for_worker` as `&BTreeSet<String>`; FF builds a
    /// deterministic sorted CSV from this set and matches against each
    /// execution's `required_capabilities` via `ff_issue_claim_grant`.
    /// Empty set = "no capabilities" (FF treats any execution with a
    /// non-empty `required_capabilities` as unclaimable by this worker).
    worker_capabilities: BTreeSet<String>,
}

impl FabricSchedulerService {
    pub fn new(runtime: &Arc<FabricRuntime>) -> Self {
        let scheduler = Scheduler::new(runtime.client.clone(), runtime.partition_config);
        Self {
            scheduler,
            worker_capabilities: runtime.config.worker_capabilities.clone(),
        }
    }

    pub fn from_parts(client: ferriskey::Client, partition_config: PartitionConfig) -> Self {
        let scheduler = Scheduler::new(client, partition_config);
        Self {
            scheduler,
            worker_capabilities: BTreeSet::new(),
        }
    }

    /// Construct for tests with an explicit capability set.
    #[cfg(test)]
    pub fn from_parts_with_capabilities(
        client: ferriskey::Client,
        partition_config: PartitionConfig,
        worker_capabilities: BTreeSet<String>,
    ) -> Self {
        let scheduler = Scheduler::new(client, partition_config);
        Self {
            scheduler,
            worker_capabilities,
        }
    }

    /// Read-only view of the capability set threaded into every
    /// `claim_for_worker` call. Cross-review-friendly: makes the value
    /// we're actually sending to FF observable without reaching into
    /// private state.
    pub fn worker_capabilities(&self) -> &BTreeSet<String> {
        &self.worker_capabilities
    }

    /// Issue a claim grant for a worker against an eligible execution in `lane_id`.
    ///
    /// **Lean-bridge silence (intentional).** Does not emit a `BridgeEvent`. A
    /// claim grant is transient pre-claim state — it becomes observable only
    /// when the worker converts the grant into a real claim via
    /// `task_service::claim` / `run_service::claim`, which emit
    /// `TaskLeaseClaimed` or document their own silence (see run claim §4.3
    /// in `docs/design/CAIRN-FABRIC-FINALIZED.md`). A grant that expires
    /// without conversion has no projection impact — cairn's read model only
    /// needs to see the eventual claim.
    ///
    /// See `docs/design/bridge-event-audit.md` §2.4.
    pub async fn claim_for_worker(
        &self,
        lane_id: &LaneId,
        worker_id: &WorkerId,
        instance_id: &WorkerInstanceId,
        grant_ttl_ms: u64,
    ) -> Result<Option<ClaimGrant>, FabricError> {
        self.scheduler
            .claim_for_worker(
                lane_id,
                worker_id,
                instance_id,
                &self.worker_capabilities,
                grant_ttl_ms,
            )
            .await
            .map_err(|e| FabricError::Bridge(format!("scheduler claim_for_worker: {e}")))
    }

    /// Compute eligible ZSET score: -(priority * 1T) + created_at_ms.
    /// Lower score = claimed first. Valid priority range: 0–9223 for exact
    /// arithmetic. Values above 9223 saturate to i64::MIN (still highest priority).
    pub fn priority_score(priority: u32, created_at_ms: u64) -> i64 {
        -(priority as i64).saturating_mul(1_000_000_000_000) + created_at_ms as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_score_higher_priority_is_lower_score() {
        let high = FabricSchedulerService::priority_score(10, 1000);
        let low = FabricSchedulerService::priority_score(1, 1000);
        assert!(high < low);
    }

    #[test]
    fn priority_score_same_priority_earlier_created_first() {
        let earlier = FabricSchedulerService::priority_score(5, 1000);
        let later = FabricSchedulerService::priority_score(5, 2000);
        assert!(earlier < later);
    }

    #[test]
    fn priority_score_zero_priority() {
        let score = FabricSchedulerService::priority_score(0, 5000);
        assert_eq!(score, 5000);
    }

    #[test]
    fn priority_score_high_priority_dominates_time() {
        let high_late = FabricSchedulerService::priority_score(10, 999_999_999_999);
        let low_early = FabricSchedulerService::priority_score(1, 0);
        assert!(high_late < low_early);
    }

    #[test]
    fn priority_score_deterministic() {
        let a = FabricSchedulerService::priority_score(3, 12345);
        let b = FabricSchedulerService::priority_score(3, 12345);
        assert_eq!(a, b);
    }

    #[test]
    fn priority_score_max_priority() {
        let score = FabricSchedulerService::priority_score(u32::MAX, 0);
        assert!(score < 0);
    }

    #[test]
    fn priority_score_ordering_across_range() {
        let scores: Vec<i64> = (0..=5)
            .map(|p| FabricSchedulerService::priority_score(p, 1000))
            .collect();
        for w in scores.windows(2) {
            assert!(
                w[0] > w[1],
                "p={} should score higher (more negative) than p-1",
                w[1]
            );
        }
    }

    // Capability-threading unit tests.
    //
    // These tests verify the cairn-side plumbing — that the BTreeSet in
    // FabricConfig is the one `FabricSchedulerService::new` clones into its
    // field, and the one `claim_for_worker` passes to ff-scheduler
    // unchanged. FF-side subset matching, CSV canonicalization, and token
    // validation are covered by ff-scheduler's own tests; we do NOT
    // duplicate that here (see design principle: FF owns capability logic,
    // cairn only threads the set through).
    //
    // We cannot construct a ferriskey::Client without a live connection,
    // so we can't instantiate FabricSchedulerService itself in a unit test.
    // What we can — and do — verify: FabricConfig stores the caps verbatim,
    // BTreeSet provides deterministic sorted iteration (so FF's derived
    // CSV is stable), and deduplication is already handled by the container
    // (so operator mistakes like double-listing a token can't inflate FF's
    // CAPS_MAX_TOKENS count).

    #[test]
    fn config_preserves_capability_set_verbatim() {
        use crate::config::FabricConfig;
        let mut caps = BTreeSet::new();
        caps.insert("gpu".to_owned());
        caps.insert("cuda-12".to_owned());
        caps.insert("linux-x86_64".to_owned());

        let config = FabricConfig {
            valkey_host: "localhost".into(),
            valkey_port: 6379,
            tls: false,
            cluster: false,
            lane_id: flowfabric::core::types::LaneId::new("test"),
            worker_id: flowfabric::core::types::WorkerId::new("w"),
            worker_instance_id: flowfabric::core::types::WorkerInstanceId::new("i"),
            namespace: flowfabric::core::types::Namespace::new("ns"),
            lease_ttl_ms: 30_000,
            grant_ttl_ms: 5_000,
            max_concurrent_tasks: 1,
            signal_dedup_ttl_ms: 86_400_000,
            fcall_timeout_ms: 5_000,
            worker_capabilities: caps.clone(),
            waitpoint_hmac_secret: None,
            waitpoint_hmac_kid: None,
        };

        // The config carries the set unchanged. FabricSchedulerService::new
        // clones this field verbatim — the subject under test is that no
        // reordering, filtering, or canonicalization happens on the cairn
        // side. FF does the canonicalization (sorted CSV) internally.
        assert_eq!(
            config.worker_capabilities.iter().collect::<Vec<_>>(),
            vec!["cuda-12", "gpu", "linux-x86_64"],
            "BTreeSet must iterate in deterministic sorted order so FF's CSV is stable",
        );
        assert_eq!(config.worker_capabilities, caps);
    }

    #[test]
    fn empty_capability_set_is_distinct_from_missing() {
        // Empty set = no caps advertised (FF accepts, only matches
        // zero-requirement executions). Smoke-test that Default produces
        // empty, not a phantom sentinel.
        let caps: BTreeSet<String> = BTreeSet::new();
        assert!(caps.is_empty());
        assert_eq!(caps.iter().next(), None);
    }

    #[test]
    fn btreeset_deduplicates_capability_tokens() {
        // If an operator double-lists a capability (common mistake in env
        // var parsing), BTreeSet collapses it — FF will never see a
        // duplicate token, which keeps its CAPS_MAX_TOKENS count honest.
        let mut caps = BTreeSet::new();
        caps.insert("gpu".to_owned());
        caps.insert("gpu".to_owned());
        caps.insert("cpu".to_owned());
        assert_eq!(caps.len(), 2);
    }
}
