//! Ingest job service boundary per RFC 003.
//!
//! Memory ingest jobs are runtime-owned async jobs that report through
//! runtime command/event surfaces.

use async_trait::async_trait;
use cairn_domain::{IngestJobId, IngestJobRecord, ProjectKey, SourceId};

use crate::error::RuntimeError;

/// Ingest job service boundary.
///
/// Per RFC 003, heavier ingest work executes asynchronously as
/// runtime-owned jobs, reporting through command/event surfaces.
#[async_trait]
pub trait IngestJobService: Send + Sync {
    /// Start a new ingest job.
    async fn start(
        &self,
        project: &ProjectKey,
        job_id: IngestJobId,
        source_id: Option<SourceId>,
        document_count: u32,
    ) -> Result<IngestJobRecord, RuntimeError>;

    /// Mark an ingest job as completed (success or failure).
    async fn complete(
        &self,
        project: &ProjectKey,
        job_id: IngestJobId,
        success: bool,
        error_message: Option<String>,
    ) -> Result<IngestJobRecord, RuntimeError>;

    /// Get an ingest job by ID.
    async fn get(&self, job_id: &IngestJobId) -> Result<Option<IngestJobRecord>, RuntimeError>;

    /// List ingest jobs for a project.
    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<IngestJobRecord>, RuntimeError>;
}
