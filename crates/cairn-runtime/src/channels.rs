use async_trait::async_trait;
use cairn_domain::{ChannelId, ChannelMessage, ChannelRecord, ProjectKey};

use crate::error::RuntimeError;

#[async_trait]
pub trait ChannelService: Send + Sync {
    async fn create(
        &self,
        project: &ProjectKey,
        name: String,
        capacity: u32,
    ) -> Result<ChannelRecord, RuntimeError>;

    async fn get(&self, channel_id: &ChannelId) -> Result<Option<ChannelRecord>, RuntimeError>;

    async fn list_channels(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ChannelRecord>, RuntimeError>;

    async fn send(
        &self,
        channel_id: &ChannelId,
        sender_id: String,
        body: String,
    ) -> Result<String, RuntimeError>;

    async fn consume(
        &self,
        channel_id: &ChannelId,
        consumer_id: String,
    ) -> Result<Option<ChannelMessage>, RuntimeError>;

    async fn list_messages(
        &self,
        channel_id: &ChannelId,
        limit: usize,
    ) -> Result<Vec<ChannelMessage>, RuntimeError>;
}
