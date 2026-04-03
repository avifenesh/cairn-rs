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
