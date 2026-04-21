use std::sync::Arc;

use cairn_store::event_log::EventLog;
use cairn_store::projections::FfLeaseHistoryCursorStore;
use tokio::task::JoinHandle;

use crate::boot::FabricRuntime;
use crate::config::FabricConfig;
use crate::error::FabricError;
use crate::event_bridge::EventBridge;
use crate::lease_history_subscriber::LeaseHistorySubscriber;
use crate::services::{
    FabricBudgetService, FabricQuotaService, FabricRotationService, FabricRunService,
    FabricSchedulerService, FabricSessionService, FabricTaskService, FabricWorkerService,
};
use crate::signal_bridge::SignalBridge;

pub struct FabricServices {
    pub runtime: Arc<FabricRuntime>,
    pub bridge: Arc<EventBridge>,
    pub runs: FabricRunService,
    pub tasks: FabricTaskService,
    pub sessions: FabricSessionService,
    pub scheduler: FabricSchedulerService,
    pub worker: FabricWorkerService,
    pub budgets: FabricBudgetService,
    pub quotas: FabricQuotaService,
    pub rotation: FabricRotationService,
    pub signals: SignalBridge,
    bridge_handle: JoinHandle<()>,
    lease_history: Option<LeaseHistorySubscriber>,
}

impl FabricServices {
    pub async fn start(
        config: FabricConfig,
        event_log: Arc<dyn EventLog + Send + Sync>,
    ) -> Result<Self, FabricError> {
        Self::start_inner(config, event_log, None).await
    }

    /// Variant that wires the lease-history subscriber against a
    /// cursor-store implementation. When `None`, the subscriber is
    /// skipped — useful for tests that don't want the background
    /// tail running against a scratch Valkey.
    pub async fn start_with_lease_history(
        config: FabricConfig,
        event_log: Arc<dyn EventLog + Send + Sync>,
        cursor_store: Arc<dyn FfLeaseHistoryCursorStore>,
    ) -> Result<Self, FabricError> {
        Self::start_inner(config, event_log, Some(cursor_store)).await
    }

    async fn start_inner(
        config: FabricConfig,
        event_log: Arc<dyn EventLog + Send + Sync>,
        cursor_store: Option<Arc<dyn FfLeaseHistoryCursorStore>>,
    ) -> Result<Self, FabricError> {
        let runtime = Arc::new(FabricRuntime::start(config).await?);
        let (bridge, bridge_handle) = EventBridge::start(event_log);
        let bridge = Arc::new(bridge);

        let lease_history = cursor_store.map(|store| {
            LeaseHistorySubscriber::start(
                runtime.client.clone(),
                runtime.partition_config.num_flow_partitions,
                bridge.clone(),
                store,
            )
        });

        let result = Self::build_services(
            runtime.clone(),
            bridge.clone(),
            bridge_handle,
            lease_history,
        );

        match result {
            Ok(services) => {
                tracing::info!("fabric services aggregate ready");
                Ok(services)
            }
            Err((e, handle)) => {
                bridge.stop();
                handle.abort();
                drop(bridge);
                Err(e)
            }
        }
    }

    fn build_services(
        runtime: Arc<FabricRuntime>,
        bridge: Arc<EventBridge>,
        bridge_handle: JoinHandle<()>,
        lease_history: Option<LeaseHistorySubscriber>,
    ) -> Result<Self, (FabricError, JoinHandle<()>)> {
        let runs = FabricRunService::new(runtime.clone(), bridge.clone());
        let tasks = FabricTaskService::new(runtime.clone(), bridge.clone());
        let sessions = FabricSessionService::new(runtime.clone(), bridge.clone());
        let scheduler = FabricSchedulerService::new(&runtime);
        let worker = FabricWorkerService::new(runtime.clone());
        let budgets = FabricBudgetService::new(runtime.clone());
        let quotas = FabricQuotaService::new(runtime.clone());
        let rotation = FabricRotationService::new(runtime.clone());
        let signals = SignalBridge::new(&runtime);

        Ok(Self {
            runtime,
            bridge,
            runs,
            tasks,
            sessions,
            scheduler,
            worker,
            budgets,
            quotas,
            rotation,
            signals,
            bridge_handle,
            lease_history,
        })
    }

    pub async fn shutdown(self) {
        let Self {
            runtime,
            bridge,
            runs: _,
            tasks: _,
            sessions: _,
            scheduler: _,
            worker: _,
            budgets: _,
            quotas: _,
            rotation: _,
            signals: _,
            bridge_handle,
            lease_history,
        } = self;

        if let Some(lh) = lease_history {
            lh.shutdown().await;
        }

        bridge.stop();
        drop(bridge);
        if let Err(e) = bridge_handle.await {
            tracing::warn!(error = %e, "event bridge consumer task panicked");
        }

        match Arc::try_unwrap(runtime) {
            Ok(rt) => rt.shutdown().await,
            Err(arc) => {
                tracing::warn!(
                    refs = Arc::strong_count(&arc),
                    "fabric runtime has outstanding references, skipping engine shutdown"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_config_from_env_defaults() {
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CAIRN_FABRIC_HOST");
        std::env::remove_var("CAIRN_FABRIC_PORT");
        std::env::remove_var("CAIRN_FABRIC_LEASE_TTL_MS");
        std::env::remove_var("CAIRN_FABRIC_MAX_TASKS");
        std::env::remove_var("CAIRN_FABRIC_GRANT_TTL_MS");
        let config = FabricConfig::from_env().unwrap();
        assert_eq!(config.valkey_host, "localhost");
        assert_eq!(config.valkey_port, 6379);
    }
}
