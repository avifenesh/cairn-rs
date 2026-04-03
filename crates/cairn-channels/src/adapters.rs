use cairn_domain::ids::ChannelId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Channel kinds supported in v1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Slack,
    Email,
    Telegram,
    Webhook,
    Plugin,
}

/// A registered outbound channel adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelAdapter {
    pub channel_id: ChannelId,
    pub project: ProjectKey,
    pub kind: ChannelKind,
    pub name: String,
    pub enabled: bool,
}

/// Message to deliver through a channel adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryMessage {
    pub channel_id: ChannelId,
    pub subject: String,
    pub body: String,
    pub recipients: Vec<String>,
}

/// Result of delivering a message through a channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryResult {
    pub delivery_ids: Vec<String>,
}

/// Seam for outbound channel delivery. Implementors send messages
/// through a specific channel backend (Slack, email, etc.).
pub trait ChannelDelivery {
    type Error;

    fn deliver(&self, message: &DeliveryMessage) -> Result<DeliveryResult, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::ChannelId;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn channel_adapter_construction() {
        let adapter = ChannelAdapter {
            channel_id: ChannelId::new("slack_eng"),
            project: ProjectKey::new("t", "w", "p"),
            kind: ChannelKind::Slack,
            name: "Engineering Slack".to_owned(),
            enabled: true,
        };
        assert_eq!(adapter.kind, ChannelKind::Slack);
        assert!(adapter.enabled);
    }

    #[test]
    fn delivery_result_carries_ids() {
        let result = DeliveryResult {
            delivery_ids: vec!["del_1".to_owned(), "del_2".to_owned()],
        };
        assert_eq!(result.delivery_ids.len(), 2);
    }
}
