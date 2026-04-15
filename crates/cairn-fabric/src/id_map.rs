use cairn_domain::tenancy::ProjectKey;
use cairn_domain::{RunId, SessionId, TenantId};
use ff_core::types::{ExecutionId, FlowId, LaneId, Namespace};
use uuid::Uuid;

// Project-specific namespace UUID for deterministic v5 generation.
// Generated once, never changes — all cairn ID mappings derive from this.
const CAIRN_NAMESPACE: Uuid = Uuid::from_bytes([
    0xa3, 0x4e, 0x7c, 0x01, 0xf8, 0x2d, 0x4b, 0x9a, 0x91, 0x5c, 0xd7, 0x6e, 0x3a, 0x1b, 0x58, 0xf0,
]);

pub fn run_to_execution_id(run_id: &RunId) -> ExecutionId {
    let input = format!("run:{}", run_id.as_str());
    let uuid = Uuid::new_v5(&CAIRN_NAMESPACE, input.as_bytes());
    ExecutionId::from_uuid(uuid)
}

pub fn session_to_flow_id(session_id: &SessionId) -> FlowId {
    let input = format!("session:{}", session_id.as_str());
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
    fn same_string_different_entity_no_collision() {
        let eid = run_to_execution_id(&RunId::new("abc"));
        let fid = session_to_flow_id(&SessionId::new("abc"));
        assert_ne!(eid.to_string(), fid.to_string());
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
