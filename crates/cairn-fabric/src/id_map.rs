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
/// This is the phase-2 helper for session-attached executions: once the
/// orchestrator grant-flow migration lands, per-session runs will route
/// via `ExecutionId::deterministic_for_flow` so the partition index is
/// derived from the session's FlowId (grouping all runs of one session
/// onto the same partition). The signature is landed here so callers
/// can migrate incrementally; wiring into service callers is phase 2
/// scope.
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

pub fn tenant_to_namespace(tenant_id: &TenantId) -> Namespace {
    let s = tenant_id.as_str().trim();
    if s.is_empty() {
        Namespace::new("default")
    } else {
        Namespace::new(s)
    }
}

pub fn project_to_lane(project: &ProjectKey) -> LaneId {
    LaneId::new(format!(
        "{}/{}/{}",
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
        assert_eq!(lane.as_str(), "t1/w1/p1");
    }
}
