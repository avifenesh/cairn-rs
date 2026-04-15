use cairn_domain::tenancy::ProjectKey;
use cairn_domain::{RunId, SessionId, TenantId};
use ff_core::types::{ExecutionId, FlowId, LaneId, Namespace};
use uuid::Uuid;

const CAIRN_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x14, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

pub fn run_to_execution_id(run_id: &RunId) -> ExecutionId {
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, run_id.as_str().as_bytes());
    ExecutionId::from_uuid(uuid)
}

pub fn session_to_flow_id(session_id: &SessionId) -> FlowId {
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, session_id.as_str().as_bytes());
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

    #[test]
    fn run_to_execution_id_deterministic() {
        let run_id = RunId::new("run_123");
        let eid1 = run_to_execution_id(&run_id);
        let eid2 = run_to_execution_id(&run_id);
        assert_eq!(eid1, eid2);
    }

    #[test]
    fn different_runs_produce_different_ids() {
        let eid1 = run_to_execution_id(&RunId::new("run_a"));
        let eid2 = run_to_execution_id(&RunId::new("run_b"));
        assert_ne!(eid1, eid2);
    }

    #[test]
    fn session_to_flow_id_deterministic() {
        let sid = SessionId::new("sess_1");
        let fid1 = session_to_flow_id(&sid);
        let fid2 = session_to_flow_id(&sid);
        assert_eq!(fid1, fid2);
    }

    #[test]
    fn different_sessions_produce_different_flow_ids() {
        let fid1 = session_to_flow_id(&SessionId::new("sess_a"));
        let fid2 = session_to_flow_id(&SessionId::new("sess_b"));
        assert_ne!(fid1, fid2);
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
