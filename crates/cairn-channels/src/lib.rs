//! Channel delivery and notification routing boundaries.

pub mod adapters;
pub mod policies;
pub mod router;

pub use adapters::{ChannelAdapter, ChannelDelivery, ChannelKind, DeliveryMessage, DeliveryResult};
pub use policies::{DeliveryPolicy, DeliveryPolicyEvaluator, DeliveryRule};
pub use router::{Notification, NotificationRouter, RoutingResult};

#[cfg(test)]
mod tests {
    use cairn_domain::ids::ChannelId;
    use cairn_domain::tenancy::ProjectKey;

    use crate::adapters::{ChannelAdapter, ChannelKind};
    use crate::router::Notification;

    #[test]
    fn notification_targets_registered_channel() {
        let adapter = ChannelAdapter {
            channel_id: ChannelId::new("slack_eng"),
            project: ProjectKey::new("t", "w", "p"),
            kind: ChannelKind::Slack,
            name: "Eng Slack".to_owned(),
            enabled: true,
        };
        let notif = Notification {
            project: ProjectKey::new("t", "w", "p"),
            subject: "Alert".to_owned(),
            body: "Something happened".to_owned(),
            target_channels: vec![adapter.channel_id.clone()],
        };
        assert_eq!(notif.target_channels[0], adapter.channel_id);
    }
}
