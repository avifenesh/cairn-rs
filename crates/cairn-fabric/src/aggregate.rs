use std::sync::Arc;

use cairn_store::event_log::EventLog;

use crate::active_tasks::ActiveTaskRegistry;
use crate::boot::FabricRuntime;
use crate::config::FabricConfig;
use crate::error::FabricError;
use crate::event_bridge::EventBridge;
use crate::services::{
    FabricBudgetService, FabricQuotaService, FabricRunService, FabricSchedulerService,
    FabricSessionService, FabricTaskService, FabricWorkerService,
};
use crate::signal_bridge::SignalBridge;

pub struct FabricServices {
    pub runtime: Arc<FabricRuntime>,
    pub bridge: Arc<EventBridge>,
    pub registry: Arc<ActiveTaskRegistry>,
    pub runs: FabricRunService,
    pub tasks: FabricTaskService,
    pub sessions: FabricSessionService,
    pub scheduler: FabricSchedulerService,
    pub worker: FabricWorkerService,
    pub budgets: FabricBudgetService,
    pub quotas: FabricQuotaService,
    pub signals: SignalBridge,
}

impl FabricServices {
    pub async fn start(
        config: FabricConfig,
        event_log: Arc<dyn EventLog + Send + Sync>,
    ) -> Result<Self, FabricError> {
        let runtime = Arc::new(FabricRuntime::start(config).await?);
        let bridge = Arc::new(EventBridge::new(event_log));
        let registry = Arc::new(ActiveTaskRegistry::new());

        let runs = FabricRunService::new(runtime.clone(), bridge.clone());
        let tasks = FabricTaskService::new(runtime.clone(), registry.clone(), bridge.clone());
        let sessions = FabricSessionService::new(runtime.clone(), bridge.clone());
        let scheduler = FabricSchedulerService::new(&runtime);
        let worker = FabricWorkerService::new(runtime.clone(), registry.clone());
        let budgets = FabricBudgetService::new(runtime.clone());
        let quotas = FabricQuotaService::new(runtime.clone());
        let signals = SignalBridge::new(&runtime);

        tracing::info!("fabric services aggregate ready");

        Ok(Self {
            runtime,
            bridge,
            registry,
            runs,
            tasks,
            sessions,
            scheduler,
            worker,
            budgets,
            quotas,
            signals,
        })
    }

    /// Shut down the Fabric runtime and all background scanners.
    ///
    /// Drops all service fields first to release their Arc<FabricRuntime> clones,
    /// then unwraps the sole remaining Arc to call Engine::shutdown().
    pub async fn shutdown(self) {
        let Self {
            runtime,
            bridge: _,
            registry: _,
            runs: _,
            tasks: _,
            sessions: _,
            scheduler: _,
            worker: _,
            budgets: _,
            quotas: _,
            signals: _,
        } = self;

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
    fn active_task_registry_accessible() {
        let registry = ActiveTaskRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn fabric_config_from_env_defaults() {
        std::env::remove_var("CAIRN_FABRIC_HOST");
        std::env::remove_var("CAIRN_FABRIC_PORT");
        std::env::remove_var("CAIRN_FABRIC_LEASE_TTL_MS");
        std::env::remove_var("CAIRN_FABRIC_MAX_TASKS");
        let config = FabricConfig::from_env().unwrap();
        assert_eq!(config.valkey_host, "localhost");
        assert_eq!(config.valkey_port, 6379);
    }
}
