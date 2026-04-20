// Integration tests for cairn-fabric against a live Valkey instance.
//
// The harness stands up a disposable Valkey container via testcontainers-rs
// (one container per `cargo test` invocation, shared across every test in
// the binary through a `tokio::sync::OnceCell`). `FabricServices::start`
// runs against the container's host:port, and FF's Lua library is loaded
// on first boot via `ff_script::loader::ensure_library`.
//
// **Parallel-safe key isolation.** There is NO `FLUSHDB` between tests.
// Running `FLUSHDB` on a shared container concurrently is destructive —
// one test's FLUSHDB wipes another's in-flight state. Instead, every
// test gets a uuid-scoped `ProjectKey` via `TestHarness::setup()`, and
// every id inside the test is generated with `uuid::Uuid::new_v4()`
// (`unique_run_id`, `unique_task_id`, `unique_session_id`,
// `ExecutionId::deterministic_solo(...)`, `BudgetId::new()`, …). Keyspaces therefore do
// not collide across parallel tests — each test operates in its own
// project partition and FF's `{p:N}` hashtags route their FCALLs to
// disjoint slots.
//
// Run with:
//   cargo test -p cairn-fabric --test integration           # parallel (default)
//   cargo test -p cairn-fabric --test integration -- --test-threads=1
//
// No `--ignored` and no `CAIRN_TEST_VALKEY_URL` required. Set
// `CAIRN_TEST_VALKEY_URL` to override the container path and point at an
// external Valkey (useful for CI jobs that provision a sidecar and don't
// want a docker-in-docker dependency).

mod integration {
    pub mod test_budget;
    pub mod test_checkpoint;
    pub mod test_event_emission;
    pub mod test_lease_history_subscriber;
    pub mod test_orchestrator_stream;
    pub mod test_run_lifecycle;
    pub mod test_session;
    pub mod test_suspension;
    pub mod test_task_dependencies;
}

use std::sync::Arc;

use cairn_domain::tenancy::ProjectKey;
use cairn_domain::{RunId, SessionId, TaskId};
use cairn_fabric::{FabricConfig, FabricServices};
use cairn_store::InMemoryStore;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage,
};
use tokio::sync::OnceCell;

// ── Shared Valkey container ──────────────────────────────────────────────
//
// `OnceCell<ContainerHandle>` — the container is started lazily by the
// first test, shared across every test in the binary, and torn down when
// the binary exits (ContainerAsync's Drop kills and removes the container).
// Tests share one Valkey DB; isolation comes from uuid-scoped
// (tenant, workspace, project) triples per test (see
// `TestHarness::setup`), NOT from FLUSHDB — parallel tests must not
// trample each other's keyspaces.

struct ContainerHandle {
    // The handle itself is held only so Drop doesn't run early.
    _container: ContainerAsync<GenericImage>,
    host: String,
    port: u16,
}

static VALKEY_CONTAINER: OnceCell<ContainerHandle> = OnceCell::const_new();

async fn get_valkey_endpoint() -> (String, u16) {
    if let Ok(url) = std::env::var("CAIRN_TEST_VALKEY_URL") {
        let parsed = url::Url::parse(&url).expect("invalid CAIRN_TEST_VALKEY_URL");
        let host = parsed.host_str().unwrap_or("localhost").to_owned();
        let port = parsed.port().unwrap_or(6379);
        return (host, port);
    }

    let handle = VALKEY_CONTAINER
        .get_or_init(|| async {
            let container = GenericImage::new("valkey/valkey", "8-alpine")
                .with_exposed_port(6379.tcp())
                .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
                .start()
                .await
                .expect("failed to start valkey container");

            let host = container
                .get_host()
                .await
                .expect("container host unavailable")
                .to_string();
            let port = container
                .get_host_port_ipv4(6379.tcp())
                .await
                .expect("container port unavailable");

            ContainerHandle {
                _container: container,
                host,
                port,
            }
        })
        .await;

    (handle.host.clone(), handle.port)
}

pub struct TestHarness {
    pub fabric: FabricServices,
    pub project: ProjectKey,
    /// Shared handle to the InMemoryStore that backs `fabric`'s bridge.
    /// Tests inspect projection state here to assert that a mutation
    /// emitted its `BridgeEvent::*` → `RuntimeEvent::*` round-trip.
    pub event_log: Arc<InMemoryStore>,
}

impl TestHarness {
    pub async fn setup() -> Self {
        let (host, port) = get_valkey_endpoint().await;

        // Per-test project scope — uuid-suffixed so parallel tests route
        // their FCALLs to disjoint `{p:N}` hashtags and never share state.
        // Also per-test tenant/workspace so tenant-level indices (session
        // and run projections, budget ZSETs keyed on (tenant,workspace))
        // stay disjoint even when future tests graduate beyond project
        // scope.
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let tenant = format!("test_tenant_{}", suffix);
        let workspace = format!("test_workspace_{}", suffix);
        let project_id = format!("test_project_{}", suffix);
        let project = ProjectKey::new(tenant.as_str(), workspace.as_str(), project_id.as_str());

        // The engine's background scanners (delayed_promoter, lease_expiry,
        // etc.) iterate the lanes listed in FabricConfig.lane_id. Task and run
        // services write to `project_to_lane(project)` — if the config lane
        // differs, the scanners never see our delayed ZSETs and retries stay
        // stuck in RetryableFailed forever. Derive the config lane from the
        // same project so both sides agree.
        let lane_id = cairn_fabric::id_map::project_to_lane(&project);

        let config = FabricConfig {
            valkey_host: host,
            valkey_port: port,
            tls: false,
            cluster: false,
            lane_id,
            worker_id: ff_core::types::WorkerId::new("test-worker"),
            worker_instance_id: ff_core::types::WorkerInstanceId::new(
                uuid::Uuid::new_v4().to_string(),
            ),
            namespace: ff_core::types::Namespace::new("test"),
            lease_ttl_ms: 30_000,
            grant_ttl_ms: 5_000,
            max_concurrent_tasks: 4,
            signal_dedup_ttl_ms: 86_400_000,
            fcall_timeout_ms: 5_000,
            worker_capabilities: std::collections::BTreeSet::new(),
            // Deterministic dev secret. Distinct from ff-test's all-zeros
            // (ff-test/src/fixtures.rs:133) so an accidental cross-contamination
            // with an ff-test-driven Valkey would surface as an HMAC auth
            // failure instead of silent acceptance. The kid is cairn-scoped so
            // it does not collide with FF's default "k1" either.
            //
            // ⚠ HMAC-ROTATION FOOTGUN — DO NOT add a rotation test to this
            // suite. The `ff:sec:{p:N}:waitpoint_hmac` hash is PARTITION-
            // scoped in FF, NOT project-scoped. `FabricServices::start`
            // (boot.rs:seed_waitpoint_hmac_secret_if_configured) HSETs
            // every one of the 256 execution partitions on every harness
            // spin-up. All TestHarness instances write the SAME (kid,
            // secret) today, so the writes are idempotent and race-benign.
            //
            // A test that CHANGES the kid or secret mid-flight would
            // rotate the shared partition keys and silently break every
            // other in-flight test's signal delivery — `lua/signal.lua`'s
            // `validate_waitpoint_token` reads `hmac_secrets` fresh on
            // every `ff_deliver_signal` FCALL and would reject tokens
            // minted under the pre-rotation secret.
            //
            // If you need to exercise HMAC rotation end-to-end, run it
            // against a DEDICATED Valkey (point the harness at a separate
            // container via `CAIRN_TEST_VALKEY_URL`, or move the test into
            // its own test binary that does not share the `VALKEY_CONTAINER`
            // OnceCell). Do NOT add it to this suite.
            waitpoint_hmac_secret: Some(
                "00000000000000000000000000000000000000000000000000000000000000aa".into(),
            ),
            waitpoint_hmac_kid: Some("cairn-test-k1".into()),
        };

        let event_log = Arc::new(InMemoryStore::default());
        let event_log_for_bridge: Arc<dyn cairn_store::event_log::EventLog + Send + Sync> =
            event_log.clone();
        let fabric = FabricServices::start(config, event_log_for_bridge)
            .await
            .expect("FabricServices::start failed — is the container reachable?");

        Self {
            fabric,
            project,
            event_log,
        }
    }

    /// Borrow the runtime's `PartitionConfig` for id_map helpers.
    ///
    /// Tests thread this into `run_to_execution_id`, `task_to_execution_id`,
    /// etc., so every ExecutionId minted inside a test lands in the same
    /// partition scheme the runtime enforces (see id_map.rs's
    /// partition-count stability contract).
    pub fn partition_config(&self) -> &ff_core::partition::PartitionConfig {
        &self.fabric.runtime.partition_config
    }

    pub fn unique_run_id(&self) -> RunId {
        RunId::new(format!("integ_run_{}", uuid::Uuid::new_v4()))
    }

    pub fn unique_session_id(&self) -> SessionId {
        SessionId::new(format!("integ_sess_{}", uuid::Uuid::new_v4()))
    }

    pub fn unique_task_id(&self) -> TaskId {
        TaskId::new(format!("integ_task_{}", uuid::Uuid::new_v4()))
    }

    pub async fn teardown(self) {
        self.fabric.shutdown().await;
    }
}
