use crate::{ChannelId, ProjectKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRecord {
    pub channel_id: ChannelId,
    pub project: ProjectKey,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub channel_id: ChannelId,
    pub message_id: String,
    pub sender_id: String,
    pub body: String,
    pub sent_at_ms: u64,
    pub consumed_by: Option<String>,
    pub consumed_at_ms: Option<u64>,
}
