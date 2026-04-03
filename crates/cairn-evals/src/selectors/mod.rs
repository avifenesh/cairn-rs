//! Selector and resolution types for prompt release targeting per RFC 006.
//!
//! Selector precedence (most specific wins):
//! 1. routing_slot
//! 2. task_type
//! 3. agent_type
//! 4. project_default

use serde::{Deserialize, Serialize};

/// Selector kind per RFC 006.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorKind {
    /// Lowest precedence.
    ProjectDefault = 0,
    AgentType = 1,
    TaskType = 2,
    /// Highest precedence.
    RoutingSlot = 3,
}

/// Structured rollout target for a prompt release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutTarget {
    pub kind: SelectorKind,
    pub selector: SelectorValue,
}

/// Selector value depending on the kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SelectorValue {
    ProjectDefault,
    AgentType { agent_type: String },
    TaskType { task_type: String },
    RoutingSlot { slot: String },
}

/// Runtime context used for prompt resolution.
///
/// The runtime provides this context; the resolver matches it against
/// active releases in selector-precedence order.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResolutionContext {
    pub agent_type: Option<String>,
    pub task_type: Option<String>,
    pub routing_slot: Option<String>,
}

impl RolloutTarget {
    pub fn project_default() -> Self {
        Self {
            kind: SelectorKind::ProjectDefault,
            selector: SelectorValue::ProjectDefault,
        }
    }

    pub fn agent_type(agent_type: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::AgentType,
            selector: SelectorValue::AgentType {
                agent_type: agent_type.into(),
            },
        }
    }

    pub fn task_type(task_type: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::TaskType,
            selector: SelectorValue::TaskType {
                task_type: task_type.into(),
            },
        }
    }

    pub fn routing_slot(slot: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::RoutingSlot,
            selector: SelectorValue::RoutingSlot { slot: slot.into() },
        }
    }

    /// Check if this target matches the given resolution context.
    pub fn matches(&self, ctx: &ResolutionContext) -> bool {
        match &self.selector {
            SelectorValue::ProjectDefault => true,
            SelectorValue::AgentType { agent_type } => {
                ctx.agent_type.as_deref() == Some(agent_type.as_str())
            }
            SelectorValue::TaskType { task_type } => {
                ctx.task_type.as_deref() == Some(task_type.as_str())
            }
            SelectorValue::RoutingSlot { slot } => {
                ctx.routing_slot.as_deref() == Some(slot.as_str())
            }
        }
    }
}

impl SelectorKind {
    /// Higher precedence kinds have higher numeric values.
    pub fn precedence(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::{ResolutionContext, RolloutTarget, SelectorKind};

    #[test]
    fn selector_precedence_order() {
        assert!(SelectorKind::RoutingSlot.precedence() > SelectorKind::TaskType.precedence());
        assert!(SelectorKind::TaskType.precedence() > SelectorKind::AgentType.precedence());
        assert!(SelectorKind::AgentType.precedence() > SelectorKind::ProjectDefault.precedence());
    }

    #[test]
    fn project_default_matches_any_context() {
        let target = RolloutTarget::project_default();
        let ctx = ResolutionContext {
            agent_type: Some("planner".to_owned()),
            task_type: None,
            routing_slot: None,
        };
        assert!(target.matches(&ctx));
    }

    #[test]
    fn agent_type_matches_specific_context() {
        let target = RolloutTarget::agent_type("planner");
        let matching = ResolutionContext {
            agent_type: Some("planner".to_owned()),
            ..Default::default()
        };
        let non_matching = ResolutionContext {
            agent_type: Some("coder".to_owned()),
            ..Default::default()
        };
        assert!(target.matches(&matching));
        assert!(!target.matches(&non_matching));
    }

    #[test]
    fn routing_slot_matches_specific_context() {
        let target = RolloutTarget::routing_slot("fallback_1");
        let matching = ResolutionContext {
            routing_slot: Some("fallback_1".to_owned()),
            ..Default::default()
        };
        assert!(target.matches(&matching));
    }
}
