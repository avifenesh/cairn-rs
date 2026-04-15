use crate::error::FabricError;

/// Recovery action summary (mirrors cairn-runtime's RecoverySummary).
#[derive(Clone, Debug, Default)]
pub struct FabricRecoverySummary {
    pub scanned: usize,
    pub actions: Vec<String>,
}

/// No-op recovery stub for the FlowFabric backend.
///
/// FF Engine's built-in scanners handle all recovery:
/// - **Expired leases**: `lease_expiry` scanner runs every 1.5s, revokes stale
///   leases and transitions executions back to eligible/failed.
/// - **Interrupted runs**: `attempt_timeout` scanner (2s) handles per-attempt
///   deadlines; `execution_deadline` scanner (5s) handles overall execution
///   deadlines. Both fail or retry the execution atomically.
/// - **Stale dependencies**: `dependency_reconciler` scanner (15s) checks
///   parent-child flow edges and unblocks parents whose children are terminal.
///
/// This stub provides the same method signatures as cairn-runtime's
/// `RecoveryService` but returns empty summaries — the FF engine does the work.
pub struct FabricRecoveryStub;

impl FabricRecoveryStub {
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

    #[tokio::test]
    async fn recover_expired_leases_returns_empty() {
        let stub = FabricRecoveryStub;
        let summary = stub.recover_expired_leases(0, 100).await.unwrap();
        assert!(summary.actions.is_empty());
        assert_eq!(summary.scanned, 0);
    }

    #[tokio::test]
    async fn recover_interrupted_runs_returns_empty() {
        let stub = FabricRecoveryStub;
        let summary = stub.recover_interrupted_runs(100).await.unwrap();
        assert!(summary.actions.is_empty());
        assert_eq!(summary.scanned, 0);
    }

    #[tokio::test]
    async fn resolve_stale_dependencies_returns_empty() {
        let stub = FabricRecoveryStub;
        let summary = stub.resolve_stale_dependencies(100).await.unwrap();
        assert!(summary.actions.is_empty());
        assert_eq!(summary.scanned, 0);
    }

    #[tokio::test]
    async fn all_methods_are_idempotent() {
        let stub = FabricRecoveryStub;
        for _ in 0..3 {
            assert!(stub.recover_expired_leases(999, 10).await.is_ok());
            assert!(stub.recover_interrupted_runs(10).await.is_ok());
            assert!(stub.resolve_stale_dependencies(10).await.is_ok());
        }
    }
}
