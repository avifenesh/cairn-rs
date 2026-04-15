use std::sync::Arc;

use ff_core::partition::PartitionConfig;
use ff_core::types::{LaneId, WorkerId, WorkerInstanceId};
use ff_scheduler::claim::{ClaimGrant, Scheduler};

use crate::boot::FabricRuntime;
use crate::error::FabricError;

pub struct FabricSchedulerService {
    scheduler: Scheduler,
}

impl FabricSchedulerService {
    pub fn new(runtime: &Arc<FabricRuntime>) -> Self {
        let scheduler = Scheduler::new(runtime.client.clone(), runtime.partition_config);
        Self { scheduler }
    }

    pub fn from_parts(client: ferriskey::Client, partition_config: PartitionConfig) -> Self {
        let scheduler = Scheduler::new(client, partition_config);
        Self { scheduler }
    }

    pub async fn claim_for_worker(
        &self,
        lane_id: &LaneId,
        worker_id: &WorkerId,
        instance_id: &WorkerInstanceId,
        grant_ttl_ms: u64,
    ) -> Result<Option<ClaimGrant>, FabricError> {
        self.scheduler
            .claim_for_worker(lane_id, worker_id, instance_id, grant_ttl_ms)
            .await
            .map_err(|e| FabricError::Bridge(format!("scheduler claim_for_worker: {e}")))
    }

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
}
