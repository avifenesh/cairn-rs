use std::sync::Arc;

use crate::boot::FabricRuntime;
use crate::error::FabricError;

#[derive(Clone, Debug, Default)]
pub struct FabricRecoverySummary {
    pub scanned: usize,
    pub actions: Vec<String>,
}

pub struct FabricRecoveryStub {
    runtime: Arc<FabricRuntime>,
}

impl FabricRecoveryStub {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }

    pub async fn log_scanner_health(&self) {
        match self.runtime.health_check().await {
            Ok(()) => {
                tracing::info!("fabric scanner health: valkey reachable, 14 FF scanners running");
            }
            Err(e) => {
                tracing::error!(error = %e, "fabric scanner health: valkey unreachable — scanners stalled");
            }
        }
    }

    pub async fn recover_expired_leases(
        &self,
        _now: u64,
        _limit: usize,
    ) -> Result<FabricRecoverySummary, FabricError> {
        Ok(FabricRecoverySummary::default())
    }

    pub async fn recover_interrupted_runs(
        &self,
        _limit: usize,
    ) -> Result<FabricRecoverySummary, FabricError> {
        Ok(FabricRecoverySummary::default())
    }

    pub async fn resolve_stale_dependencies(
        &self,
        _limit: usize,
    ) -> Result<FabricRecoverySummary, FabricError> {
        Ok(FabricRecoverySummary::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_summary_default_is_empty() {
        let summary = FabricRecoverySummary::default();
        assert!(summary.actions.is_empty());
        assert_eq!(summary.scanned, 0);
    }
}
