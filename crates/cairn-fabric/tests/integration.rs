// Integration tests for cairn-fabric against a live Valkey instance.
//
// All tests are #[ignore] by default — they require a running Valkey.
//
// Run with:
//   CAIRN_TEST_VALKEY_URL=redis://localhost:6379 cargo test -p cairn-fabric --test integration -- --ignored
//
// Single test:
//   CAIRN_TEST_VALKEY_URL=redis://localhost:6379 cargo test -p cairn-fabric --test integration test_create_and_read_run -- --ignored

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

pub struct TestHarness {
    pub fabric: FabricServices,
    pub project: ProjectKey,
}

impl TestHarness {
    pub async fn setup() -> Self {
        let url = std::env::var("CAIRN_TEST_VALKEY_URL")
            .unwrap_or_else(|_| "redis://localhost:6379".into());

        let parsed = url::Url::parse(&url).expect("invalid CAIRN_TEST_VALKEY_URL");
        let host = parsed.host_str().unwrap_or("localhost").to_owned();
        let port = parsed.port().unwrap_or(6379);
        let tls = matches!(parsed.scheme(), "rediss" | "valkeys");

        let project = ProjectKey::new("test_tenant", "test_workspace", "test_project");

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
            tls,
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
            .expect("failed to connect to Valkey — is it running?");

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
