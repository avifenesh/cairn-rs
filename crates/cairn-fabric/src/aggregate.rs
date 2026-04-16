use std::sync::Arc;

use cairn_store::event_log::EventLog;
use tokio::task::JoinHandle;

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
    bridge_handle: JoinHandle<()>,
}

impl FabricServices {
    pub async fn start(
        config: FabricConfig,
        event_log: Arc<dyn EventLog + Send + Sync>,
    ) -> Result<Self, FabricError> {
        let runtime = Arc::new(FabricRuntime::start(config).await?);
        let (bridge, bridge_handle) = EventBridge::start(event_log);
        let bridge = Arc::new(bridge);

        let result = Self::build_services(runtime.clone(), bridge.clone(), bridge_handle);

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
    ) -> Result<Self, (FabricError, JoinHandle<()>)> {
        let registry = Arc::new(ActiveTaskRegistry::new());

        let runs = FabricRunService::new(runtime.clone(), bridge.clone());
        let tasks = FabricTaskService::new(runtime.clone(), registry.clone(), bridge.clone());
        let sessions = FabricSessionService::new(runtime.clone(), bridge.clone());
        let scheduler = FabricSchedulerService::new(&runtime);
        let worker = FabricWorkerService::new(runtime.clone(), registry.clone());
        let budgets = FabricBudgetService::new(runtime.clone());
        let quotas = FabricQuotaService::new(runtime.clone());
        let signals = SignalBridge::new(&runtime);

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
            bridge_handle,
        })
    }

    pub async fn shutdown(self) {
        let Self {
            runtime,
            bridge,
            registry: _,
            runs: _,
            tasks: _,
            sessions: _,
            scheduler: _,
            worker: _,
            budgets: _,
            quotas: _,
            signals: _,
            bridge_handle,
        } = self;

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
    fn active_task_registry_accessible() {
        let registry = ActiveTaskRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

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
