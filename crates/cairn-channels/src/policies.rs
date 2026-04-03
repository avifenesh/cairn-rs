use cairn_domain::ids::ChannelId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Delivery policy controlling when and how a channel delivers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPolicy {
    pub project: ProjectKey,
    pub channel_id: ChannelId,
    pub rules: Vec<DeliveryRule>,
}

/// Individual delivery rule within a policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule_type", rename_all = "snake_case")]
pub enum DeliveryRule {
    RateLimit { max_per_hour: u32 },
    QuietHours { start_hour: u8, end_hour: u8 },
    RequirePriority { min_priority: String },
}

/// Seam for delivery policy evaluation. Implementors decide whether
/// a delivery should proceed given current policies.
pub trait DeliveryPolicyEvaluator {
    type Error;

    fn evaluate(&self, policy: &DeliveryPolicy) -> Result<bool, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::ChannelId;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn delivery_policy_with_rate_limit() {
        let policy = DeliveryPolicy {
            project: ProjectKey::new("t", "w", "p"),
            channel_id: ChannelId::new("slack_eng"),
            rules: vec![DeliveryRule::RateLimit { max_per_hour: 10 }],
        };
        assert_eq!(policy.rules.len(), 1);
    }

    #[test]
    fn delivery_policy_with_multiple_rules() {
        let policy = DeliveryPolicy {
            project: ProjectKey::new("t", "w", "p"),
            channel_id: ChannelId::new("email_alerts"),
            rules: vec![
                DeliveryRule::RateLimit { max_per_hour: 5 },
                DeliveryRule::QuietHours {
                    start_hour: 22,
                    end_hour: 7,
                },
            ],
        };
        assert_eq!(policy.rules.len(), 2);
    }
}
