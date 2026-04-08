use crate::ids::SignalId;
use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Durable current-state record for signal events (current_state_plus_audit durability).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalRecord {
    pub id: SignalId,
    pub project: ProjectKey,
    pub source: String,
    pub payload: serde_json::Value,
    pub timestamp_ms: u64,
}

/// A subscription to a signal type for routing notifications.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignalSubscription {
    pub subscription_id: String,
    pub project: crate::tenancy::ProjectKey,
    pub signal_kind: String,
    pub target_run_id: Option<crate::ids::RunId>,
    pub target_mailbox_id: Option<String>,
    pub filter_expression: Option<String>,
    pub created_at_ms: u64,
    /// Legacy: alias for signal_kind (for backward compat).
    #[serde(default)]
    pub signal_type: String,
    /// Legacy: generic target string (for backward compat).
    #[serde(default)]
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::SignalRecord;
    use crate::ids::SignalId;
    use crate::tenancy::ProjectKey;

    #[test]
    fn signal_record_carries_source_and_payload() {
        let record = SignalRecord {
            id: SignalId::new("signal_1"),
            project: ProjectKey::new("t", "w", "p"),
            source: "webhook".to_owned(),
            payload: serde_json::json!({"key": "value"}),
            timestamp_ms: 100,
        };

        assert_eq!(record.id.as_str(), "signal_1");
        assert_eq!(record.source, "webhook");
        assert_eq!(record.payload["key"], "value");
    }
}
