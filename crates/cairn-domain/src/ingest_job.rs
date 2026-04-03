use crate::ids::{IngestJobId, SourceId};
use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Lifecycle state for a memory ingest job (current_state_plus_audit durability).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestJobState {
    Pending,
    Processing,
    Completed,
    Failed,
}

/// Durable current-state record for memory ingest jobs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestJobRecord {
    pub id: IngestJobId,
    pub project: ProjectKey,
    pub source_id: Option<SourceId>,
    pub document_count: u32,
    pub state: IngestJobState,
    pub error_message: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_job_record_round_trips() {
        let record = IngestJobRecord {
            id: IngestJobId::new("job_1"),
            project: ProjectKey::new("t", "w", "p"),
            source_id: Some(SourceId::new("src_1")),
            document_count: 5,
            state: IngestJobState::Pending,
            error_message: None,
            created_at: 100,
            updated_at: 100,
        };

        assert_eq!(record.id.as_str(), "job_1");
        assert_eq!(record.state, IngestJobState::Pending);
    }
}
