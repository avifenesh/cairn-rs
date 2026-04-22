use std::sync::Arc;

use ferriskey::Client;
use ff_backend_valkey::ValkeyBackend;
use ff_core::backend::{ScannerFilter, ValkeyConnection};
use ff_core::completion_backend::CompletionBackend;
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
        tracing::info!(
            host = %config.valkey_host,
            port = config.valkey_port,
            tls = config.tls,
            cluster = config.cluster,
            "connecting to valkey"
        );

        let mut last_err = String::new();
        let mut client = None;
        for attempt in 0..CONNECT_MAX_ATTEMPTS {
            // Rebuild each attempt: `ClientBuilder::build` consumes self.
            let result = config.valkey_client_builder().build().await;
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

        // Verify the connected Valkey has the Functions API (>= 7.0),
        // with a boot-time WARN when the detected major is below 8.0.
        // 60s retry budget tolerates rolling upgrades. See
        // `version_check` module docs for the full rationale.
        crate::version_check::verify_valkey_version(&client).await?;

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

        // PERF#5/#6: operators need to see the partition counts on boot
        // so a mis-set cluster (e.g. half the nodes at 64, half at 256)
        // is obvious before any ExecutionId is minted. ExecutionId
        // stability depends on keeping `num_flow_partitions` fixed across
        // the cluster lifetime (RFC-011 bumped the default from 64 → 256).
        tracing::info!(
            num_flow_partitions = partition_config.num_flow_partitions,
            num_budget_partitions = partition_config.num_budget_partitions,
            num_quota_partitions = partition_config.num_quota_partitions,
            "fabric: initialised with partition config (default changed \
             64→256 in RFC-011; ExecutionId stability depends on keeping \
             this count fixed across the cluster lifetime)"
        );

        // Seed the waitpoint HMAC secret BEFORE Engine::start so any
        // suspend-path FCALL the engine scanners trigger finds an
        // initialized secrets hash. Idempotent: re-running boot with the
        // same secret is safe; HSETs are per-partition hash-tagged so no
        // CROSSSLOT risk under cluster.
        seed_waitpoint_hmac_secret_if_configured(&client, &config, &partition_config).await?;

        // Build the per-consumer `ScannerFilter` (FF PR #127 / issue #122).
        //
        // Scope choice — **instance_tag only, namespace is `None`**.
        //
        // FF's `ScannerFilter` has two axes: `namespace` (matched against
        // `exec_core.namespace`) and `instance_tag` (matched against the
        // `cairn.instance_id` entry on the execution's tags hash).
        //
        // Cairn's `exec_core.namespace` is written per-*tenant*
        // (`id_map::tenant_to_namespace(project.tenant_id)`, see
        // `services/run_service.rs::namespace` and the session-service
        // sibling). A single cairn-fabric process serves many tenants,
        // so wiring `ScannerFilter.namespace` to any one of them would
        // collapse scanner scope to just that tenant's executions.
        // Tenant scoping is enforced at the cairn-store projection
        // layer, not via FF scanners — so we leave this axis unset.
        //
        // `instance_tag`, by contrast, is written per-*cairn-app
        // instance* (task_service + run_service `HSET cairn.instance_id
        // <worker_instance_id>`). It's the exact axis the cross-instance
        // isolation invariant needs — two cairn-apps sharing a Valkey
        // must each see only their own executions' scanner cycles and
        // completion frames.
        //
        // Supersedes cairn's PR #106 client-side filter in
        // `LeaseHistorySubscriber::fetch_entity_context` — the upstream
        // backend filter now drops foreign frames before they hit the
        // cairn subscriber, and the matching predicate on the
        // subscribe_completions_filtered stream keeps this engine's
        // DAG dispatch loop blind to foreign completions.
        // `ScannerFilter` is `#[non_exhaustive]` on the FF side
        // (future dimensions like `lane_id` / `worker_instance` can
        // land additively), so we can't use a bare struct literal or
        // struct-update syntax. Mutate after `default()` instead.
        let mut scanner_filter = ScannerFilter::default();
        scanner_filter.instance_tag = Some((
            "cairn.instance_id".to_owned(),
            config.worker_instance_id.as_str().to_owned(),
        ));

        // Construct a ValkeyBackend around the already-dialed client so
        // the completion subscriber can reuse a single connection
        // topology and open its dedicated RESP3 subscriber from the
        // retained `ValkeyConnection`. This replaces FF 0.3.0's
        // `CompletionListenerConfig` — PR #127 removed the implicit
        // listener field on EngineConfig in favour of an explicit
        // stream handed to `Engine::start_with_completions`.
        let mut valkey_conn = ValkeyConnection::new(config.valkey_host.clone(), config.valkey_port);
        valkey_conn.tls = config.tls;
        valkey_conn.cluster = config.cluster;
        let backend = ValkeyBackend::from_client_partitions_and_connection(
            client.clone(),
            partition_config,
            valkey_conn,
        );

        // Open the filtered completion stream. The backend applies the
        // filter at push time (one HGET on the exec's tags hash per
        // frame when `instance_tag` is set), so foreign completions
        // never reach the dispatch loop. This closes the
        // cross-instance leak that cairn's PR #106 addressed
        // client-side and the FF#122 data-plane audit flagged in the
        // DAG dispatch path.
        let completion_stream = backend
            .subscribe_completions_filtered(&scanner_filter)
            .await
            .map_err(|e| FabricError::Valkey(format!("subscribe_completions_filtered: {e}")))?;

        let engine_config = EngineConfig {
            partition_config,
            lanes: vec![config.lane_id.clone()],
            scanner_filter: scanner_filter.clone(),
            ..EngineConfig::default()
        };

        // `Engine::start_with_completions` is the PR #127 replacement
        // for the old `completion_listener: Some(_)` field. Supplying
        // the stream here wires push-based DAG dispatch; scanners also
        // honour `scanner_filter` so every execution-shaped scan
        // (lease_expiry, attempt_timeout, etc.) skips foreign
        // candidates before the scanner FCALL hot path.
        let engine = Engine::start_with_completions(
            engine_config,
            client.clone(),
            std::sync::Arc::new(ff_observability::Metrics::new()),
            completion_stream,
        );
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
/// Fails loud if `config.waitpoint_hmac_secret` is `None` — production
/// can't ship without it because every `ff_suspend_execution` would
/// reject with `hmac_secret_not_initialized`.
///
/// Hash layout per partition:
///   HSET waitpoint_hmac_secrets:{p:N} current_kid <kid>
///   HSET waitpoint_hmac_secrets:{p:N} secret:<kid> <secret_hex>
///
/// Idempotent on re-run with the same secret. **Not** a live rotation —
/// use FF's `rotate_waitpoint_hmac_secret` for that.
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
            return Err(FabricError::Config(
                "CAIRN_FABRIC_WAITPOINT_HMAC_SECRET is required — boot refuses \
                 to ship a runtime that would reject every ff_suspend_execution \
                 with hmac_secret_not_initialized. Set the secret (64 hex chars) \
                 plus CAIRN_FABRIC_WAITPOINT_HMAC_KID."
                    .to_owned(),
            ));
        }
    };

    let num_partitions = partition_config.num_flow_partitions;
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
