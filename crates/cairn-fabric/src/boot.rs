use std::sync::Arc;

use ferriskey::Client;
use flowfabric::core::backend::{ScannerFilter, ValkeyConnection};
use flowfabric::core::capability::Capabilities;
use flowfabric::core::completion_backend::CompletionBackend;
use flowfabric::core::contracts::SeedWaitpointHmacSecretArgs;
use flowfabric::core::engine_backend::EngineBackend;
use flowfabric::core::partition::PartitionConfig;
use flowfabric::engine::{Engine, EngineConfig};
use flowfabric::valkey::ValkeyBackend;

use crate::config::FabricConfig;
use crate::error::FabricError;

pub struct FabricRuntime {
    pub client: Client,
    pub engine: Engine,
    pub partition_config: PartitionConfig,
    pub config: Arc<FabricConfig>,
    /// Shared FF observability handle. Constructed once at
    /// `FabricRuntime::start` and cloned into the `Engine` — so the
    /// counters/histograms the engine records into at runtime are the
    /// same ones rendered here. `Metrics` is internally `Arc`-based; the
    /// explicit `Arc` here matches the engine's expected ownership and
    /// lets cairn-app append FF's Prometheus text to its `/metrics`
    /// response without threading a second handle through startup.
    pub ff_metrics: Arc<ff_observability::Metrics>,
    /// FF 0.10 (FF#277) flat capabilities struct, computed once at
    /// boot and cached. Consumers inspect this to grey-render features
    /// a backend does not support without round-tripping into Valkey.
    /// CG-a wires the startup call + getter; CJ-2 consumes it from the
    /// UI greyrender. CG-c migrated the BTreeMap to the flat `Supports`
    /// struct per the v0.9→v0.10 consumer migration guide.
    pub capabilities: Capabilities,
    /// Typed handle to the backend. Held for the post-boot surface
    /// (`restore_frames` stream reads, etc.) that now takes
    /// `&dyn EngineBackend`. Kept alongside the raw `client` for CG-a
    /// — CG-b drops the raw client once `subscribe_lease_history` is
    /// trait-based (FF#282).
    pub backend: Arc<dyn EngineBackend>,
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

        // FF 0.9 (FF#281): backend.prepare() replaces the hand-rolled
        // `ff_script::loader::ensure_library` retry loop. The Valkey
        // impl runs FUNCTION LOAD REPLACE and is idempotent — safe to
        // call on every boot. Fails loud with EngineError::Backend on
        // transport errors; cairn propagates as FabricError::Engine.
        let prepare_outcome = backend
            .prepare()
            .await
            .map_err(|e| FabricError::Engine(Box::new(e)))?;
        tracing::info!(outcome = ?prepare_outcome, "backend prepared (FF#281)");

        // FF 0.9 (FF#280): seed the waitpoint HMAC secret via the
        // trait method. Replaces cairn's per-partition HSET loop.
        // Idempotent upstream — operators can call it every boot and
        // observe `SeedOutcome::AlreadySeeded` after the first.
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
        let seed_outcome = backend
            .seed_waitpoint_hmac_secret(SeedWaitpointHmacSecretArgs::new(kid, secret))
            .await
            .map_err(|e| FabricError::Engine(Box::new(e)))?;
        tracing::info!(kid = %kid, outcome = ?seed_outcome, "waitpoint HMAC secret seeded (FF#280)");

        // FF 0.10 (FF#277 flat reshape): snapshot the backend's
        // capabilities once at boot so consumers (cairn-app `/v1/status`,
        // UI grey-rendering) can reason about backend-parity gaps without
        // per-request RTT. CJ-2 consumes this via AppState.
        let capabilities = backend.capabilities();
        tracing::info!(
            backend = %capabilities.identity.family,
            version = ?capabilities.identity.version,
            "backend capabilities captured (FF#277)"
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
        // Build the shared FF metrics registry once. The engine
        // records into this same `Arc<Metrics>` the /metrics handler
        // later renders — cloning the Arc is how FF's own crates share
        // the registry across threads (see ff-observability 0.3.2
        // `real.rs`: every instrument handle is itself `Arc`-backed).
        let ff_metrics = std::sync::Arc::new(ff_observability::Metrics::new());
        let engine = Engine::start_with_completions(
            engine_config,
            client.clone(),
            std::sync::Arc::clone(&ff_metrics),
            completion_stream,
        );
        tracing::info!("fabric runtime started");

        // `ValkeyBackend::from_client_partitions_and_connection` returns
        // an `Arc<ValkeyBackend>`; coerce to the trait object so the
        // post-boot surface (`restore_frames`, future trait-consumers
        // from CG-b) holds a stable dyn-compatible handle.
        let backend: Arc<dyn EngineBackend> = backend;

        Ok(Self {
            client,
            engine,
            partition_config,
            config: Arc::new(config),
            ff_metrics,
            capabilities,
            backend,
        })
    }

    /// Expose the cached capability matrix. Stable snapshot captured at
    /// boot; callers MUST treat it as read-only (FF#277).
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Expose the backend handle for post-boot consumers that need the
    /// typed trait surface (stream reads, future lease-history
    /// subscriber in CG-b).
    pub fn backend(&self) -> &Arc<dyn EngineBackend> {
        &self.backend
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

// FF 0.9 (FF#280) adopted: `seed_waitpoint_hmac_secret_if_configured`
// (~45 LOC of per-partition HSET loops) was deleted in favour of
// `EngineBackend::seed_waitpoint_hmac_secret`. Call site lives in
// `FabricRuntime::start` above.
