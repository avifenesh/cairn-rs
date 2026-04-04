use crate::error::StoreError;
use async_trait::async_trait;
use cairn_domain::{ChannelId, ChannelMessage, ChannelRecord, ProjectKey};

#[async_trait]
pub trait ChannelReadModel: Send + Sync {
    async fn get_channel(
        &self,
        channel_id: &ChannelId,
    ) -> Result<Option<ChannelRecord>, StoreError>;

    async fn list_channels(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ChannelRecord>, StoreError>;

    async fn list_messages(
        &self,
        channel_id: &ChannelId,
        limit: usize,
    ) -> Result<Vec<ChannelMessage>, StoreError>;
}
