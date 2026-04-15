use std::sync::Arc;

use ferriskey::Client;
use ff_core::partition::PartitionConfig;
use ff_engine::{Engine, EngineConfig};

use crate::config::FabricConfig;
use crate::error::FabricError;

pub struct FabricRuntime {
    pub client: Client,
    pub engine: Engine,
    pub partition_config: PartitionConfig,
    pub config: Arc<FabricConfig>,
}

impl FabricRuntime {
    pub async fn start(config: FabricConfig) -> Result<Self, FabricError> {
        let url = config.valkey_url();
        tracing::info!(url = %url, "connecting to valkey");

        let client = if config.cluster {
            Client::connect_cluster(&[url.as_str()])
                .await
                .map_err(|e| FabricError::Valkey(e.to_string()))?
        } else {
            Client::connect(&url)
                .await
                .map_err(|e| FabricError::Valkey(e.to_string()))?
        };

        ff_script::loader::ensure_library(&client)
            .await
            .map_err(|e| FabricError::Valkey(format!("script load: {e}")))?;

        let partition_config = PartitionConfig::default();
        let engine_config = EngineConfig {
            partition_config,
            lanes: vec![config.lane_id.clone()],
            ..EngineConfig::default()
        };

        let engine = Engine::start(engine_config, client.clone());
        tracing::info!("fabric runtime started");

        Ok(Self {
            client,
            engine,
            partition_config,
            config: Arc::new(config),
        })
    }

    // TODO: add health_check() -> Result<(), FabricError> that PINGs Valkey.
    // Wire into cairn-app /health endpoint as fabric_ok.

    pub async fn shutdown(self) {
        tracing::info!("shutting down fabric runtime");
        self.engine.shutdown().await;
    }
}
