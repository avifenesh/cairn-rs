use cairn_domain::tenancy::ProjectKey;
use cairn_domain::{RunId, SessionId, TaskId, TenantId};
use ff_core::types::{ExecutionId, FlowId, LaneId, Namespace};
use uuid::Uuid;

// FINAL — changing this UUID orphans all existing execution/flow IDs in Valkey.
// All cairn ID mappings derive from this namespace via UUID v5.
const CAIRN_NAMESPACE: Uuid = Uuid::from_bytes([
    0xa3, 0x4e, 0x7c, 0x01, 0xf8, 0x2d, 0x4b, 0x9a, 0x91, 0x5c, 0xd7, 0x6e, 0x3a, 0x1b, 0x58, 0xf0,
]);

pub fn run_to_execution_id(project: &ProjectKey, run_id: &RunId) -> ExecutionId {
    let input = format!(
        "run:\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, run_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    ExecutionId::from_uuid(uuid)
}

pub fn task_to_execution_id(project: &ProjectKey, task_id: &TaskId) -> ExecutionId {
    let input = format!(
        "task:\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, task_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    ExecutionId::from_uuid(uuid)
}

pub fn session_to_flow_id(project: &ProjectKey, session_id: &SessionId) -> FlowId {
    let input = format!(
        "session:\0{}\0{}\0{}\0{}",
        project.tenant_id, project.workspace_id, project.project_id, session_id
    );
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    FlowId::from_uuid(uuid)
}

pub fn tenant_to_namespace(tenant_id: &TenantId) -> Namespace {
    Namespace::new(tenant_id.as_str())
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
        let run_id = RunId::new("run_123");
        let eid1 = run_to_execution_id(&p, &run_id);
        let eid2 = run_to_execution_id(&p, &run_id);
        assert_eq!(eid1, eid2);
    }

    #[test]
    fn different_runs_produce_different_ids() {
        let p = test_project();
        let eid1 = run_to_execution_id(&p, &RunId::new("run_a"));
        let eid2 = run_to_execution_id(&p, &RunId::new("run_b"));
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn same_run_different_tenants_no_collision() {
        let p1 = ProjectKey::new("tenant_a", "w", "p");
        let p2 = ProjectKey::new("tenant_b", "w", "p");
        let eid1 = run_to_execution_id(&p1, &RunId::new("run_1"));
        let eid2 = run_to_execution_id(&p2, &RunId::new("run_1"));
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn same_run_different_projects_no_collision() {
        let p1 = ProjectKey::new("t", "w", "project_a");
        let p2 = ProjectKey::new("t", "w", "project_b");
        let eid1 = run_to_execution_id(&p1, &RunId::new("run_1"));
        let eid2 = run_to_execution_id(&p2, &RunId::new("run_1"));
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
        let eid = run_to_execution_id(&p, &RunId::new("abc"));
        let fid = session_to_flow_id(&p, &SessionId::new("abc"));
        assert_ne!(eid.to_string(), fid.to_string());
    }

    #[test]
    fn delimiter_collision_impossible() {
        let p1 = ProjectKey::new("a:b", "c", "d");
        let p2 = ProjectKey::new("a", "b:c", "d");
        let eid1 = run_to_execution_id(&p1, &RunId::new("run_1"));
        let eid2 = run_to_execution_id(&p2, &RunId::new("run_1"));
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
        let tid = TaskId::new("task_1");
        let eid1 = task_to_execution_id(&p, &tid);
        let eid2 = task_to_execution_id(&p, &tid);
        assert_eq!(eid1, eid2);
    }

    #[test]
    fn task_and_run_same_string_no_collision() {
        let p = test_project();
        let eid_run = run_to_execution_id(&p, &RunId::new("abc"));
        let eid_task = task_to_execution_id(&p, &TaskId::new("abc"));
        assert_ne!(eid_run, eid_task);
    }

    #[test]
    fn different_tasks_produce_different_ids() {
        let p = test_project();
        let eid1 = task_to_execution_id(&p, &TaskId::new("task_a"));
        let eid2 = task_to_execution_id(&p, &TaskId::new("task_b"));
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn same_task_different_projects_no_collision() {
        let p1 = ProjectKey::new("t", "w", "project_a");
        let p2 = ProjectKey::new("t", "w", "project_b");
        let eid1 = task_to_execution_id(&p1, &TaskId::new("task_1"));
        let eid2 = task_to_execution_id(&p2, &TaskId::new("task_1"));
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
