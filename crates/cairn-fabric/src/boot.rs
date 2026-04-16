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

const CONNECT_MAX_ATTEMPTS: u32 = 3;
const CONNECT_BACKOFF_MS: [u64; 3] = [1_000, 2_000, 4_000];

impl FabricRuntime {
    // Steady-state reconnect after transient disconnects is handled by
    // ferriskey's internal connection pool — it re-establishes transparently
    // on the next command. This retry loop only covers initial startup.
    pub async fn start(config: FabricConfig) -> Result<Self, FabricError> {
        let url = config.valkey_url();
        tracing::info!(url = %url, "connecting to valkey");

        let mut last_err = String::new();
        let mut client = None;
        for attempt in 0..CONNECT_MAX_ATTEMPTS {
            let result = if config.cluster {
                Client::connect_cluster(&[url.as_str()]).await
            } else {
                Client::connect(&url).await
            };
            match result {
                Ok(c) => {
                    client = Some(c);
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    if attempt + 1 < CONNECT_MAX_ATTEMPTS {
                        let backoff = CONNECT_BACKOFF_MS[attempt as usize];
                        tracing::warn!(
                            attempt = attempt + 1,
                            error = %last_err,
                            backoff_ms = backoff,
                            "valkey connect failed, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                    }
                }
            }
        }
        let client = client.ok_or(FabricError::Valkey(last_err))?;

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

    pub async fn fcall(
        &self,
        function: &str,
        keys: &[&str],
        args: &[&str],
    ) -> Result<ferriskey::Value, FabricError> {
        let timeout = std::time::Duration::from_millis(self.config.fcall_timeout_ms);
        match tokio::time::timeout(timeout, self.client.fcall(function, keys, args)).await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => Err(FabricError::Valkey(format!("{function}: {e}"))),
            Err(_) => Err(FabricError::Valkey(format!(
                "{function}: timeout after {}ms",
                self.config.fcall_timeout_ms
            ))),
        }
    }

    pub async fn health_check(&self) -> Result<(), FabricError> {
        let _: Option<String> = self
            .client
            .hget("ff:health_check", "noop")
            .await
            .map_err(|e| FabricError::Valkey(format!("health check: {e}")))?;
        Ok(())
    }

    pub async fn shutdown(self) {
        tracing::info!("shutting down fabric runtime");
        self.engine.shutdown().await;
    }
}
