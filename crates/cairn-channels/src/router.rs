use cairn_domain::ids::ChannelId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// A notification to be routed to one or more channels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub project: ProjectKey,
    pub subject: String,
    pub body: String,
    pub target_channels: Vec<ChannelId>,
}

/// Result of routing a notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingResult {
    pub dispatched: Vec<ChannelId>,
    pub skipped: Vec<ChannelId>,
}

/// Seam for notification routing. Implementors decide which channels
/// receive a given notification based on project config and policies.
pub trait NotificationRouter {
    type Error;

    fn route(&self, notification: &Notification) -> Result<RoutingResult, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::ChannelId;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn notification_construction() {
        let notif = Notification {
            project: ProjectKey::new("t", "w", "p"),
            subject: "Build failed".to_owned(),
            body: "CI pipeline #42 failed".to_owned(),
            target_channels: vec![ChannelId::new("slack_eng")],
        };
        assert_eq!(notif.target_channels.len(), 1);
    }

    #[test]
    fn routing_result_tracks_dispatched_and_skipped() {
        let result = RoutingResult {
            dispatched: vec![ChannelId::new("slack_eng")],
            skipped: vec![ChannelId::new("email_disabled")],
        };
        assert_eq!(result.dispatched.len(), 1);
        assert_eq!(result.skipped.len(), 1);
    }
}
