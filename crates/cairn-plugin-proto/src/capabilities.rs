use serde::{Deserialize, Serialize};

/// Canonical plugin capability family names per RFC 007.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFamily {
    ToolProvider,
    SignalSource,
    ChannelProvider,
    PostTurnHook,
    PolicyHook,
    EvalScorer,
}

impl CapabilityFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityFamily::ToolProvider => "tool_provider",
            CapabilityFamily::SignalSource => "signal_source",
            CapabilityFamily::ChannelProvider => "channel_provider",
            CapabilityFamily::PostTurnHook => "post_turn_hook",
            CapabilityFamily::PolicyHook => "policy_hook",
            CapabilityFamily::EvalScorer => "eval_scorer",
        }
    }
}

/// Canonical plugin invocation outcome statuses per RFC 007.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationStatus {
    Success,
    RetryableFailure,
    PermanentFailure,
    Timeout,
    Canceled,
    ProtocolViolation,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_family_str_matches_rfc() {
        assert_eq!(CapabilityFamily::ToolProvider.as_str(), "tool_provider");
        assert_eq!(CapabilityFamily::EvalScorer.as_str(), "eval_scorer");
    }

    #[test]
    fn invocation_status_serde() {
        let json = serde_json::to_string(&InvocationStatus::Success).unwrap();
        assert_eq!(json, "\"success\"");
    }
}
