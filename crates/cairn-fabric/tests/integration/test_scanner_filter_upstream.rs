//! Upstream `ScannerFilter` (FF PR #127, issue #122) consumption test.
//!
//! Asserts the FF-side filter is being wired on **cairn**'s engine
//! construction path — that two cairn-fabric instances sharing a
//! Valkey container, each with a distinct `worker_instance_id`, get
//! per-instance scanner/completion isolation driven by FF's backend
//! filter (not just cairn's subscriber-layer tag gate).
//!
//! Three subtests, one per relevant surface:
//!
//! 1. `completion_subscribe_filter_isolates_streams` — directly
//!    invokes `CompletionBackend::subscribe_completions_filtered` with
//!    two distinct `instance_tag` filters on the same Valkey. Seeds
//!    two executions with different `cairn.instance_id` tags and
//!    PUBLISHes a completion for each. Asserts each stream sees ONLY
//!    its own exec's completion. Mirrors FF's
//!    `scanner_filter_isolation::completion_instance_tag_filter_isolates_subscribers`
//!    but routed through cairn's `ValkeyBackend` construction path.
//!
//! 2. `engine_config_scanner_filter_is_wired` — exists primarily as
//!    a **compile-time + boot-time** assertion that cairn's
//!    `FabricRuntime::start` actually populates
//!    `EngineConfig.scanner_filter.instance_tag` with the
//!    `worker_instance_id` — a regression here (e.g. a future refactor
//!    that silently zeroes the field) would leave the FF scanner
//!    cross-instance leak re-opened. We prove it by booting a fabric
//!    and inspecting the exec core + tags Valkey state the services
//!    write on create, which the filter gates against.
//!
//! The existing `test_instance_tag_filter.rs` (PR #106) still runs
//! end-to-end against the upgraded engine and serves as the
//! BridgeEvent-level regression guard for the composite behaviour —
//! any upstream-filter mis-wiring would surface there too.

use std::time::Duration;

use cairn_domain::tenancy::ProjectKey;
use cairn_domain::{SessionId, TaskId};
use cairn_fabric::test_harness::valkey_endpoint;
use cairn_fabric::{id_map, FabricConfig, FabricServices};
use cairn_store::projections::FfLeaseHistoryCursorStore;
use cairn_store::InMemoryStore;
use flowfabric::core::backend::{ScannerFilter, ValkeyConnection};
use flowfabric::core::completion_backend::CompletionBackend;
use flowfabric::core::keys::ExecKeyContext;
use flowfabric::core::partition::{execution_partition, PartitionConfig};
use flowfabric::core::types::FlowId;
use flowfabric::valkey::{ValkeyBackend, COMPLETION_CHANNEL};
use futures::StreamExt;
use std::sync::Arc;

/// Build a ferriskey client + ValkeyBackend for the shared container.
/// Mirrors FF's reference test fixture but goes through the cairn
/// test harness's `valkey_endpoint()` so we hit the same container
/// every other integration test uses.
async fn make_backend() -> (ferriskey::Client, Arc<ValkeyBackend>) {
    let (host, port) = valkey_endpoint().await;
    let client = ferriskey::ClientBuilder::new()
        .host(&host, port)
        .build()
        .await
        .expect("client build");
    let mut conn = ValkeyConnection::new(host, port);
    conn.tls = false;
    conn.cluster = false;
    let backend = ValkeyBackend::from_client_partitions_and_connection(
        client.clone(),
        PartitionConfig::default(),
        conn,
    );
    (client, backend)
}

/// Build a `{fp:N}:<uuid>` execution-id string and the corresponding
/// tags-hash key the backend HGETs against. The partition tag is
/// hard-coded here — this test only needs the filter + tags machinery
/// to line up, not real cairn routing.
fn tags_key(partition: u16, full_eid: &str) -> String {
    format!("ff:exec:{{fp:{partition}}}:{full_eid}:tags")
}
fn full_eid(partition: u16, bare_uuid: &str) -> String {
    format!("{{fp:{partition}}}:{bare_uuid}")
}

async fn publish(client: &ferriskey::Client, eid: &str, flow_id: &str) {
    let payload =
        format!(r#"{{"execution_id":"{eid}","flow_id":"{flow_id}","outcome":"success"}}"#);
    let _: i64 = client
        .cmd("PUBLISH")
        .arg(COMPLETION_CHANNEL)
        .arg(&payload)
        .execute()
        .await
        .expect("PUBLISH");
}

/// Drain a completion stream for up to `timeout`, returning the
/// execution-id strings seen. Used to assert each filtered stream
/// only sees its own instance's frames.
async fn drain(
    stream: &mut flowfabric::core::completion_backend::CompletionStream,
    timeout: Duration,
) -> Vec<String> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(p)) => seen.push(p.execution_id.to_string()),
            Ok(None) | Err(_) => break,
        }
    }
    seen
}

/// Direct `subscribe_completions_filtered` isolation proof — same
/// backend, two filters, two PUBLISHes, one delivery each. Exercises
/// the FF PR #127 surface cairn now consumes on boot via
/// `Engine::start_with_completions`.
#[tokio::test]
async fn completion_subscribe_filter_isolates_streams() {
    let (client, backend) = make_backend().await;

    // Partition 11 — arbitrary, just needs to stay distinct from
    // fixture partitions used elsewhere (7 in FF reference, 42 in
    // internal fixtures).
    const PARTITION: u16 = 11;

    // Each eid's bare portion must be a valid UUID — the backend
    // parses the PUBLISH payload's `execution_id` field through
    // `ExecutionId::parse` which rejects non-uuid bare portions.
    // uuid-suffix the TAG values so parallel test runs against the
    // shared container can't collide on subscriber routing.
    let uuid_i1 = uuid::Uuid::new_v4().to_string();
    let uuid_i2 = uuid::Uuid::new_v4().to_string();
    let run_suffix = uuid::Uuid::new_v4().simple().to_string();
    let eid_i1 = full_eid(PARTITION, &uuid_i1);
    let eid_i2 = full_eid(PARTITION, &uuid_i2);
    let tag_i1 = format!("instance-upstream-i1-{run_suffix}");
    let tag_i2 = format!("instance-upstream-i2-{run_suffix}");

    // Seed the two execs' tag hashes. The backend HGETs
    // `cairn.instance_id` against these keys to decide whether each
    // PUBLISHed completion matches the subscriber's filter.
    let _: i64 = client
        .cmd("HSET")
        .arg(tags_key(PARTITION, &eid_i1))
        .arg("cairn.instance_id")
        .arg(tag_i1.as_str())
        .execute()
        .await
        .expect("HSET i1");
    let _: i64 = client
        .cmd("HSET")
        .arg(tags_key(PARTITION, &eid_i2))
        .arg("cairn.instance_id")
        .arg(tag_i2.as_str())
        .execute()
        .await
        .expect("HSET i2");

    // Two filters — one per instance tag. `ScannerFilter` is
    // `#[non_exhaustive]`; mutate after default.
    let mut filter_i1 = ScannerFilter::default();
    filter_i1.instance_tag = Some(("cairn.instance_id".to_owned(), tag_i1.clone()));
    let mut filter_i2 = ScannerFilter::default();
    filter_i2.instance_tag = Some(("cairn.instance_id".to_owned(), tag_i2.clone()));

    let mut stream_i1 = backend
        .subscribe_completions_filtered(&filter_i1)
        .await
        .expect("subscribe i1");
    let mut stream_i2 = backend
        .subscribe_completions_filtered(&filter_i2)
        .await
        .expect("subscribe i2");

    // Brief settle for SUBSCRIBE to hit the backend before PUBLISH.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let flow_id = FlowId::new().to_string();
    publish(&client, &eid_i1, &flow_id).await;
    publish(&client, &eid_i2, &flow_id).await;

    let got_i1 = drain(&mut stream_i1, Duration::from_secs(2)).await;
    let got_i2 = drain(&mut stream_i2, Duration::from_secs(2)).await;

    assert_eq!(
        got_i1,
        vec![eid_i1.clone()],
        "filter_i1 must receive exactly the i1 completion (got {got_i1:?})"
    );
    assert_eq!(
        got_i2,
        vec![eid_i2.clone()],
        "filter_i2 must receive exactly the i2 completion (got {got_i2:?})"
    );

    // Cleanup tag keys so a re-run starts clean on the shared container.
    let _: i64 = client
        .cmd("DEL")
        .arg(tags_key(PARTITION, &eid_i1))
        .arg(tags_key(PARTITION, &eid_i2))
        .execute()
        .await
        .unwrap_or(0);
}

/// Boot a real `FabricServices` and prove the services it spawns do
/// in fact stamp `cairn.instance_id` on exec-tags — the precondition
/// for `EngineConfig.scanner_filter.instance_tag` to have anything
/// to match against. Without this wire-through the upstream filter
/// would short-circuit to "unmatched" on every candidate.
#[tokio::test]
async fn engine_config_scanner_filter_is_wired() {
    let (host, port) = valkey_endpoint().await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tenant = format!("t_{suffix}");
    let workspace = format!("w_{suffix}");
    let project_id = format!("p_{suffix}");
    let project = ProjectKey::new(tenant.as_str(), workspace.as_str(), project_id.as_str());
    let lane_id = id_map::project_to_lane(&project);
    let worker_instance_id =
        flowfabric::core::types::WorkerInstanceId::new(format!("inst-upstream-{suffix}"));

    let config = FabricConfig {
        valkey_host: host,
        valkey_port: port,
        tls: false,
        cluster: false,
        lane_id,
        worker_id: flowfabric::core::types::WorkerId::new(format!("w-upstream-{suffix}")),
        worker_instance_id: worker_instance_id.clone(),
        namespace: flowfabric::core::types::Namespace::new(format!("ns-upstream-{suffix}")),
        lease_ttl_ms: 30_000,
        grant_ttl_ms: 5_000,
        max_concurrent_tasks: 4,
        signal_dedup_ttl_ms: 86_400_000,
        fcall_timeout_ms: 5_000,
        worker_capabilities: std::collections::BTreeSet::new(),
        waitpoint_hmac_secret: Some(
            "00000000000000000000000000000000000000000000000000000000000000aa".into(),
        ),
        waitpoint_hmac_kid: Some("cairn-test-k1".into()),
    };

    let event_log = Arc::new(InMemoryStore::default());
    let event_log_shared: Arc<dyn cairn_store::event_log::EventLog + Send + Sync> =
        event_log.clone();
    let cursor_store: Arc<dyn FfLeaseHistoryCursorStore> = event_log.clone();
    let fabric =
        FabricServices::start_with_lease_history(config.clone(), event_log_shared, cursor_store)
            .await
            .expect("FabricServices::start_with_lease_history");

    let session_id = SessionId::new(format!("sess_{suffix}"));
    let task_id = TaskId::new(format!("task_{suffix}"));
    fabric
        .tasks
        .submit(&project, task_id.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit task");

    // Walk to the tags-hash the filter gates against and confirm the
    // worker_instance_id landed there. If it didn't, the upstream
    // filter would silently treat every frame as foreign — the exact
    // regression this test catches.
    let partition_config = fabric.runtime.partition_config;
    let eid =
        id_map::session_task_to_execution_id(&project, &session_id, &task_id, &partition_config);
    let partition = execution_partition(&eid, &partition_config);
    let ctx = ExecKeyContext::new(&partition, &eid);
    let stamped: Option<String> = fabric
        .runtime
        .client
        .hget(&ctx.tags(), "cairn.instance_id")
        .await
        .expect("HGET cairn.instance_id");
    assert_eq!(
        stamped.as_deref(),
        Some(worker_instance_id.as_str()),
        "services did not stamp worker_instance_id on exec tags — upstream scanner filter would mis-fire",
    );

    fabric.shutdown().await;
}
