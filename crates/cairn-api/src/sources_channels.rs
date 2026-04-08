use async_trait::async_trait;
use cairn_channels::adapters::ChannelAdapter;
use cairn_domain::ids::{ChannelId, SourceId};
use cairn_domain::tenancy::ProjectKey;
use cairn_signal::pollers::SignalSource;

use crate::endpoints::ListQuery;
use crate::http::ListResponse;

/// API error for source/channel operations.
#[derive(Debug)]
pub enum SourceChannelError {
    NotFound(String),
    InvalidInput(String),
    Internal(String),
}

/// Bulk action that can be applied to a set of signal sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BulkSourceAction {
    /// Re-trigger polling for degraded or stalled sources.
    Retry,
    /// Pause polling for the specified sources.
    Pause,
    /// Resume polling for previously paused sources.
    Resume,
}

/// Request body for `POST /v1/sources/bulk`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BulkSourceActionRequest {
    /// Source IDs the action should be applied to.
    pub source_ids: Vec<String>,
    /// Action to apply to all listed sources.
    pub action: BulkSourceAction,
}

/// Response for `POST /v1/sources/bulk`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BulkSourceActionResponse {
    /// Source IDs the action was applied to successfully.
    #[serde(default)]
    pub succeeded: Vec<String>,
    /// Source IDs that failed together with the per-source error message.
    /// Each entry is `(source_id, error_message)`.
    #[serde(default)]
    pub failed: Vec<(String, String)>,
}

impl BulkSourceActionResponse {
    /// Convenience: an empty response (no successes, no failures).
    pub fn empty() -> Self {
        Self {
            succeeded: vec![],
            failed: vec![],
        }
    }
}

/// API endpoint boundary for signal source management.
#[async_trait]
pub trait SourceEndpoints: Send + Sync {
    async fn list_sources(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<SignalSource>, SourceChannelError>;

    async fn get_source(
        &self,
        source_id: &SourceId,
    ) -> Result<Option<SignalSource>, SourceChannelError>;

    async fn trigger_poll(&self, source_id: &SourceId) -> Result<(), SourceChannelError>;

    /// Apply a bulk action (retry / pause / resume) to a set of sources.
    ///
    /// Default stub — returns an empty success response.
    /// Implementors should override to drive the actual source lifecycle.
    async fn bulk_action(
        &self,
        _project: &ProjectKey,
        _request: &BulkSourceActionRequest,
    ) -> Result<BulkSourceActionResponse, SourceChannelError> {
        Ok(BulkSourceActionResponse::empty())
    }
}

/// API endpoint boundary for channel management.
#[async_trait]
pub trait ChannelEndpoints: Send + Sync {
    async fn list_channels(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<ChannelAdapter>, SourceChannelError>;

    async fn get_channel(
        &self,
        channel_id: &ChannelId,
    ) -> Result<Option<ChannelAdapter>, SourceChannelError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::SourceId;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_signal::pollers::{SignalSource, SourceKind};

    #[test]
    fn source_channel_error_variants() {
        let err = SourceChannelError::NotFound("src_1".to_owned());
        assert!(matches!(err, SourceChannelError::NotFound(_)));
    }

    #[test]
    fn signal_source_used_in_api() {
        let source = SignalSource {
            source_id: SourceId::new("rss_1"),
            project: ProjectKey::new("t", "w", "p"),
            kind: SourceKind::Rss,
            name: "Tech News".to_owned(),
        };
        assert_eq!(source.kind, SourceKind::Rss);
    }
}
