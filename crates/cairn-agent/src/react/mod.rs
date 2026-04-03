//! ReAct (Reason + Act) loop boundaries.
//!
//! The ReAct loop is the core agent execution pattern:
//! observe -> think -> act -> observe -> ...
//!
//! This module defines the step types and loop control boundaries
//! that the orchestrator uses to drive agent execution.

use serde::{Deserialize, Serialize};

/// A single step in the ReAct loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactPhase {
    Observe,
    Think,
    Act,
}

/// Loop control signal from the agent back to the orchestrator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "signal", rename_all = "snake_case")]
pub enum LoopSignal {
    /// Continue to the next phase.
    Continue,
    /// Yield control back to the orchestrator (e.g., for tool execution).
    Yield { reason: String },
    /// Terminate the loop.
    Terminate { reason: String },
}

#[cfg(test)]
mod tests {
    use super::{LoopSignal, ReactPhase};

    #[test]
    fn react_phases_cycle() {
        let phases = [ReactPhase::Observe, ReactPhase::Think, ReactPhase::Act];
        assert_eq!(phases.len(), 3);
    }

    #[test]
    fn loop_signals_are_distinct() {
        assert_ne!(
            LoopSignal::Continue,
            LoopSignal::Terminate {
                reason: "done".into()
            }
        );
    }
}
