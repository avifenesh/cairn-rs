//! Reflection and runtime advisory boundaries.
//!
//! Reflection provides the agent with self-inspection capabilities:
//! reviewing past actions, evaluating progress, and deciding whether
//! to adjust strategy or escalate.

use serde::{Deserialize, Serialize};

/// Advisory signal from the reflection layer to the orchestrator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "advisory", rename_all = "snake_case")]
pub enum ReflectionAdvisory {
    /// Execution is progressing normally.
    OnTrack,
    /// Agent is making slow progress; consider strategy change.
    SlowProgress { iterations_elapsed: u32 },
    /// Agent appears stuck in a loop.
    LoopDetected { pattern: String },
    /// Agent should escalate to a human or supervisor.
    Escalate { reason: String },
}

#[cfg(test)]
mod tests {
    use super::ReflectionAdvisory;

    #[test]
    fn advisories_are_distinct() {
        assert_ne!(
            ReflectionAdvisory::OnTrack,
            ReflectionAdvisory::Escalate {
                reason: "stuck".into()
            }
        );
    }
}
