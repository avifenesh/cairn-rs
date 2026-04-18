// Integration tests for cairn-fabric against a live Valkey instance.
//
// The harness stands up a disposable Valkey container via testcontainers-rs
// (one container per `cargo test` invocation, shared across every test in
// the binary through a `tokio::sync::OnceCell`). `FabricServices::start`
// runs against the container's host:port, and FF's Lua library is loaded
// on first boot via `ff_script::loader::ensure_library`. Each test calls
// `TestHarness::setup()` which acquires the shared container handle and
// issues `FLUSHDB` so every test starts from a clean Valkey.
//
// Run with:
//   cargo test -p cairn-fabric --test integration
//
// No `--ignored` and no `CAIRN_TEST_VALKEY_URL` required. Set
// `CAIRN_TEST_VALKEY_URL` to override the container path and point at an
// external Valkey (useful for CI jobs that provision a sidecar and don't
// want a docker-in-docker dependency).

mod integration {
    pub mod test_budget;
    pub mod test_checkpoint;
    pub mod test_run_lifecycle;
    pub mod test_session;
    pub mod test_suspension;
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
// Tests do NOT get isolated databases; we rely on `FLUSHDB` between tests
// to keep them independent, which is sufficient because FF's Lua library
// is idempotent under FUNCTION LOAD REPLACE and the OnceCell guarantees
// `ff_script::loader::ensure_library` runs exactly once.

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
}

impl TestHarness {
    pub async fn setup() -> Self {
        let (host, port) = get_valkey_endpoint().await;

        // Give every test a fresh keyspace. The FF Lua library survives
        // FLUSHDB (Valkey FUNCTIONs are stored separately) so we do not
        // need to reload it per-test.
        flushdb(&host, port).await;

        let project_id = format!("test_project_{}", uuid::Uuid::new_v4().simple());
        let project = ProjectKey::new("test_tenant", "test_workspace", project_id.as_str());

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
            waitpoint_hmac_secret: Some(
                "00000000000000000000000000000000000000000000000000000000000000aa".into(),
            ),
            waitpoint_hmac_kid: Some("cairn-test-k1".into()),
        };

        let event_log = Arc::new(InMemoryStore::default());
        let fabric = FabricServices::start(config, event_log)
            .await
            .expect("FabricServices::start failed — is the container reachable?");

        Self { fabric, project }
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

async fn flushdb(host: &str, port: u16) {
    use ferriskey::Client;
    let url = format!("redis://{host}:{port}");
    let client = Client::connect(&url)
        .await
        .expect("failed to connect ferriskey Client for FLUSHDB");
    let _: String = client
        .cmd("FLUSHDB")
        .execute()
        .await
        .expect("FLUSHDB failed");
}
