//! Deterministic cairn → FlowFabric ID mapping.
//!
//! # Partition-count stability contract
//!
//! Partition-count stability is a hard requirement: changing
//! `num_flow_partitions` orphans existing `ExecutionId`s in Valkey. The
//! ExecutionId encoding baked in by `ExecutionId::deterministic_solo` /
//! `ExecutionId::deterministic_for_flow` includes the partition index
//! computed against the `PartitionConfig` in effect at mint time; if the
//! config changes after the ID is minted, Valkey lookups will target a
//! different partition than the one the data was written to. See FF
//! `ExecutionId::deterministic_*` rustdoc for the full derivation.
//!
//! All mint helpers in this module therefore take `&PartitionConfig`
//! explicitly so callers are forced to thread the cluster-wide config
//! through (rather than reaching for a default).
//!
//! The underlying UUID-v5 derivation (namespace + per-entity input
//! string) is unchanged from v1: we continue to mint a stable
//! `Uuid::new_v5(&CAIRN_NAMESPACE, input)` and hand that UUID to the FF
//! constructor. Changing the CAIRN_NAMESPACE or NAMESPACE_VERSION is a
//! second, independent way to orphan IDs (and is reserved for explicit
//! migration events).

use cairn_domain::tenancy::ProjectKey;
use cairn_domain::{RunId, SessionId, TaskId, TenantId};
use ff_core::partition::PartitionConfig;
use ff_core::types::{ExecutionId, FlowId, LaneId, Namespace};
use uuid::Uuid;

// Stable namespace UUID for all cairn→FF ID mappings (UUID v5).
// Changing this orphans all existing execution/flow IDs in Valkey.
// Migration path: increment NAMESPACE_VERSION, rebuild executions from
// cairn's EventLog (which retains the original RunId/TaskId/SessionId).
const CAIRN_NAMESPACE: Uuid = Uuid::from_bytes([
    0xa3, 0x4e, 0x7c, 0x01, 0xf8, 0x2d, 0x4b, 0x9a, 0x91, 0x5c, 0xd7, 0x6e, 0x3a, 0x1b, 0x58, 0xf0,
]);

const NAMESPACE_VERSION: u8 = 1;

/// Mint a deterministic `ExecutionId` for a cairn run in solo (no-parent-flow) mode.
///
/// Uses `ExecutionId::deterministic_solo` so the resulting ID carries the
/// project's `LaneId` and a partition index derived from `config`. The
/// partition index MUST remain stable across the cluster's lifetime —
/// see module docs for the contract.
pub fn run_to_execution_id(
    project: &ProjectKey,
    run_id: &RunId,
    config: &PartitionConfig,
) -> ExecutionId {
    let input = format!(
        "v{NAMESPACE_VERSION}:run:\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, run_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    ExecutionId::deterministic_solo(&project_to_lane(project), config, uuid)
}

/// Mint a deterministic `ExecutionId` for a cairn task in solo mode.
///
/// See [`run_to_execution_id`] for the partition-stability contract.
pub fn task_to_execution_id(
    project: &ProjectKey,
    task_id: &TaskId,
    config: &PartitionConfig,
) -> ExecutionId {
    let input = format!(
        "v{NAMESPACE_VERSION}:task:\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, task_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    ExecutionId::deterministic_solo(&project_to_lane(project), config, uuid)
}

/// Mint a `FlowId` for a cairn session. Unchanged from v1: `FlowId::from_uuid`
/// still exists at FF rev 1b19dd10 and does not depend on `PartitionConfig`
/// because a `FlowId` is a pure handle, not a routed execution address.
pub fn session_to_flow_id(project: &ProjectKey, session_id: &SessionId) -> FlowId {
    let input = format!(
        "v{NAMESPACE_VERSION}:session:\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, session_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    FlowId::from_uuid(uuid)
}

/// Mint an `ExecutionId` scoped to a session's `FlowId` for a given run.
///
/// RFC-011 Phase 2: per-session runs route via
/// `ExecutionId::deterministic_for_flow` so the partition index is derived
/// from the session's FlowId. This co-locates every run of a session on
/// the same Valkey partition, which is the whole point of the
/// `{fp:N}:<uuid>` hash-tag scheme.
pub fn session_run_to_execution_id(
    project: &ProjectKey,
    session_id: &SessionId,
    run_id: &RunId,
    config: &PartitionConfig,
) -> ExecutionId {
    let input = format!(
        "v{NAMESPACE_VERSION}:session_run:\0{}\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, session_id, run_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    let flow = session_to_flow_id(project, session_id);
    ExecutionId::deterministic_for_flow(&flow, config, uuid)
}

/// Mint an `ExecutionId` scoped to a session's `FlowId` for a given task.
///
/// RFC-011 Phase 2: per-session tasks share the session's FF FlowId just
/// like runs, so `execution_partition` lands them on the same partition
/// as every other run/task of the same session.
pub fn session_task_to_execution_id(
    project: &ProjectKey,
    session_id: &SessionId,
    task_id: &TaskId,
    config: &PartitionConfig,
) -> ExecutionId {
    let input = format!(
        "v{NAMESPACE_VERSION}:session_task:\0{}\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, session_id, task_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    let flow = session_to_flow_id(project, session_id);
    ExecutionId::deterministic_for_flow(&flow, config, uuid)
}

/// Map a cairn `TenantId` to an FF `Namespace`.
///
/// Empty/whitespace tenants map to the shared `"default"` namespace.
/// Callers MUST validate `tenant_id` is non-empty at the API boundary
/// (see `cairn-app::validate::require_id`) to avoid tenant-scope
/// collapse. This fallback exists only for bootstrap tooling that
/// legitimately uses `"default"` as its tenant.
pub fn tenant_to_namespace(tenant_id: &TenantId) -> Namespace {
    let s = tenant_id.as_str().trim();
    if s.is_empty() {
        Namespace::new("default")
    } else {
        Namespace::new(s)
    }
}

/// Mint a `LaneId` from a `ProjectKey`.
///
/// Uses null-byte delimiters so projects whose ids contain slashes
/// (e.g. `tenant="a/b"` + `workspace="c"` vs `tenant="a"` +
/// `workspace="b/c"`) produce distinct `LaneId`s. Cross-project key
/// collisions on the FF side would otherwise merge unrelated tenants'
/// routing state.
pub fn project_to_lane(project: &ProjectKey) -> LaneId {
    LaneId::new(format!(
        "v{NAMESPACE_VERSION}:project:\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_project() -> ProjectKey {
        ProjectKey::new("t1", "w1", "p1")
    }

    #[test]
    fn run_to_execution_id_deterministic() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let run_id = RunId::new("run_123");
        let eid1 = run_to_execution_id(&p, &run_id, &cfg);
        let eid2 = run_to_execution_id(&p, &run_id, &cfg);
        assert_eq!(eid1, eid2);
    }

    #[test]
    fn deterministic_solo_stable_across_calls() {
        // Core Option B invariant: repeated calls with identical
        // (project, run, config) inputs MUST return byte-identical
        // ExecutionIds — otherwise Valkey lookups would miss and
        // orphan in-flight work.
        let p = test_project();
        let cfg = PartitionConfig::default();
        let run_id = RunId::new("run_stability");
        let eid1 = run_to_execution_id(&p, &run_id, &cfg);
        let eid2 = run_to_execution_id(&p, &run_id, &cfg);
        let eid3 = run_to_execution_id(&p, &run_id, &cfg);
        assert_eq!(eid1, eid2);
        assert_eq!(eid2, eid3);
        // And task path too.
        let tid = TaskId::new("task_stability");
        let tid1 = task_to_execution_id(&p, &tid, &cfg);
        let tid2 = task_to_execution_id(&p, &tid, &cfg);
        assert_eq!(tid1, tid2);
    }

    #[test]
    fn different_runs_produce_different_ids() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let eid1 = run_to_execution_id(&p, &RunId::new("run_a"), &cfg);
        let eid2 = run_to_execution_id(&p, &RunId::new("run_b"), &cfg);
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn same_run_different_tenants_no_collision() {
        let cfg = PartitionConfig::default();
        let p1 = ProjectKey::new("tenant_a", "w", "p");
        let p2 = ProjectKey::new("tenant_b", "w", "p");
        let eid1 = run_to_execution_id(&p1, &RunId::new("run_1"), &cfg);
        let eid2 = run_to_execution_id(&p2, &RunId::new("run_1"), &cfg);
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn same_run_different_projects_no_collision() {
        let cfg = PartitionConfig::default();
        let p1 = ProjectKey::new("t", "w", "project_a");
        let p2 = ProjectKey::new("t", "w", "project_b");
        let eid1 = run_to_execution_id(&p1, &RunId::new("run_1"), &cfg);
        let eid2 = run_to_execution_id(&p2, &RunId::new("run_1"), &cfg);
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn session_to_flow_id_deterministic() {
        let p = test_project();
        let sid = SessionId::new("sess_1");
        let fid1 = session_to_flow_id(&p, &sid);
        let fid2 = session_to_flow_id(&p, &sid);
        assert_eq!(fid1, fid2);
    }

    #[test]
    fn different_sessions_produce_different_flow_ids() {
        let p = test_project();
        let fid1 = session_to_flow_id(&p, &SessionId::new("sess_a"));
        let fid2 = session_to_flow_id(&p, &SessionId::new("sess_b"));
        assert_ne!(fid1, fid2);
    }

    #[test]
    fn same_session_different_tenants_no_collision() {
        let p1 = ProjectKey::new("tenant_a", "w", "p");
        let p2 = ProjectKey::new("tenant_b", "w", "p");
        let fid1 = session_to_flow_id(&p1, &SessionId::new("sess_1"));
        let fid2 = session_to_flow_id(&p2, &SessionId::new("sess_1"));
        assert_ne!(fid1, fid2);
    }

    #[test]
    fn same_string_different_entity_no_collision() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let eid = run_to_execution_id(&p, &RunId::new("abc"), &cfg);
        let fid = session_to_flow_id(&p, &SessionId::new("abc"));
        assert_ne!(eid.to_string(), fid.to_string());
    }

    #[test]
    fn delimiter_collision_impossible() {
        let cfg = PartitionConfig::default();
        let p1 = ProjectKey::new("a:b", "c", "d");
        let p2 = ProjectKey::new("a", "b:c", "d");
        let eid1 = run_to_execution_id(&p1, &RunId::new("run_1"), &cfg);
        let eid2 = run_to_execution_id(&p2, &RunId::new("run_1"), &cfg);
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn session_delimiter_collision_impossible() {
        let p1 = ProjectKey::new("a:b", "c", "d");
        let p2 = ProjectKey::new("a", "b:c", "d");
        let fid1 = session_to_flow_id(&p1, &SessionId::new("sess_1"));
        let fid2 = session_to_flow_id(&p2, &SessionId::new("sess_1"));
        assert_ne!(fid1, fid2);
    }

    #[test]
    fn task_to_execution_id_deterministic() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let tid = TaskId::new("task_1");
        let eid1 = task_to_execution_id(&p, &tid, &cfg);
        let eid2 = task_to_execution_id(&p, &tid, &cfg);
        assert_eq!(eid1, eid2);
    }

    #[test]
    fn task_and_run_same_string_no_collision() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let eid_run = run_to_execution_id(&p, &RunId::new("abc"), &cfg);
        let eid_task = task_to_execution_id(&p, &TaskId::new("abc"), &cfg);
        assert_ne!(eid_run, eid_task);
    }

    #[test]
    fn different_tasks_produce_different_ids() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let eid1 = task_to_execution_id(&p, &TaskId::new("task_a"), &cfg);
        let eid2 = task_to_execution_id(&p, &TaskId::new("task_b"), &cfg);
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn same_task_different_projects_no_collision() {
        let cfg = PartitionConfig::default();
        let p1 = ProjectKey::new("t", "w", "project_a");
        let p2 = ProjectKey::new("t", "w", "project_b");
        let eid1 = task_to_execution_id(&p1, &TaskId::new("task_1"), &cfg);
        let eid2 = task_to_execution_id(&p2, &TaskId::new("task_1"), &cfg);
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn tenant_to_namespace_preserves_value() {
        let ns = tenant_to_namespace(&TenantId::new("acme"));
        assert_eq!(ns.as_str(), "acme");
    }

    #[test]
    fn project_to_lane_format() {
        let project = ProjectKey::new("t1", "w1", "p1");
        let lane = project_to_lane(&project);
        assert_eq!(lane.as_str(), "v1:project:\0t1\0w1\0p1");
    }

    /// F02 regression guard: with slash-delimited LaneIds, ProjectKey("a/b","c","d")
    /// and ProjectKey("a","b/c","d") would BOTH produce `"a/b/c/d"` and alias onto
    /// the same FF routing lane. Null-byte delimiters prevent that collision —
    /// same pattern as the UUID-v5 inputs upstream.
    #[test]
    fn project_to_lane_no_cross_project_collision() {
        let lane_1 = project_to_lane(&ProjectKey::new("a/b", "c", "d"));
        let lane_2 = project_to_lane(&ProjectKey::new("a", "b/c", "d"));
        assert_ne!(lane_1, lane_2);
    }

    // ── session_run_to_execution_id coverage ───────────────────────────

    #[test]
    fn session_run_to_execution_id_deterministic() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let sid = SessionId::new("sess_stable");
        let rid = RunId::new("run_stable");
        let e1 = session_run_to_execution_id(&p, &sid, &rid, &cfg);
        let e2 = session_run_to_execution_id(&p, &sid, &rid, &cfg);
        let e3 = session_run_to_execution_id(&p, &sid, &rid, &cfg);
        assert_eq!(e1, e2);
        assert_eq!(e2, e3);
    }

    #[test]
    fn session_run_to_execution_id_different_runs_differ() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let sid = SessionId::new("sess_1");
        let e1 = session_run_to_execution_id(&p, &sid, &RunId::new("run_a"), &cfg);
        let e2 = session_run_to_execution_id(&p, &sid, &RunId::new("run_b"), &cfg);
        assert_ne!(e1, e2);
    }

    #[test]
    fn session_run_to_execution_id_different_sessions_differ() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let rid = RunId::new("run_shared");
        let e1 = session_run_to_execution_id(&p, &SessionId::new("sess_a"), &rid, &cfg);
        let e2 = session_run_to_execution_id(&p, &SessionId::new("sess_b"), &rid, &cfg);
        assert_ne!(e1, e2);
    }

    /// session_run_to_execution_id routes via `deterministic_for_flow` which
    /// derives its partition from the session's FlowId. run_to_execution_id
    /// routes via `deterministic_solo` keyed on the project's LaneId. Even
    /// with identical `(project, run_id)`, the two paths MUST produce
    /// distinct ExecutionIds — otherwise a session-attached run would
    /// alias onto the solo run's FF state.
    #[test]
    fn session_run_to_execution_id_different_from_run() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let rid = RunId::new("run_shared_between_paths");
        let solo = run_to_execution_id(&p, &rid, &cfg);
        let via_session = session_run_to_execution_id(&p, &SessionId::new("sess_x"), &rid, &cfg);
        assert_ne!(solo, via_session);
    }

    /// SEC-001: delimiter-collision guard for the 5-field session_run input.
    /// Every naive single-char delimiter scheme has an input pair that
    /// collides; the null-byte scheme doesn't. Pin several concrete pairs.
    #[test]
    fn session_run_delimiter_collision_impossible() {
        let cfg = PartitionConfig::default();

        // tenant boundary
        let e1 = session_run_to_execution_id(
            &ProjectKey::new("a:b", "c", "d"),
            &SessionId::new("s"),
            &RunId::new("r1"),
            &cfg,
        );
        let e2 = session_run_to_execution_id(
            &ProjectKey::new("a", "b:c", "d"),
            &SessionId::new("s"),
            &RunId::new("r1"),
            &cfg,
        );
        assert_ne!(e1, e2);

        // workspace/project boundary
        let e3 = session_run_to_execution_id(
            &ProjectKey::new("t", "w:p", "q"),
            &SessionId::new("s"),
            &RunId::new("r1"),
            &cfg,
        );
        let e4 = session_run_to_execution_id(
            &ProjectKey::new("t", "w", "p:q"),
            &SessionId::new("s"),
            &RunId::new("r1"),
            &cfg,
        );
        assert_ne!(e3, e4);

        // session/run boundary
        let e5 = session_run_to_execution_id(
            &test_project(),
            &SessionId::new("s:r"),
            &RunId::new("1"),
            &cfg,
        );
        let e6 = session_run_to_execution_id(
            &test_project(),
            &SessionId::new("s"),
            &RunId::new("r:1"),
            &cfg,
        );
        assert_ne!(e5, e6);
    }

    /// TC#1/#3: The partition index chosen at mint time MUST be baked into
    /// the resulting ExecutionId. If `num_flow_partitions` is ever changed
    /// after IDs are minted, lookups will target a different partition from
    /// where the state was written. Proves the invariant by minting with
    /// two distinct configs and asserting the IDs differ.
    #[test]
    fn deterministic_solo_respects_partition_config() {
        let p = test_project();
        let rid = RunId::new("run_partition_sensitive");
        let cfg_64 = PartitionConfig {
            num_flow_partitions: 64,
            num_budget_partitions: 32,
            num_quota_partitions: 32,
        };
        let cfg_256 = PartitionConfig {
            num_flow_partitions: 256,
            num_budget_partitions: 32,
            num_quota_partitions: 32,
        };
        let e_64 = run_to_execution_id(&p, &rid, &cfg_64);
        let e_256 = run_to_execution_id(&p, &rid, &cfg_256);
        // At least one of the 64/256 partition indices must differ for some
        // input; this fixture is chosen to demonstrate the general invariant.
        // The assertion is that partition count DOES affect the output — if
        // the two IDs are byte-equal, `deterministic_solo` is ignoring the
        // config, which is the exact regression we're guarding against.
        assert_ne!(
            e_64, e_256,
            "deterministic_solo must incorporate partition_config into the \
             resulting ExecutionId; got identical ids across 64-vs-256 configs"
        );
    }

    /// TC#3: broad sanity — five distinct (project, run) pairs must produce
    /// five pairwise-distinct ExecutionIds, and each must be self-stable.
    #[test]
    fn deterministic_solo_multiple_fixtures() {
        let cfg = PartitionConfig::default();
        let fixtures: [(ProjectKey, RunId); 5] = [
            (ProjectKey::new("t1", "w1", "p1"), RunId::new("r_alpha")),
            (ProjectKey::new("t2", "w1", "p1"), RunId::new("r_alpha")),
            (ProjectKey::new("t1", "w2", "p1"), RunId::new("r_alpha")),
            (ProjectKey::new("t1", "w1", "p2"), RunId::new("r_alpha")),
            (ProjectKey::new("t1", "w1", "p1"), RunId::new("r_beta")),
        ];
        let ids: Vec<ExecutionId> = fixtures
            .iter()
            .map(|(p, r)| run_to_execution_id(p, r, &cfg))
            .collect();

        // Self-stable.
        for (i, (p, r)) in fixtures.iter().enumerate() {
            assert_eq!(ids[i], run_to_execution_id(p, r, &cfg));
        }

        // Pairwise distinct.
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j], "fixtures {i} and {j} collided");
            }
        }
    }

    /// TC#4: pin the CAIRN_NAMESPACE UUID bytes against accidental rewrite.
    /// Any change here silently orphans every ExecutionId / FlowId in
    /// every Valkey cluster cairn ever wrote to — must be a deliberate
    /// migration event tied to NAMESPACE_VERSION, never a casual edit.
    #[test]
    fn cairn_namespace_uuid_stable() {
        assert_eq!(
            CAIRN_NAMESPACE.as_bytes(),
            &[
                0xa3, 0x4e, 0x7c, 0x01, 0xf8, 0x2d, 0x4b, 0x9a, 0x91, 0x5c, 0xd7, 0x6e, 0x3a, 0x1b,
                0x58, 0xf0,
            ]
        );
        assert_eq!(NAMESPACE_VERSION, 1);
    }

    // ── tenant_to_namespace edge cases (SEC-003) ───────────────────────

    #[test]
    fn tenant_to_namespace_empty_maps_to_default() {
        let ns = tenant_to_namespace(&TenantId::new(""));
        assert_eq!(ns.as_str(), "default");
    }

    #[test]
    fn tenant_to_namespace_whitespace_maps_to_default() {
        let ns = tenant_to_namespace(&TenantId::new("   "));
        assert_eq!(ns.as_str(), "default");
    }

    #[test]
    fn tenant_to_namespace_trims_surrounding() {
        let ns = tenant_to_namespace(&TenantId::new(" acme "));
        assert_eq!(ns.as_str(), "acme");
    }

    // ── session_task_to_execution_id coverage ──────────────────────────

    #[test]
    fn session_task_to_execution_id_deterministic() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let sid = SessionId::new("sess_task_stable");
        let tid = TaskId::new("task_stable");
        let e1 = session_task_to_execution_id(&p, &sid, &tid, &cfg);
        let e2 = session_task_to_execution_id(&p, &sid, &tid, &cfg);
        let e3 = session_task_to_execution_id(&p, &sid, &tid, &cfg);
        assert_eq!(e1, e2);
        assert_eq!(e2, e3);
    }

    #[test]
    fn same_task_different_sessions_no_collision() {
        let p = test_project();
        let cfg = PartitionConfig::default();
        let tid = TaskId::new("task_shared_across_sessions");
        let e1 = session_task_to_execution_id(&p, &SessionId::new("sess_a"), &tid, &cfg);
        let e2 = session_task_to_execution_id(&p, &SessionId::new("sess_b"), &tid, &cfg);
        assert_ne!(e1, e2);
    }

    #[test]
    fn session_task_to_execution_id_different_from_task() {
        // session_task_to_execution_id routes via `deterministic_for_flow`
        // while task_to_execution_id routes via `deterministic_solo`.
        // Identical (project, task_id) pairs must still produce distinct
        // ExecutionIds across the two paths — see
        // session_run_to_execution_id_different_from_run for the run-side
        // mirror.
        let p = test_project();
        let cfg = PartitionConfig::default();
        let tid = TaskId::new("task_shared_between_paths");
        let solo = task_to_execution_id(&p, &tid, &cfg);
        let via_session =
            session_task_to_execution_id(&p, &SessionId::new("sess_x"), &tid, &cfg);
        assert_ne!(solo, via_session);
    }

    #[test]
    fn session_task_and_session_run_no_collision() {
        // Same session, same id-string — the entity-kind prefix in the
        // UUID-v5 input must keep task and run separate.
        let p = test_project();
        let cfg = PartitionConfig::default();
        let sid = SessionId::new("sess_collide");
        let e_run = session_run_to_execution_id(&p, &sid, &RunId::new("abc"), &cfg);
        let e_task = session_task_to_execution_id(&p, &sid, &TaskId::new("abc"), &cfg);
        assert_ne!(e_run, e_task);
    }

    #[test]
    fn session_task_delimiter_collision_impossible() {
        let cfg = PartitionConfig::default();
        let e1 = session_task_to_execution_id(
            &ProjectKey::new("a:b", "c", "d"),
            &SessionId::new("s"),
            &TaskId::new("t1"),
            &cfg,
        );
        let e2 = session_task_to_execution_id(
            &ProjectKey::new("a", "b:c", "d"),
            &SessionId::new("s"),
            &TaskId::new("t1"),
            &cfg,
        );
        assert_ne!(e1, e2);

        let e3 = session_task_to_execution_id(
            &test_project(),
            &SessionId::new("s:t"),
            &TaskId::new("1"),
            &cfg,
        );
        let e4 = session_task_to_execution_id(
            &test_project(),
            &SessionId::new("s"),
            &TaskId::new("t:1"),
            &cfg,
        );
        assert_ne!(e3, e4);
    }

    /// RFC-011 co-location invariant: runs and tasks scoped to the same
    /// session must land on the same `execution_partition(&eid, &cfg)`
    /// result. This is the point of the session-scoped ExecutionId path —
    /// the partition index is derived from the session's FlowId, so every
    /// execution that uses `deterministic_for_flow(&flow, ...)` shares a
    /// partition with every other execution of the same flow.
    #[test]
    fn same_session_runs_and_tasks_same_partition() {
        use ff_core::partition::execution_partition;
        let p = test_project();
        let cfg = PartitionConfig::default();
        let sid = SessionId::new("sess_colocate");

        let run_a = session_run_to_execution_id(&p, &sid, &RunId::new("run_1"), &cfg);
        let run_b = session_run_to_execution_id(&p, &sid, &RunId::new("run_2"), &cfg);
        let task_a = session_task_to_execution_id(&p, &sid, &TaskId::new("task_1"), &cfg);
        let task_b = session_task_to_execution_id(&p, &sid, &TaskId::new("task_2"), &cfg);

        let p_run_a = execution_partition(&run_a, &cfg);
        let p_run_b = execution_partition(&run_b, &cfg);
        let p_task_a = execution_partition(&task_a, &cfg);
        let p_task_b = execution_partition(&task_b, &cfg);

        assert_eq!(p_run_a, p_run_b, "runs in same session must co-locate");
        assert_eq!(p_run_a, p_task_a, "task must co-locate with runs in same session");
        assert_eq!(p_task_a, p_task_b, "tasks in same session must co-locate");
    }
}
