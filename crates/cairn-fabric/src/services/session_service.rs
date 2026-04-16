use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::lifecycle::SessionState;
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::SessionId;
use cairn_store::projections::SessionRecord;
use ff_core::keys::FlowKeyContext;
use ff_core::partition::{flow_partition, Partition};
use ff_core::types::{FlowId, Namespace, TimestampMs};

use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::parse_project_key;
use crate::id_map;

pub struct FabricSessionService {
    runtime: Arc<FabricRuntime>,
    bridge: Arc<EventBridge>,
}

impl FabricSessionService {
    pub fn new(runtime: Arc<FabricRuntime>, bridge: Arc<EventBridge>) -> Self {
        Self { runtime, bridge }
    }

    fn flow_id(&self, project: &ProjectKey, session_id: &SessionId) -> FlowId {
        id_map::session_to_flow_id(project, session_id)
    }

    fn flow_partition(&self, fid: &FlowId) -> Partition {
        flow_partition(fid, &self.runtime.partition_config)
    }

    fn namespace(&self, project: &ProjectKey) -> Namespace {
        id_map::tenant_to_namespace(&project.tenant_id)
    }

    async fn read_flow_summary(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
    ) -> Result<Option<HashMap<String, String>>, FabricError> {
        let fid = self.flow_id(project, session_id);
        let partition = self.flow_partition(&fid);
        let fctx = FlowKeyContext::new(&partition, &fid);

        let fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&fctx.summary())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL flow summary: {e}")))?;

        if fields.is_empty() {
            return Ok(None);
        }
        Ok(Some(fields))
    }

    async fn read_session_record(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, FabricError> {
        let fid = self.flow_id(project, session_id);
        let partition = self.flow_partition(&fid);
        let fctx = FlowKeyContext::new(&partition, &fid);

        let core: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&fctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL flow core: {e}")))?;

        if core.is_empty() {
            return Ok(None);
        }

        let summary = self
            .read_flow_summary(project, session_id)
            .await?
            .unwrap_or_default();

        Ok(Some(build_session_record(session_id, &core, &summary)))
    }

    pub async fn create(
        &self,
        project: &ProjectKey,
        session_id: SessionId,
    ) -> Result<SessionRecord, FabricError> {
        let fid = self.flow_id(project, &session_id);
        let partition = self.flow_partition(&fid);
        let fctx = FlowKeyContext::new(&partition, &fid);
        let namespace = self.namespace(project);
        let now = TimestampMs::now();

        let project_str = format!(
            "{}/{}/{}",
            project.tenant_id, project.workspace_id, project.project_id
        );

        let keys: Vec<String> = vec![fctx.core(), fctx.members()];
        let args: Vec<String> = vec![
            fid.to_string(),
            "cairn_session".to_owned(),
            namespace.to_string(),
            now.to_string(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_create_flow", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_flow: {e}")))?;

        let _: i64 = self
            .runtime
            .client
            .hset(&fctx.core(), "cairn.project", &project_str)
            .await
            .map_err(|e| FabricError::Internal(format!("hset cairn.project: {e}")))?;
        let _: i64 = self
            .runtime
            .client
            .hset(&fctx.core(), "cairn.session_id", session_id.as_str())
            .await
            .map_err(|e| FabricError::Internal(format!("hset cairn.session_id: {e}")))?;

        let now_ms = now.0 as u64;
        Ok(SessionRecord {
            session_id,
            project: project.clone(),
            state: SessionState::Open,
            version: 0,
            created_at: now_ms,
            updated_at: now_ms,
        })
    }

    pub async fn get(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, FabricError> {
        self.read_session_record(project, session_id).await
    }

    pub async fn list(
        &self,
        _project: &ProjectKey,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<SessionRecord>, FabricError> {
        // FF flows are partitioned by flow_id, not indexed by project.
        // The cairn-store projection serves list queries from the event log.
        Ok(Vec::new())
    }

    pub async fn archive(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
    ) -> Result<SessionRecord, FabricError> {
        let fid = self.flow_id(project, session_id);
        let partition = self.flow_partition(&fid);
        let fctx = FlowKeyContext::new(&partition, &fid);

        let exists: std::collections::HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&fctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL flow core: {e}")))?;

        if exists.is_empty() {
            return Err(FabricError::NotFound {
                entity: "session",
                id: session_id.to_string(),
            });
        }

        let now = TimestampMs::now();

        let keys: Vec<String> = vec![fctx.core(), fctx.members()];
        let args: Vec<String> = vec![
            fid.to_string(),
            "session archived".to_owned(),
            "cancel_all".to_owned(),
            now.to_string(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_cancel_flow", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_cancel_flow: {e}")))?;

        let _: i64 = self
            .runtime
            .client
            .hset(&fctx.core(), "cairn.archived", "true")
            .await
            .map_err(|e| FabricError::Internal(format!("hset cairn.archived: {e}")))?;

        match self.read_session_record(project, session_id).await? {
            Some(record) => {
                self.bridge.emit(BridgeEvent::SessionArchived {
                    session_id: session_id.clone(),
                    project: record.project.clone(),
                });
                Ok(record)
            }
            None => Err(FabricError::NotFound {
                entity: "session",
                id: session_id.to_string(),
            }),
        }
    }
}

fn flow_state_to_session_state(state: &str) -> SessionState {
    match state {
        "completed" => SessionState::Completed,
        "failed" => SessionState::Failed,
        "cancelled" => SessionState::Failed,
        _ => SessionState::Open,
    }
}

fn build_session_record(
    session_id: &SessionId,
    core: &HashMap<String, String>,
    summary: &HashMap<String, String>,
) -> SessionRecord {
    let project_str = core.get("cairn.project").cloned().unwrap_or_default();
    let project = parse_project_key(&project_str);

    let is_archived = core
        .get("cairn.archived")
        .map(|v| v == "true")
        .unwrap_or(false);
    let state = if is_archived {
        SessionState::Archived
    } else {
        let flow_state = summary
            .get("public_state")
            .or_else(|| core.get("state"))
            .cloned()
            .unwrap_or_default();
        flow_state_to_session_state(&flow_state)
    };

    let created_at = core
        .get("created_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let updated_at = summary
        .get("last_mutation_at")
        .or_else(|| core.get("last_mutation_at"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(created_at);
    let version = core
        .get("graph_revision")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    SessionRecord {
        session_id: session_id.clone(),
        project,
        state,
        version,
        created_at,
        updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_state_open_variants() {
        assert_eq!(flow_state_to_session_state("open"), SessionState::Open);
        assert_eq!(flow_state_to_session_state("running"), SessionState::Open);
        assert_eq!(flow_state_to_session_state("blocked"), SessionState::Open);
        assert_eq!(flow_state_to_session_state("waiting"), SessionState::Open);
        assert_eq!(flow_state_to_session_state(""), SessionState::Open);
        assert_eq!(flow_state_to_session_state("unknown"), SessionState::Open);
    }

    #[test]
    fn flow_state_terminal_variants() {
        assert_eq!(
            flow_state_to_session_state("completed"),
            SessionState::Completed
        );
        assert_eq!(flow_state_to_session_state("failed"), SessionState::Failed);
        assert_eq!(
            flow_state_to_session_state("cancelled"),
            SessionState::Failed
        );
    }

    #[test]
    fn session_id_maps_to_stable_flow_id() {
        let p = ProjectKey::new("t", "w", "p");
        let sid = SessionId::new("sess_42");
        let fid1 = id_map::session_to_flow_id(&p, &sid);
        let fid2 = id_map::session_to_flow_id(&p, &sid);
        assert_eq!(fid1, fid2);
    }

    #[test]
    fn different_sessions_different_flows() {
        let p = ProjectKey::new("t", "w", "p");
        let fid1 = id_map::session_to_flow_id(&p, &SessionId::new("sess_a"));
        let fid2 = id_map::session_to_flow_id(&p, &SessionId::new("sess_b"));
        assert_ne!(fid1, fid2);
    }

    #[test]
    fn build_record_completed_flow() {
        let sid = SessionId::new("sess_test");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "t/w/p".to_owned());
        core.insert("created_at".to_owned(), "1000".to_owned());
        core.insert("graph_revision".to_owned(), "3".to_owned());

        let mut summary = HashMap::new();
        summary.insert("public_state".to_owned(), "completed".to_owned());
        summary.insert("last_mutation_at".to_owned(), "2000".to_owned());

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.session_id.as_str(), "sess_test");
        assert_eq!(record.project.tenant_id.as_str(), "t");
        assert_eq!(record.project.workspace_id.as_str(), "w");
        assert_eq!(record.project.project_id.as_str(), "p");
        assert_eq!(record.state, SessionState::Completed);
        assert_eq!(record.version, 3);
        assert_eq!(record.created_at, 1000);
        assert_eq!(record.updated_at, 2000);
    }

    #[test]
    fn build_record_open_when_active() {
        let sid = SessionId::new("sess_active");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "t/w/p".to_owned());
        core.insert("created_at".to_owned(), "500".to_owned());

        let mut summary = HashMap::new();
        summary.insert("public_state".to_owned(), "running".to_owned());

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.state, SessionState::Open);
    }

    #[test]
    fn build_record_falls_back_to_core_state() {
        let sid = SessionId::new("sess_core");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "t/w/p".to_owned());
        core.insert("created_at".to_owned(), "100".to_owned());
        core.insert("state".to_owned(), "failed".to_owned());

        let summary = HashMap::new();

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.state, SessionState::Failed);
    }

    #[test]
    fn build_record_defaults_when_empty() {
        let sid = SessionId::new("sess_empty");
        let core = HashMap::new();
        let summary = HashMap::new();

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.state, SessionState::Open);
        assert_eq!(record.project.tenant_id.as_str(), "default_tenant");
        assert_eq!(record.version, 1);
        assert_eq!(record.created_at, 0);
    }

    #[test]
    fn build_record_updated_at_falls_back_to_created_at() {
        let sid = SessionId::new("sess_no_update");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "t/w/p".to_owned());
        core.insert("created_at".to_owned(), "999".to_owned());

        let summary = HashMap::new();

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.updated_at, 999);
    }

    #[test]
    fn build_record_failed_flow() {
        let sid = SessionId::new("sess_fail");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "x/y/z".to_owned());
        core.insert("created_at".to_owned(), "100".to_owned());

        let mut summary = HashMap::new();
        summary.insert("public_state".to_owned(), "failed".to_owned());

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.state, SessionState::Failed);
    }

    #[test]
    fn build_record_cancelled_maps_to_failed() {
        let sid = SessionId::new("sess_cancel");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "t/w/p".to_owned());
        core.insert("created_at".to_owned(), "100".to_owned());

        let mut summary = HashMap::new();
        summary.insert("public_state".to_owned(), "cancelled".to_owned());

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.state, SessionState::Failed);
    }

    #[test]
    fn build_record_archived_overrides_flow_state() {
        let sid = SessionId::new("sess_archived");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "t/w/p".to_owned());
        core.insert("created_at".to_owned(), "100".to_owned());
        core.insert("cairn.archived".to_owned(), "true".to_owned());

        let mut summary = HashMap::new();
        summary.insert("public_state".to_owned(), "cancelled".to_owned());

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.state, SessionState::Archived);
    }

    #[test]
    fn build_record_not_archived_when_flag_absent() {
        let sid = SessionId::new("sess_not_archived");
        let mut core = HashMap::new();
        core.insert("cairn.project".to_owned(), "t/w/p".to_owned());
        core.insert("created_at".to_owned(), "100".to_owned());

        let mut summary = HashMap::new();
        summary.insert("public_state".to_owned(), "completed".to_owned());

        let record = build_session_record(&sid, &core, &summary);
        assert_eq!(record.state, SessionState::Completed);
    }
}
