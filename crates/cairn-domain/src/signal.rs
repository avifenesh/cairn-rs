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
