use std::sync::Arc;

use ferriskey::Client;
use ff_core::keys::IndexKeys;
use ff_core::partition::{Partition, PartitionConfig, PartitionFamily};
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

        let mut lib_loaded = false;
        for attempt in 0..CONNECT_MAX_ATTEMPTS {
            match ff_script::loader::ensure_library(&client).await {
                Ok(()) => {
                    lib_loaded = true;
                    break;
                }
                Err(e) => {
                    let msg = e.to_string();
                    if attempt + 1 < CONNECT_MAX_ATTEMPTS {
                        let backoff = 500 * (1 << attempt);
                        tracing::warn!(
                            attempt = attempt + 1,
                            error = %msg,
                            backoff_ms = backoff,
                            "ensure_library failed, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                    } else {
                        return Err(FabricError::Valkey(format!("script load: {msg}")));
                    }
                }
            }
        }
        if !lib_loaded {
            return Err(FabricError::Valkey("script load: retries exhausted".into()));
        }

        let partition_config = PartitionConfig::default();

        // Seed the waitpoint HMAC secret BEFORE Engine::start so any
        // suspend-path FCALL the engine scanners trigger (e.g. auto-resume
        // on a pre-existing suspended execution) finds an initialized
        // secrets hash. FF's mint_waitpoint_token at lua/helpers.lua:204-218
        // requires current_kid + secret:<kid> on every execution partition.
        //
        // Mirrors the install path from FF's own ff-test fixtures
        // (ff-test/src/fixtures.rs:141-157). Cluster-safe: each HSET targets
        // a distinct {p:N} hash tag, so every write lands on its own slot —
        // no CROSSSLOT risk. Serial issuance rather than parallel because
        // 256 HSETs against localhost are sub-second and bursting them in
        // parallel offers no operator benefit (boot is rare).
        seed_waitpoint_hmac_secret_if_configured(&client, &config, &partition_config).await?;

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
        if cfg!(debug_assertions) {
            let k: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
            let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            crate::fcall::verify_builder_counts(function, &k, &a)?;
        }
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

/// Seed the waitpoint HMAC secret across every execution partition.
///
/// No-op when `config.waitpoint_hmac_secret` is `None` — logs a WARN so the
/// operator knows suspend/signal paths will fail with
/// `hmac_secret_not_initialized` until they either configure a secret or
/// seed via FF admin tooling.
///
/// The hash layout (per partition) is:
///   HSET waitpoint_hmac_secrets:{p:N} current_kid <kid>
///   HSET waitpoint_hmac_secrets:{p:N} secret:<kid> <secret_hex>
///
/// Matches `ff_test::fixtures::install_waitpoint_hmac_secret` in
/// FF @a098710 (crates/ff-test/src/fixtures.rs:141-157) which is the
/// authoritative pattern. Idempotent: HSET overwrites; re-running boot with
/// a new secret rotates in-place (NOT a safe rotation — use FF's
/// rotate_waitpoint_hmac_secret helper for live rotations).
async fn seed_waitpoint_hmac_secret_if_configured(
    client: &Client,
    config: &FabricConfig,
    partition_config: &PartitionConfig,
) -> Result<(), FabricError> {
    let (secret, kid) = match (
        config.waitpoint_hmac_secret.as_deref(),
        config.resolved_waitpoint_hmac_kid(),
    ) {
        (Some(s), Some(k)) => (s, k),
        _ => {
            tracing::warn!(
                "no HMAC secret configured; ff_suspend_execution will fail \
                 with hmac_secret_not_initialized until operator seeds \
                 waitpoint_hmac_secrets on every execution partition"
            );
            return Ok(());
        }
    };

    let num_partitions = partition_config.num_execution_partitions;
    let secret_field = format!("secret:{kid}");

    for index in 0..num_partitions {
        let partition = Partition {
            family: PartitionFamily::Execution,
            index,
        };
        let key = IndexKeys::new(&partition).waitpoint_hmac_secrets();

        let _: i64 = client.hset(&key, "current_kid", kid).await.map_err(|e| {
            FabricError::Valkey(format!("HSET current_kid on partition {index}: {e}"))
        })?;
        let _: i64 = client
            .hset(&key, &secret_field, secret)
            .await
            .map_err(|e| {
                FabricError::Valkey(format!("HSET {secret_field} on partition {index}: {e}"))
            })?;
    }

    tracing::info!(
        kid = %kid,
        partitions = num_partitions,
        "seeded waitpoint HMAC secret"
    );
    Ok(())
}
