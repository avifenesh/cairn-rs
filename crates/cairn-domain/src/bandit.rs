//! Bandit experimentation domain types (GAP-013).
//!
//! Multi-armed bandit for prompt A/B testing. Each arm is bound to a
//! `PromptReleaseId` so the bandit steers live traffic toward the
//! best-performing prompt release.
//!
//! Two selection strategies are supported:
//! - **EpsilonGreedy**: with probability `epsilon` picks a random arm
//!   (exploration), otherwise picks the arm with the highest empirical
//!   mean reward (exploitation).
//! - **UCB1**: picks the arm that maximises the Upper Confidence Bound,
//!   balancing exploitation with exploration via a confidence bonus.

use serde::{Deserialize, Serialize};

use crate::ids::{PromptReleaseId, TenantId};

/// Selection strategy for a bandit experiment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BanditStrategy {
    /// ε-greedy: explore uniformly at random with probability `epsilon`.
    EpsilonGreedy { epsilon: f64 },
    /// UCB1: deterministic; picks arm with highest upper confidence bound.
    Ucb1,
}

/// One arm in a bandit experiment, bound to a specific prompt release.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanditArm {
    /// Stable identifier for this arm within the experiment.
    pub arm_id: String,
    /// The prompt release that this arm evaluates.
    pub prompt_release_id: PromptReleaseId,
    /// Initial weight / prior weight (used as tie-breaker, default 1.0).
    pub weight: f64,
    /// Total number of times this arm was selected.
    pub pulls: u64,
    /// Cumulative reward received across all pulls.
    pub reward_sum: f64,
}

impl BanditArm {
    pub fn new(arm_id: impl Into<String>, prompt_release_id: PromptReleaseId) -> Self {
        Self {
            arm_id: arm_id.into(),
            prompt_release_id,
            weight: 1.0,
            pulls: 0,
            reward_sum: 0.0,
        }
    }

    /// Empirical mean reward (0.5 default when unpulled — optimistic initialisation).
    pub fn mean_reward(&self) -> f64 {
        if self.pulls == 0 {
            0.5
        } else {
            self.reward_sum / self.pulls as f64
        }
    }

    /// UCB1 score given total pulls across the entire experiment.
    ///
    /// `total_pulls` — sum of all arm pulls in the experiment.
    /// Returns `f64::MAX` for unpulled arms so they are selected first.
    pub fn ucb1_score(&self, total_pulls: u64) -> f64 {
        if self.pulls == 0 || total_pulls == 0 {
            return f64::MAX;
        }
        let mean = self.reward_sum / self.pulls as f64;
        let confidence = (2.0 * (total_pulls as f64).ln() / self.pulls as f64).sqrt();
        mean + confidence
    }
}

/// A bandit experiment binding multiple prompt releases for live A/B testing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanditExperiment {
    pub experiment_id: String,
    pub tenant_id: TenantId,
    pub arms: Vec<BanditArm>,
    pub strategy: BanditStrategy,
    /// For EpsilonGreedy: exploration probability (0.0–1.0).
    /// Ignored for UCB1.
    pub epsilon: f64,
    pub created_at_ms: u64,
}

impl BanditExperiment {
    /// Total pulls across all arms in this experiment.
    pub fn total_pulls(&self) -> u64 {
        self.arms.iter().map(|a| a.pulls).sum()
    }

    /// Get a mutable reference to an arm by ID.
    pub fn arm_mut(&mut self, arm_id: &str) -> Option<&mut BanditArm> {
        self.arms.iter_mut().find(|a| a.arm_id == arm_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(id: &str) -> PromptReleaseId {
        PromptReleaseId::new(id)
    }

    #[test]
    fn bandit_arm_mean_reward_unpulled_is_half() {
        let arm = BanditArm::new("a", release("pr_1"));
        assert_eq!(arm.mean_reward(), 0.5);
    }

    #[test]
    fn bandit_arm_mean_reward_after_pulls() {
        let mut arm = BanditArm::new("a", release("pr_1"));
        arm.pulls = 4;
        arm.reward_sum = 3.0;
        assert_eq!(arm.mean_reward(), 0.75);
    }

    #[test]
    fn bandit_arm_ucb1_unpulled_is_max() {
        let arm = BanditArm::new("a", release("pr_1"));
        assert_eq!(arm.ucb1_score(10), f64::MAX);
    }

    #[test]
    fn bandit_experiment_total_pulls() {
        let exp = BanditExperiment {
            experiment_id: "e1".to_owned(),
            tenant_id: TenantId::new("t1"),
            arms: vec![
                {
                    let mut a = BanditArm::new("a", release("pr_1"));
                    a.pulls = 5;
                    a
                },
                {
                    let mut a = BanditArm::new("b", release("pr_2"));
                    a.pulls = 3;
                    a
                },
            ],
            strategy: BanditStrategy::EpsilonGreedy { epsilon: 0.1 },
            epsilon: 0.1,
            created_at_ms: 0,
        };
        assert_eq!(exp.total_pulls(), 8);
    }

    #[test]
    fn bandit_strategy_serde() {
        let s = BanditStrategy::EpsilonGreedy { epsilon: 0.15 };
        let json = serde_json::to_string(&s).unwrap();
        let back: BanditStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);

        let ucb = BanditStrategy::Ucb1;
        let j2 = serde_json::to_string(&ucb).unwrap();
        let b2: BanditStrategy = serde_json::from_str(&j2).unwrap();
        assert_eq!(b2, BanditStrategy::Ucb1);
    }
}
