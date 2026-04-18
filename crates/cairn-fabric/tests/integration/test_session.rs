// Session lifecycle integration tests.
//
// Covers FCALLs previously uncovered by the integration suite:
//   - ff_create_flow   (via sessions.create)
//   - ff_cancel_flow   (via sessions.archive)
//
// Runs against a testcontainers-provisioned Valkey (see tests/integration.rs).
// `TestHarness::setup()` issues FLUSHDB between tests so index zsets do not
// collide across runs — the earlier `unique_session_id()` crutch stays only
// as defense in depth.

use std::collections::HashMap;

use cairn_domain::lifecycle::SessionState;
use cairn_domain::SessionId;
use ff_core::keys::FlowKeyContext;
use ff_core::partition::flow_partition;

use crate::TestHarness;

/// Read the FF `flow_core` hash for a session's flow_id directly from
/// Valkey. Used to prove that `sessions.archive` actually mutated
/// Valkey — not just that the Rust return value changed.
///
/// The field set we assert on comes from FF `lua/flow.lua`:
///   - `ff_create_flow` (lua/flow.lua:83-90): sets `public_flow_state="open"`
///   - `ff_cancel_flow` (lua/flow.lua:181-185): sets `public_flow_state="cancelled"`,
///     `cancelled_at`, `cancel_reason`, `last_mutation_at`
async fn read_flow_core(h: &TestHarness, session_id: &SessionId) -> HashMap<String, String> {
    let fid = cairn_fabric::id_map::session_to_flow_id(&h.project, session_id);
    let partition = flow_partition(&fid, &h.fabric.runtime.partition_config);
    let ctx = FlowKeyContext::new(&partition, &fid);
    let fields: HashMap<String, String> = h
        .fabric
        .runtime
        .client
        .hgetall(&ctx.core())
        .await
        .expect("HGETALL flow core failed");
    fields
}

/// #5 from the coverage audit: create + archive happy path.
///
/// Exercises `ff_create_flow` (via `sessions.create`) and `ff_cancel_flow`
/// (via `sessions.archive`). Session is the root of the entity hierarchy
/// (session → run → task); a regression here is platform-down.
///
/// Includes GAP #6 post-archive Valkey assertion: we query
/// `flow_core.public_flow_state` directly to prove FF wrote `cancelled`,
/// not just that our Rust service said so.
#[tokio::test]
async fn test_session_create_and_cancel_flow() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();

    let created = h
        .fabric
        .sessions
        .create(&h.project, session_id.clone())
        .await
        .expect("create failed");

    assert_eq!(created.session_id, session_id);
    assert_eq!(created.project.tenant_id, h.project.tenant_id);
    assert_eq!(created.project.workspace_id, h.project.workspace_id);
    assert_eq!(created.project.project_id, h.project.project_id);
    assert_eq!(
        created.state,
        SessionState::Open,
        "freshly created session must be Open, got {:?}",
        created.state,
    );

    // Post-create Valkey assertion: ff_create_flow at lua/flow.lua:83-90
    // writes public_flow_state="open" into flow_core.
    let pre = read_flow_core(&h, &session_id).await;
    assert_eq!(
        pre.get("public_flow_state").map(|s| s.as_str()),
        Some("open"),
        "flow_core.public_flow_state must be 'open' after create, got {:?}",
        pre.get("public_flow_state"),
    );

    let fetched = h
        .fabric
        .sessions
        .get(&h.project, &session_id)
        .await
        .expect("get failed")
        .expect("session must be readable after create");
    assert_eq!(fetched.session_id, session_id);
    assert_eq!(fetched.state, SessionState::Open);

    let archived = h
        .fabric
        .sessions
        .archive(&h.project, &session_id)
        .await
        .expect("archive failed");
    assert_eq!(archived.session_id, session_id);
    assert_eq!(
        archived.state,
        SessionState::Archived,
        "archived session must report Archived state, got {:?}",
        archived.state,
    );

    // GAP #6: prove FF wrote the cancel, not just cairn's archive flag.
    // ff_cancel_flow at lua/flow.lua:181-185 sets public_flow_state="cancelled",
    // cancelled_at=<ts>, cancel_reason=<reason>, last_mutation_at=<ts>.
    // cairn-fabric ALSO sets `cairn.archived="true"` on the same hash
    // (session_service.rs:217). Assert BOTH — one catches FF regressions,
    // the other catches cairn-side regressions.
    let post = read_flow_core(&h, &session_id).await;
    assert_eq!(
        post.get("public_flow_state").map(|s| s.as_str()),
        Some("cancelled"),
        "flow_core.public_flow_state must be 'cancelled' after archive, got {:?}",
        post.get("public_flow_state"),
    );
    assert!(
        post.get("cancelled_at")
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "flow_core.cancelled_at must be set after archive, got {:?}",
        post.get("cancelled_at"),
    );
    assert_eq!(
        post.get("cairn.archived").map(|s| s.as_str()),
        Some("true"),
        "flow_core.cairn.archived must be 'true' after archive, got {:?}",
        post.get("cairn.archived"),
    );

    let after = h
        .fabric
        .sessions
        .get(&h.project, &session_id)
        .await
        .expect("get after archive failed")
        .expect("archived session must still be readable");
    assert_eq!(
        after.state,
        SessionState::Archived,
        "post-archive get must still report Archived, got {:?}",
        after.state,
    );

    h.teardown().await;
}

/// Second archive on an already-archived session must be idempotent. The
/// `ff_cancel_flow` Lua returns `flow_already_terminal`
/// (see /tmp/FlowFabric/lua/flow.lua:172-175), and
/// `session_service::archive` specifically swallows that status — see the
/// `if !msg.contains("flow_already_terminal")` guard at session_service.rs:209.
///
/// This test pins that behavior so a regression in the error-swallow branch
/// (or a contract drift on the FF side that changes the error code) is
/// caught immediately. Post-condition Valkey assertion confirms the flow
/// remains in `cancelled` state (i.e. the idempotent path did not corrupt
/// the hash).
#[tokio::test]
async fn test_double_archive_is_idempotent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();

    h.fabric
        .sessions
        .create(&h.project, session_id.clone())
        .await
        .expect("create failed");

    let first = h
        .fabric
        .sessions
        .archive(&h.project, &session_id)
        .await
        .expect("first archive failed");
    assert_eq!(first.state, SessionState::Archived);

    let second = h
        .fabric
        .sessions
        .archive(&h.project, &session_id)
        .await
        .expect("second archive must be idempotent, not error");
    assert_eq!(
        second.state,
        SessionState::Archived,
        "second archive must still report Archived, got {:?}",
        second.state,
    );
    assert_eq!(
        second.session_id, first.session_id,
        "second archive must return the same session record"
    );

    // Post-condition: flow_core still reports cancelled. Anything else
    // (e.g. "open" from an accidental re-create) would indicate the
    // idempotent branch corrupted state.
    let post = read_flow_core(&h, &session_id).await;
    assert_eq!(
        post.get("public_flow_state").map(|s| s.as_str()),
        Some("cancelled"),
        "flow_core.public_flow_state must REMAIN 'cancelled' after double-archive, got {:?}",
        post.get("public_flow_state"),
    );

    h.teardown().await;
}
