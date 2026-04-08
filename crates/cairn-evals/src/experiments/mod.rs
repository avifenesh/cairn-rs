//! Online experiment engine backed by multi-armed bandit strategies (GAP-013).
//!
//! Wraps `cairn_domain::bandit::BanditStrategy` with an eval-layer experiment
//! model that uses simple win/trial counters instead of continuous rewards.
//! Intended for prompt A/B testing: arms map to prompt releases or model variants.
//!
//! # Strategies
//! - **EpsilonGreedy** — with probability `epsilon` pick a random arm (explore);
//!   otherwise pick the arm with the highest win rate (exploit).
//! - **UCB1** — deterministic; pick the arm that maximises
//!   `win_rate + sqrt(2 * ln(total_trials) / arm_trials)`.
//!   Untried arms always win (score = `f64::MAX`).
//!
//! # Usage
//! ```rust,ignore
//! let mut engine = ExperimentEngine::new();
//! let id = engine.create_experiment(
//!     "prompt-test".to_owned(),
//!     vec!["arm-a".to_owned(), "arm-b".to_owned()],
//!     BanditStrategy::EpsilonGreedy { epsilon: 0.1 },
//! );
//! let arm = engine.select_arm(&id).unwrap();
//! engine.record_win(&id, &arm);
//! ```

use std::collections::HashMap;

use cairn_domain::bandit::BanditStrategy;
use serde::{Deserialize, Serialize};

// ── ExperimentArm ─────────────────────────────────────────────────────────

/// One arm in an experiment, mapped to a prompt release or model variant.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperimentArm {
    /// Stable identifier for this arm within the experiment.
    pub arm_id: String,
    /// Human-readable label (e.g. `"prompt-v2-claude-sonnet"`).
    pub label: String,
    /// Number of positive outcomes recorded for this arm.
    pub wins: u32,
    /// Total number of times this arm was evaluated.
    pub trials: u32,
}

impl ExperimentArm {
    fn win_rate(&self) -> f64 {
        if self.trials == 0 {
            0.0
        } else {
            self.wins as f64 / self.trials as f64
        }
    }

    fn ucb1_score(&self, total_trials: u32) -> f64 {
        if self.trials == 0 || total_trials == 0 {
            return f64::MAX;
        }
        let rate = self.wins as f64 / self.trials as f64;
        let confidence = (2.0 * (total_trials as f64).ln() / self.trials as f64).sqrt();
        rate + confidence
    }
}

// ── BanditExperiment ──────────────────────────────────────────────────────

/// An online experiment driven by a bandit selection strategy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BanditExperiment {
    pub experiment_id: String,
    pub name: String,
    pub arms: Vec<ExperimentArm>,
    pub strategy: BanditStrategy,
    /// Exploration probability for `EpsilonGreedy` (ignored for UCB1).
    pub epsilon: f64,
    pub created_at_ms: u64,
    /// Whether the experiment is actively routing traffic.
    pub active: bool,
}

impl BanditExperiment {
    /// Select an arm according to the configured strategy.
    ///
    /// - `EpsilonGreedy`: with probability `epsilon` returns a random arm;
    ///   otherwise returns the arm with the highest win rate.
    /// - `UCB1`: returns the arm with the highest UCB1 score (untried arms
    ///   always win).
    ///
    /// Returns `None` only when the experiment has no arms.
    pub fn select_arm(&self) -> Option<&ExperimentArm> {
        if self.arms.is_empty() {
            return None;
        }
        match &self.strategy {
            BanditStrategy::EpsilonGreedy { epsilon } => {
                if rand_unit() < *epsilon {
                    // Explore: uniform random arm.
                    let n = self.arms.len();
                    let idx = (rand_u64() as usize) % n;
                    self.arms.get(idx)
                } else {
                    // Exploit: highest win rate.
                    self.arms.iter().max_by(|a, b| {
                        a.win_rate()
                            .partial_cmp(&b.win_rate())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                }
            }
            BanditStrategy::Ucb1 => {
                let total: u32 = self.arms.iter().map(|a| a.trials).sum();
                self.arms.iter().max_by(|a, b| {
                    a.ucb1_score(total)
                        .partial_cmp(&b.ucb1_score(total))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            }
        }
    }

    /// Record an outcome for an arm.
    ///
    /// `win = true` increments both `wins` and `trials`;
    /// `win = false` increments only `trials`.
    pub fn record_outcome(&mut self, arm_id: &str, win: bool) {
        if let Some(arm) = self.arms.iter_mut().find(|a| a.arm_id == arm_id) {
            arm.trials += 1;
            if win {
                arm.wins += 1;
            }
        }
    }

    /// Return the win rate for each arm as `(arm_id, rate)` sorted by `arm_id`.
    ///
    /// Arms with no trials have a rate of `0.0`.
    pub fn win_rates(&self) -> Vec<(String, f64)> {
        let mut rates: Vec<_> = self
            .arms
            .iter()
            .map(|a| (a.arm_id.clone(), a.win_rate()))
            .collect();
        rates.sort_by(|a, b| a.0.cmp(&b.0));
        rates
    }
}

// ── ExperimentStore ───────────────────────────────────────────────────────

/// In-memory store for experiments, keyed by `experiment_id`.
#[derive(Default)]
pub struct ExperimentStore {
    experiments: HashMap<String, BanditExperiment>,
}

impl ExperimentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, exp: BanditExperiment) {
        self.experiments.insert(exp.experiment_id.clone(), exp);
    }

    pub fn get(&self, id: &str) -> Option<&BanditExperiment> {
        self.experiments.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut BanditExperiment> {
        self.experiments.get_mut(id)
    }

    /// List all experiments with `active == true`.
    pub fn list_active(&self) -> Vec<&BanditExperiment> {
        let mut active: Vec<_> = self.experiments.values().filter(|e| e.active).collect();
        active.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
        active
    }

    pub fn len(&self) -> usize {
        self.experiments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.experiments.is_empty()
    }
}

// ── ExperimentEngine ──────────────────────────────────────────────────────

/// Service facade for experiment lifecycle management.
pub struct ExperimentEngine {
    store: ExperimentStore,
    next_id: u64,
}

impl ExperimentEngine {
    pub fn new() -> Self {
        Self {
            store: ExperimentStore::new(),
            next_id: 0,
        }
    }

    /// Create a new active experiment.
    ///
    /// Each element of `arms` becomes both the `arm_id` and `label`.
    /// Returns the new `experiment_id`.
    pub fn create_experiment(
        &mut self,
        name: String,
        arms: Vec<String>,
        strategy: BanditStrategy,
    ) -> String {
        self.next_id += 1;
        let experiment_id = format!("exp_{}", self.next_id);
        let epsilon = match &strategy {
            BanditStrategy::EpsilonGreedy { epsilon } => *epsilon,
            BanditStrategy::Ucb1 => 0.0,
        };
        let experiment_arms = arms
            .into_iter()
            .map(|label| ExperimentArm {
                arm_id: label.clone(),
                label,
                wins: 0,
                trials: 0,
            })
            .collect();
        let exp = BanditExperiment {
            experiment_id: experiment_id.clone(),
            name,
            arms: experiment_arms,
            strategy,
            epsilon,
            created_at_ms: now_ms(),
            active: true,
        };
        self.store.insert(exp);
        experiment_id
    }

    /// Select an arm for the given experiment, returning its `arm_id`.
    pub fn select_arm(&self, experiment_id: &str) -> Option<String> {
        self.store
            .get(experiment_id)
            .and_then(|exp| exp.select_arm())
            .map(|arm| arm.arm_id.clone())
    }

    /// Record a win for `arm_id` in `experiment_id`.
    pub fn record_win(&mut self, experiment_id: &str, arm_id: &str) {
        if let Some(exp) = self.store.get_mut(experiment_id) {
            exp.record_outcome(arm_id, true);
        }
    }

    /// Record a loss (non-win trial) for `arm_id` in `experiment_id`.
    pub fn record_loss(&mut self, experiment_id: &str, arm_id: &str) {
        if let Some(exp) = self.store.get_mut(experiment_id) {
            exp.record_outcome(arm_id, false);
        }
    }

    /// Return win rates for all arms in the experiment, or `None` if not found.
    pub fn experiment_stats(&self, experiment_id: &str) -> Option<Vec<(String, f64)>> {
        self.store.get(experiment_id).map(|exp| exp.win_rates())
    }
}

impl Default for ExperimentEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Minimal PRNG (no external crate required) ─────────────────────────────
//
// XorShift64 seeded from nanosecond wall time. Used exclusively for the
// epsilon-greedy exploration branch. Good enough for A/B routing; not
// cryptographic.

fn rand_u64() -> u64 {
    let seed = now_ns().wrapping_add(0x9e3779b97f4a7c15);
    xorshift64(seed)
}

fn rand_unit() -> f64 {
    (rand_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn eg(epsilon: f64) -> BanditStrategy {
        BanditStrategy::EpsilonGreedy { epsilon }
    }

    // 1. create_experiment_adds_arms
    #[test]
    fn create_experiment_adds_arms() {
        let mut engine = ExperimentEngine::new();
        let id = engine.create_experiment(
            "test".to_owned(),
            vec!["arm-a".to_owned(), "arm-b".to_owned(), "arm-c".to_owned()],
            eg(0.1),
        );
        let exp = engine.store.get(&id).unwrap();
        assert_eq!(exp.arms.len(), 3);
        assert_eq!(exp.arms[0].arm_id, "arm-a");
        assert_eq!(exp.arms[1].arm_id, "arm-b");
        assert_eq!(exp.arms[2].arm_id, "arm-c");
        assert!(exp.active);
    }

    // 2. epsilon_greedy_selects_best_arm_after_training
    //
    // With epsilon=0.0 (pure exploit), the best arm must always be selected.
    // We also verify the probabilistic case: epsilon=0.1 → best arm selected
    // significantly more than 70% of 200 trials.
    #[test]
    fn epsilon_greedy_selects_best_arm_after_training() {
        let mut exp = BanditExperiment {
            experiment_id: "e1".to_owned(),
            name: "test".to_owned(),
            arms: vec![
                ExperimentArm {
                    arm_id: "good".to_owned(),
                    label: "good".to_owned(),
                    wins: 100,
                    trials: 100,
                },
                ExperimentArm {
                    arm_id: "bad".to_owned(),
                    label: "bad".to_owned(),
                    wins: 10,
                    trials: 100,
                },
            ],
            strategy: eg(0.0), // pure exploit
            epsilon: 0.0,
            created_at_ms: 0,
            active: true,
        };

        // Pure exploit → always selects "good".
        for _ in 0..50 {
            let arm = exp.select_arm().unwrap();
            assert_eq!(
                arm.arm_id, "good",
                "pure-exploit must always pick the best arm"
            );
        }

        // With epsilon=0.1, best arm should still dominate.
        exp.strategy = eg(0.1);
        exp.epsilon = 0.1;
        let good_count = (0..200)
            .filter(|_| exp.select_arm().unwrap().arm_id == "good")
            .count();
        assert!(
            good_count > 140,
            "epsilon=0.1 → best arm expected >70% of time; got {}/200",
            good_count
        );
    }

    // 3. ucb1_selects_unexplored_arm_first
    #[test]
    fn ucb1_selects_unexplored_arm_first() {
        let exp = BanditExperiment {
            experiment_id: "e2".to_owned(),
            name: "ucb".to_owned(),
            arms: vec![
                ExperimentArm {
                    arm_id: "explored".to_owned(),
                    label: "e".to_owned(),
                    wins: 50,
                    trials: 100,
                },
                ExperimentArm {
                    arm_id: "unexplored".to_owned(),
                    label: "u".to_owned(),
                    wins: 0,
                    trials: 0,
                },
            ],
            strategy: BanditStrategy::Ucb1,
            epsilon: 0.0,
            created_at_ms: 0,
            active: true,
        };
        let arm = exp.select_arm().unwrap();
        assert_eq!(
            arm.arm_id, "unexplored",
            "UCB1 must select untried arm first"
        );
    }

    // 4. record_outcome_updates_win_rates
    #[test]
    fn record_outcome_updates_win_rates() {
        let mut exp = BanditExperiment {
            experiment_id: "e3".to_owned(),
            name: "rec".to_owned(),
            arms: vec![ExperimentArm {
                arm_id: "x".to_owned(),
                label: "x".to_owned(),
                wins: 0,
                trials: 0,
            }],
            strategy: eg(0.0),
            epsilon: 0.0,
            created_at_ms: 0,
            active: true,
        };
        exp.record_outcome("x", true);
        exp.record_outcome("x", true);
        exp.record_outcome("x", false);

        let arm = exp.arms.iter().find(|a| a.arm_id == "x").unwrap();
        assert_eq!(arm.trials, 3);
        assert_eq!(arm.wins, 2);
        assert!((arm.win_rate() - 2.0 / 3.0).abs() < 1e-9);
    }

    // 5. list_active_returns_only_active_experiments
    #[test]
    fn list_active_returns_only_active_experiments() {
        let mut engine = ExperimentEngine::new();
        let id1 = engine.create_experiment("active".to_owned(), vec!["a".to_owned()], eg(0.1));
        let id2 = engine.create_experiment("inactive".to_owned(), vec!["b".to_owned()], eg(0.1));
        // Deactivate experiment 2.
        engine.store.get_mut(&id2).unwrap().active = false;

        let active = engine.store.list_active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].experiment_id, id1);
    }

    // 6. win_rates_zero_for_untrialed_arms
    #[test]
    fn win_rates_zero_for_untrialed_arms() {
        let mut engine = ExperimentEngine::new();
        let id = engine.create_experiment(
            "fresh".to_owned(),
            vec!["arm1".to_owned(), "arm2".to_owned()],
            eg(0.0),
        );
        let rates = engine.experiment_stats(&id).unwrap();
        for (_, rate) in &rates {
            assert_eq!(*rate, 0.0, "untrialed arms must have 0.0 win rate");
        }
    }

    // 7. experiment_engine_create_and_select
    #[test]
    fn experiment_engine_create_and_select() {
        let mut engine = ExperimentEngine::new();
        let id = engine.create_experiment(
            "e7".to_owned(),
            vec!["alpha".to_owned(), "beta".to_owned()],
            eg(0.0),
        );
        // Seed "beta" as dominant.
        let exp = engine.store.get_mut(&id).unwrap();
        exp.arms[1].wins = 80;
        exp.arms[1].trials = 100;

        let selected = engine.select_arm(&id).unwrap();
        assert_eq!(
            selected, "beta",
            "pure exploit must pick the highest win-rate arm"
        );
    }

    // 8. experiment_engine_record_and_stats
    #[test]
    fn experiment_engine_record_and_stats() {
        let mut engine = ExperimentEngine::new();
        let id = engine.create_experiment(
            "e8".to_owned(),
            vec!["p".to_owned(), "q".to_owned()],
            eg(0.0),
        );
        engine.record_win(&id, "p");
        engine.record_win(&id, "p");
        engine.record_loss(&id, "q");

        let stats = engine.experiment_stats(&id).unwrap();
        let p_rate = stats.iter().find(|(k, _)| k == "p").unwrap().1;
        let q_rate = stats.iter().find(|(k, _)| k == "q").unwrap().1;
        assert!((p_rate - 1.0).abs() < 1e-9, "p should have 100% win rate");
        assert_eq!(q_rate, 0.0, "q should have 0% win rate after 1 loss");
    }

    // 9. ucb1_balances_exploration_vs_exploitation
    //
    // With one very good arm (90 wins / 100 trials) and one fresh arm,
    // after the fresh arm gets its first trial UCB1 should eventually
    // pick the high-reward arm more often when it has more data.
    #[test]
    fn ucb1_balances_exploration_vs_exploitation() {
        let mut engine = ExperimentEngine::new();
        let id = engine.create_experiment(
            "ucb-balance".to_owned(),
            vec!["strong".to_owned(), "weak".to_owned()],
            BanditStrategy::Ucb1,
        );

        // Prime the strong arm.
        for _ in 0..90 {
            engine.record_win(&id, "strong");
        }
        for _ in 0..10 {
            engine.record_loss(&id, "strong");
        }

        // Give "weak" enough trials to shrink its UCB1 confidence interval.
        // With only 3 trials, weak's CI is huge (sqrt(2*ln(103)/3) ≈ 1.76)
        // and would beat strong's score. After 40 losses, it shrinks enough.
        for _ in 0..40 {
            engine.record_loss(&id, "weak");
        }

        // Now strong (90% win rate, 100 trials) should dominate over
        // weak (0% win rate, 43 trials): UCB1_strong ≈ 1.21 > UCB1_weak ≈ 0.52.
        let strong_count = (0..20)
            .filter(|_| engine.select_arm(&id).unwrap() == "strong")
            .count();
        assert!(
            strong_count >= 15,
            "UCB1 should favour strong arm after sufficient exploration; got {}/20",
            strong_count
        );
    }

    // 10. epsilon_zero_always_selects_best
    #[test]
    fn epsilon_zero_always_selects_best() {
        let mut engine = ExperimentEngine::new();
        let id = engine.create_experiment(
            "pure-exploit".to_owned(),
            vec![
                "mediocre".to_owned(),
                "champion".to_owned(),
                "poor".to_owned(),
            ],
            eg(0.0),
        );

        let exp = engine.store.get_mut(&id).unwrap();
        exp.arms[0].wins = 5;
        exp.arms[0].trials = 10; // 50%
        exp.arms[1].wins = 9;
        exp.arms[1].trials = 10; // 90%
        exp.arms[2].wins = 1;
        exp.arms[2].trials = 10; // 10%

        // With epsilon=0, every selection must return "champion".
        for _ in 0..100 {
            let selected = engine.select_arm(&id).unwrap();
            assert_eq!(
                selected, "champion",
                "epsilon=0 must always exploit the best arm"
            );
        }
    }

    // Bonus: select_arm returns None for empty arms
    #[test]
    fn select_arm_returns_none_for_empty_experiment() {
        let exp = BanditExperiment {
            experiment_id: "empty".to_owned(),
            name: "n".to_owned(),
            arms: vec![],
            strategy: eg(0.1),
            epsilon: 0.1,
            created_at_ms: 0,
            active: true,
        };
        assert!(exp.select_arm().is_none());
    }

    // Bonus: store len and is_empty
    #[test]
    fn experiment_store_len_and_is_empty() {
        let mut store = ExperimentStore::new();
        assert!(store.is_empty());
        store.insert(BanditExperiment {
            experiment_id: "x".to_owned(),
            name: "x".to_owned(),
            arms: vec![],
            strategy: BanditStrategy::Ucb1,
            epsilon: 0.0,
            created_at_ms: 0,
            active: true,
        });
        assert_eq!(store.len(), 1);
    }
}
