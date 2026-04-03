use serde::{Deserialize, Serialize};

/// Shared runtime selector inputs used for prompt and provider resolution.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectorContext {
    pub agent_type: Option<String>,
    pub task_type: Option<String>,
    pub routing_slot: Option<String>,
}

impl SelectorContext {
    pub fn project_default() -> Self {
        Self::default()
    }

    pub fn with_agent_type(mut self, agent_type: impl Into<String>) -> Self {
        self.agent_type = Some(agent_type.into());
        self
    }

    pub fn with_task_type(mut self, task_type: impl Into<String>) -> Self {
        self.task_type = Some(task_type.into());
        self
    }

    pub fn with_routing_slot(mut self, routing_slot: impl Into<String>) -> Self {
        self.routing_slot = Some(routing_slot.into());
        self
    }
}

/// Structured rollout target from RFC 006.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RolloutTarget {
    ProjectDefault,
    AgentType { agent_type: String },
    TaskType { task_type: String },
    RoutingSlot { slot: String },
}

impl RolloutTarget {
    pub fn precedence(&self) -> u8 {
        match self {
            RolloutTarget::RoutingSlot { .. } => 4,
            RolloutTarget::TaskType { .. } => 3,
            RolloutTarget::AgentType { .. } => 2,
            RolloutTarget::ProjectDefault => 1,
        }
    }

    pub fn matches(&self, context: &SelectorContext) -> bool {
        match self {
            RolloutTarget::ProjectDefault => true,
            RolloutTarget::AgentType { agent_type } => {
                context.agent_type.as_deref() == Some(agent_type.as_str())
            }
            RolloutTarget::TaskType { task_type } => {
                context.task_type.as_deref() == Some(task_type.as_str())
            }
            RolloutTarget::RoutingSlot { slot } => {
                context.routing_slot.as_deref() == Some(slot.as_str())
            }
        }
    }
}

pub fn best_matching_target<'a>(
    targets: &'a [RolloutTarget],
    context: &SelectorContext,
) -> Option<&'a RolloutTarget> {
    targets
        .iter()
        .filter(|target| target.matches(context))
        .max_by_key(|target| target.precedence())
}

#[cfg(test)]
mod tests {
    use super::{best_matching_target, RolloutTarget, SelectorContext};

    #[test]
    fn routing_slot_is_most_specific_match() {
        let context = SelectorContext::project_default()
            .with_agent_type("planner")
            .with_task_type("review")
            .with_routing_slot("fallback_1");
        let targets = vec![
            RolloutTarget::ProjectDefault,
            RolloutTarget::AgentType {
                agent_type: "planner".to_owned(),
            },
            RolloutTarget::TaskType {
                task_type: "review".to_owned(),
            },
            RolloutTarget::RoutingSlot {
                slot: "fallback_1".to_owned(),
            },
        ];

        let matched = best_matching_target(&targets, &context);

        assert_eq!(
            matched,
            Some(&RolloutTarget::RoutingSlot {
                slot: "fallback_1".to_owned(),
            })
        );
    }

    #[test]
    fn project_default_always_matches() {
        let target = RolloutTarget::ProjectDefault;

        assert!(target.matches(&SelectorContext::default()));
    }
}
