use std::collections::BTreeMap;
use std::sync::Arc;

use cairn_domain::lifecycle::SessionState;
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::SessionId;
use cairn_store::projections::SessionRecord;
use flowfabric::core::types::{FlowId, Namespace, TimestampMs};

use crate::boot::FabricRuntime;
use crate::engine::ControlPlaneBackend;
use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::try_parse_project_key;
use crate::id_map;

pub struct FabricSessionService {
    /// Retained for API parity with the other fabric services. Not
    /// read directly from this service anymore — all FF-state reads
    /// go through `engine` and all FCALL-dispatch goes through
    /// `control_plane`.
    #[allow(dead_code)]
    runtime: Arc<FabricRuntime>,
    bridge: Arc<EventBridge>,
    engine: Arc<dyn crate::engine::Engine>,
    control_plane: Arc<dyn ControlPlaneBackend>,
}

impl FabricSessionService {
    pub fn new(
        runtime: Arc<FabricRuntime>,
        bridge: Arc<EventBridge>,
        engine: Arc<dyn crate::engine::Engine>,
        control_plane: Arc<dyn ControlPlaneBackend>,
    ) -> Self {
        Self {
            runtime,
            bridge,
            engine,
            control_plane,
        }
    }

    fn flow_id(&self, project: &ProjectKey, session_id: &SessionId) -> FlowId {
        id_map::session_to_flow_id(project, session_id)
    }

    fn namespace(&self, project: &ProjectKey) -> Namespace {
        id_map::tenant_to_namespace(&project.tenant_id)
    }

    async fn read_session_record(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, FabricError> {
        let fid = self.flow_id(project, session_id);
        match self.engine.describe_flow(&fid).await? {
            None => Ok(None),
            Some(snapshot) => Ok(Some(build_session_record(session_id, project, &snapshot))),
        }
    }

    pub async fn create(
        &self,
        project: &ProjectKey,
        session_id: SessionId,
    ) -> Result<SessionRecord, FabricError> {
        let fid = self.flow_id(project, &session_id);
        let namespace = self.namespace(project);
        let now = TimestampMs::now();

        let project_str = format!(
            "{}/{}/{}",
            project.tenant_id, project.workspace_id, project.project_id
        );

        self.control_plane
            .create_flow(crate::engine::CreateFlowInput {
                flow_id: fid.clone(),
                flow_kind: "cairn_session".to_owned(),
                namespace,
            })
            .await?;

        // Stamp the cairn-scoped flow tags in one round-trip via the
        // engine abstraction. Replaces two sequential direct HSETs
        // on `fctx.core()` — same observable semantics (both tags
        // written before the bridge event fires), one fewer Valkey
        // round-trip, and no direct Valkey layout knowledge in the
        // service layer.
        let mut cairn_tags: BTreeMap<String, String> = BTreeMap::new();
        cairn_tags.insert("cairn.project".to_owned(), project_str);
        cairn_tags.insert(
            "cairn.session_id".to_owned(),
            session_id.as_str().to_owned(),
        );
        self.engine.set_flow_tags(&fid, &cairn_tags).await?;

        // Emit the bridge event unconditionally so the cairn-store
        // `SessionReadModel` projection gets a record even when FF
        // returns `ok_already_satisfied` for the underlying
        // `ff_create_flow` call. The previous `!is_already_satisfied`
        // guard was retry-unsafe in the same shape as the one removed
        // from `FabricRunService::suspend` (see run_service.rs L823-L832):
        // if the cairn process crashed between the FCALL committing
        // and the bridge emit, the next boot's retry would hit
        // `ok_already_satisfied` and skip the emit permanently — the
        // projection would have a permanent gap and every downstream
        // handler that resolves project-from-session-id (e.g.
        // `FabricSessionServiceAdapter::get` via
        // `resolve_project_from_session_id`) would 404 forever. That
        // failure mode is exactly what surfaced the RFC 020
        // `recovery_summary_emitted_once_per_boot` test #11 regression
        // (task #178): POST /v1/runs looked up the just-created session
        // via the read model, saw `None`, and returned 404 "session not
        // found". The projection is idempotent on `EventId`, so a
        // duplicate emit on a real replay is a harmless re-write —
        // strictly safer than dropping the event. Mirrors the
        // `ExecutionCreated` pattern in `FabricRunService::start`.
        self.bridge
            .emit(BridgeEvent::SessionCreated {
                session_id: session_id.clone(),
                project: project.clone(),
            })
            .await;

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

        // Check the flow exists before issuing the FF-side cancel.
        // Uses the typed snapshot rather than a raw HGETALL — same
        // semantics, just goes through the engine abstraction.
        if self.engine.describe_flow(&fid).await?.is_none() {
            return Err(FabricError::NotFound {
                entity: "session",
                id: session_id.to_string(),
            });
        }

        // `AlreadyTerminal` is acceptable — the flow may already be
        // completed/cancelled, but cairn still needs to mark it
        // archived. The trait collapses that branch into a typed
        // variant so services never see the FF envelope text.
        let _outcome = self
            .control_plane
            .cancel_flow(crate::engine::CancelFlowInput {
                flow_id: fid.clone(),
                reason: "session archived".to_owned(),
                cancel_mode: "cancel_all".to_owned(),
            })
            .await?;

        // Route the archive-flag write through the engine
        // abstraction instead of reaching into `fctx.core()`
        // directly. Same observable semantics — `cairn.archived` is
        // written on the flow-core hash before the
        // `SessionArchived` bridge event fires.
        self.engine
            .set_flow_tag(&fid, "cairn.archived", "true")
            .await?;

        match self.read_session_record(project, session_id).await? {
            Some(record) => {
                self.bridge
                    .emit(BridgeEvent::SessionArchived {
                        session_id: session_id.clone(),
                        project: record.project.clone(),
                    })
                    .await;
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
    caller_project: &ProjectKey,
    snapshot: &crate::engine::FlowSnapshot,
) -> SessionRecord {
    let project = snapshot
        .tags
        .get("cairn.project")
        .and_then(|s| try_parse_project_key(s))
        .unwrap_or_else(|| caller_project.clone());

    let is_archived = snapshot
        .tags
        .get("cairn.archived")
        .map(|v| v == "true")
        .unwrap_or(false);
    let state = if is_archived {
        SessionState::Archived
    } else {
        flow_state_to_session_state(&snapshot.public_flow_state)
    };

    SessionRecord {
        session_id: session_id.clone(),
        project,
        state,
        // graph_revision is monotonic across the flow's lifetime;
        // cairn uses it as the SessionRecord optimistic-concurrency
        // version. Matches `SessionService::create`'s `version: 0`
        // on fresh flows — a read right after create sees the same
        // value the creator saw, so optimistic-concurrency checks
        // don't misfire.
        version: snapshot.graph_revision,
        created_at: snapshot.created_at.0 as u64,
        updated_at: snapshot.last_mutation_at.0 as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

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

    /// Test helper: build a FlowSnapshot with common fields for
    /// build_session_record regression tests. Keeps tests readable
    /// without reconstructing the full struct every time.
    fn fake_flow_snapshot(
        project_tag: Option<&str>,
        public_flow_state: &str,
        graph_revision: u64,
        created_at: i64,
        last_mutation_at: i64,
        archived: bool,
    ) -> crate::engine::FlowSnapshot {
        use std::collections::BTreeMap;
        let mut tags = BTreeMap::new();
        if let Some(p) = project_tag {
            tags.insert("cairn.project".to_owned(), p.to_owned());
        }
        if archived {
            tags.insert("cairn.archived".to_owned(), "true".to_owned());
        }
        crate::engine::FlowSnapshot {
            flow_id: FlowId::from_uuid(uuid::Uuid::nil()),
            kind: "cairn_session".to_owned(),
            namespace: Namespace::new("default"),
            node_count: 0,
            edge_count: 0,
            graph_revision,
            public_flow_state: public_flow_state.to_owned(),
            created_at: TimestampMs::from_millis(created_at),
            last_mutation_at: TimestampMs::from_millis(last_mutation_at),
            tags,
        }
    }

    #[test]
    fn build_record_completed_flow() {
        let sid = SessionId::new("sess_test");
        let snap = fake_flow_snapshot(Some("t/w/p"), "completed", 3, 1000, 2000, false);
        let record = build_session_record(&sid, &test_project(), &snap);
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
        let snap = fake_flow_snapshot(Some("t/w/p"), "running", 0, 500, 500, false);
        let record = build_session_record(&sid, &test_project(), &snap);
        assert_eq!(record.state, SessionState::Open);
    }

    #[test]
    fn build_record_defaults_when_empty() {
        let sid = SessionId::new("sess_empty");
        let snap = fake_flow_snapshot(None, "", 0, 0, 0, false);
        let record = build_session_record(&sid, &test_project(), &snap);
        assert_eq!(record.state, SessionState::Open);
        assert_eq!(record.project.tenant_id.as_str(), "t");
        // Fresh flow has graph_revision=0; match SessionService::create.
        assert_eq!(record.version, 0);
        assert_eq!(record.created_at, 0);
    }

    #[test]
    fn build_record_updated_at_from_snapshot() {
        let sid = SessionId::new("sess_no_update");
        let snap = fake_flow_snapshot(Some("t/w/p"), "", 0, 999, 999, false);
        let record = build_session_record(&sid, &test_project(), &snap);
        assert_eq!(record.updated_at, 999);
    }

    #[test]
    fn build_record_failed_flow() {
        let sid = SessionId::new("sess_fail");
        let snap = fake_flow_snapshot(Some("x/y/z"), "failed", 0, 100, 100, false);
        let record = build_session_record(&sid, &test_project(), &snap);
        assert_eq!(record.state, SessionState::Failed);
    }

    #[test]
    fn build_record_cancelled_maps_to_failed() {
        let sid = SessionId::new("sess_cancel");
        let snap = fake_flow_snapshot(Some("t/w/p"), "cancelled", 0, 100, 100, false);
        let record = build_session_record(&sid, &test_project(), &snap);
        assert_eq!(record.state, SessionState::Failed);
    }

    #[test]
    fn build_record_archived_overrides_flow_state() {
        let sid = SessionId::new("sess_archived");
        let snap = fake_flow_snapshot(Some("t/w/p"), "cancelled", 0, 100, 100, true);
        let record = build_session_record(&sid, &test_project(), &snap);
        assert_eq!(record.state, SessionState::Archived);
    }

    #[test]
    fn build_record_not_archived_when_flag_absent() {
        let sid = SessionId::new("sess_not_archived");
        let snap = fake_flow_snapshot(Some("t/w/p"), "completed", 0, 100, 100, false);
        let record = build_session_record(&sid, &test_project(), &snap);
        assert_eq!(record.state, SessionState::Completed);
    }
}
