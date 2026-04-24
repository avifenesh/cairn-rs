//! Adapter bridging [`cairn_store::projections::ToolCallApprovalReadModel`]
//! (store-side projection row) to the runtime-facing
//! [`crate::tool_call_approvals::ToolCallApprovalReader`] trait.
//!
//! PR-5 of the BP-v2 wave introduced the propose-then-await execute path;
//! that path needs a reader that can re-hydrate an approved proposal from
//! the persistent projection when the in-memory cache has been evicted
//! (restart, eviction, or cross-process resume). The three store backends
//! (Postgres / SQLite / InMemory) already implement the projection trait,
//! so this adapter is a zero-logic pass-through: fetch the record, pick
//! the effective args per the domain precedence invariant, return the
//! lean [`ApprovedProposal`] shape the runtime consumes.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::ToolCallId;
use cairn_store::projections::{ToolCallApprovalReadModel, ToolCallApprovalState};

use crate::error::RuntimeError;
use crate::tool_call_approvals::{ApprovedProposal, ToolCallApprovalReader};

/// Generic adapter over any store that implements
/// [`ToolCallApprovalReadModel`].
pub struct ToolCallApprovalReaderAdapter<T>
where
    T: ToolCallApprovalReadModel + Send + Sync + 'static,
{
    inner: Arc<T>,
}

impl<T> ToolCallApprovalReaderAdapter<T>
where
    T: ToolCallApprovalReadModel + Send + Sync + 'static,
{
    pub fn new(inner: Arc<T>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<T> ToolCallApprovalReader for ToolCallApprovalReaderAdapter<T>
where
    T: ToolCallApprovalReadModel + Send + Sync + 'static,
{
    async fn get_tool_call_approval(
        &self,
        call_id: &ToolCallId,
    ) -> Result<Option<ApprovedProposal>, RuntimeError> {
        let record = self
            .inner
            .get(call_id)
            .await
            .map_err(RuntimeError::Store)?;

        let Some(record) = record else {
            return Ok(None);
        };

        // Only surface Approved records — a Pending/Rejected/Timeout
        // record must not leak into the runtime's "approved" path.
        if record.state != ToolCallApprovalState::Approved {
            return Ok(None);
        }

        // Domain precedence invariant (see `cairn_domain::events::ToolCallApproved`):
        //   1. `approved_tool_args` if set on the approval event,
        //   2. else the most recent `ToolCallAmended.new_tool_args`,
        //   3. else the original `ToolCallProposed.tool_args`.
        let tool_args = record
            .approved_tool_args
            .clone()
            .or_else(|| record.amended_tool_args.clone())
            .unwrap_or_else(|| record.original_tool_args.clone());

        Ok(Some(ApprovedProposal {
            call_id: record.call_id.clone(),
            tool_name: record.tool_name.clone(),
            tool_args,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{ApprovalMatchPolicy, ProjectKey, RunId, SessionId};
    use cairn_store::projections::ToolCallApprovalRecord;
    use cairn_store::StoreError;
    use serde_json::json;
    use std::sync::Mutex;

    struct FakeRm {
        rec: Mutex<Option<ToolCallApprovalRecord>>,
    }

    #[async_trait]
    impl ToolCallApprovalReadModel for FakeRm {
        async fn get(
            &self,
            _call_id: &ToolCallId,
        ) -> Result<Option<ToolCallApprovalRecord>, StoreError> {
            Ok(self.rec.lock().unwrap().clone())
        }
        async fn list_for_run(
            &self,
            _run_id: &RunId,
        ) -> Result<Vec<ToolCallApprovalRecord>, StoreError> {
            Ok(vec![])
        }
        async fn list_for_session(
            &self,
            _session_id: &SessionId,
        ) -> Result<Vec<ToolCallApprovalRecord>, StoreError> {
            Ok(vec![])
        }
        async fn list_pending_for_project(
            &self,
            _project: &ProjectKey,
            _limit: usize,
            _offset: usize,
        ) -> Result<Vec<ToolCallApprovalRecord>, StoreError> {
            Ok(vec![])
        }
    }

    fn rec(state: ToolCallApprovalState) -> ToolCallApprovalRecord {
        ToolCallApprovalRecord {
            call_id: ToolCallId::new("tc"),
            session_id: SessionId::new("s"),
            run_id: RunId::new("r"),
            project: ProjectKey::new("t", "w", "p"),
            tool_name: "read".into(),
            original_tool_args: json!({"path": "/a"}),
            amended_tool_args: None,
            approved_tool_args: None,
            display_summary: None,
            match_policy: ApprovalMatchPolicy::Exact,
            state,
            operator_id: None,
            scope: None,
            reason: None,
            proposed_at_ms: 1,
            approved_at_ms: None,
            rejected_at_ms: None,
            last_amended_at_ms: None,
            version: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn pending_record_maps_to_none() {
        let rm = Arc::new(FakeRm {
            rec: Mutex::new(Some(rec(ToolCallApprovalState::Pending))),
        });
        let adapter = ToolCallApprovalReaderAdapter::new(rm);
        let got = adapter
            .get_tool_call_approval(&ToolCallId::new("tc"))
            .await
            .unwrap();
        assert!(got.is_none(), "pending must not leak as approved");
    }

    #[tokio::test]
    async fn approved_record_maps_with_original_args() {
        let rm = Arc::new(FakeRm {
            rec: Mutex::new(Some(rec(ToolCallApprovalState::Approved))),
        });
        let adapter = ToolCallApprovalReaderAdapter::new(rm);
        let got = adapter
            .get_tool_call_approval(&ToolCallId::new("tc"))
            .await
            .unwrap()
            .expect("approved record");
        assert_eq!(got.tool_name, "read");
        assert_eq!(got.tool_args, json!({"path": "/a"}));
    }

    #[tokio::test]
    async fn approved_record_prefers_approved_args_over_amended_and_original() {
        let mut r = rec(ToolCallApprovalState::Approved);
        r.amended_tool_args = Some(json!({"path": "/b"}));
        r.approved_tool_args = Some(json!({"path": "/c"}));
        let rm = Arc::new(FakeRm {
            rec: Mutex::new(Some(r)),
        });
        let adapter = ToolCallApprovalReaderAdapter::new(rm);
        let got = adapter
            .get_tool_call_approval(&ToolCallId::new("tc"))
            .await
            .unwrap()
            .expect("approved record");
        assert_eq!(got.tool_args, json!({"path": "/c"}));
    }

    #[tokio::test]
    async fn approved_record_falls_back_to_amended_when_no_approved_args() {
        let mut r = rec(ToolCallApprovalState::Approved);
        r.amended_tool_args = Some(json!({"path": "/b"}));
        let rm = Arc::new(FakeRm {
            rec: Mutex::new(Some(r)),
        });
        let adapter = ToolCallApprovalReaderAdapter::new(rm);
        let got = adapter
            .get_tool_call_approval(&ToolCallId::new("tc"))
            .await
            .unwrap()
            .expect("approved record");
        assert_eq!(got.tool_args, json!({"path": "/b"}));
    }
}
