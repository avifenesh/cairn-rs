//! Bandit experimentation service (GAP-013).
//!
//! In-memory multi-armed bandit for prompt A/B testing. Supports two
//! selection strategies:
//! - `EpsilonGreedy`: explore with probability `epsilon`, exploit otherwise.
//! - `UCB1`: Upper Confidence Bound; deterministic, no randomness needed.
//!
//! `BanditServiceImpl` is `Send + Sync` via `Arc<Mutex<HashMap>>`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cairn_domain::bandit::{BanditArm, BanditExperiment, BanditStrategy};
use cairn_domain::ids::{PromptReleaseId, TenantId};

use crate::error::RuntimeError;

// ── Selectors ────────────────────────────────────────────────────────────

/// Select the best arm from a non-empty slice using epsilon-greedy.
///
/// With probability `epsilon` returns a uniformly random arm index (exploration).
/// Otherwise returns the arm with the highest `mean_reward()` (exploitation).
/// Ties broken by first-occurrence order.
pub fn epsilon_greedy_select(arms: &[BanditArm], epsilon: f64, rng: f64) -> &BanditArm {
    debug_assert!(!arms.is_empty());
    if rng < epsilon {
        // Exploration: uniformly random.
        // Map rng (which is in [0, epsilon)) into [0, 1) and scale to arms.
        let idx = ((rng / epsilon) * arms.len() as f64) as usize;
        &arms[idx.min(arms.len() - 1)]
    } else {
        // Exploitation: highest mean reward.
        arms.iter()
            .max_by(|a, b| {
                a.mean_reward()
                    .partial_cmp(&b.mean_reward())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("arms non-empty")
    }
}

/// Select the best arm using UCB1 (deterministic).
pub fn ucb1_select(arms: &[BanditArm], total_pulls: u64) -> &BanditArm {
    debug_assert!(!arms.is_empty());
    arms.iter()
        .max_by(|a, b| {
            a.ucb1_score(total_pulls)
                .partial_cmp(&b.ucb1_score(total_pulls))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("arms non-empty")
}

// ── Service boundary ──────────────────────────────────────────────────────

/// Service error for bandit operations.
#[derive(Debug)]
pub enum BanditError {
    /// No experiment with this ID.
    ExperimentNotFound(String),
    /// No arm with this ID in the experiment.
    ArmNotFound { experiment_id: String, arm_id: String },
    /// Experiment has no arms.
    NoArms(String),
    /// Invalid configuration (e.g. epsilon out of range).
    InvalidConfig(String),
}

impl std::fmt::Display for BanditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BanditError::ExperimentNotFound(id) => write!(f, "bandit experiment '{id}' not found"),
            BanditError::ArmNotFound { experiment_id, arm_id } => {
                write!(f, "arm '{arm_id}' not found in experiment '{experiment_id}'")
            }
            BanditError::NoArms(id) => write!(f, "experiment '{id}' has no arms"),
            BanditError::InvalidConfig(msg) => write!(f, "invalid bandit config: {msg}"),
        }
    }
}

impl std::error::Error for BanditError {}

impl From<BanditError> for RuntimeError {
    fn from(e: BanditError) -> Self {
        RuntimeError::Internal(e.to_string())
    }
}

// ── Request / response types ──────────────────────────────────────────────

/// Request to create a new bandit experiment.
pub struct CreateExperimentRequest {
    pub experiment_id: String,
    pub tenant_id: TenantId,
    /// Arms to register, each bound to a prompt release.
    pub arms: Vec<(String, PromptReleaseId)>,
    pub strategy: BanditStrategy,
    /// Epsilon for EpsilonGreedy (ignored for UCB1). Must be in [0, 1].
    pub epsilon: f64,
    pub created_at_ms: u64,
}

/// Result of a `select_arm` call.
#[derive(Clone, Debug)]
pub struct SelectedArm {
    pub arm_id: String,
    pub prompt_release_id: PromptReleaseId,
}

// ── Implementation ────────────────────────────────────────────────────────

/// In-memory bandit service. All experiments are keyed by `experiment_id`.
#[derive(Clone)]
pub struct BanditServiceImpl {
    experiments: Arc<Mutex<HashMap<String, BanditExperiment>>>,
    /// Optional RNG override for deterministic tests (None = use real randomness).
    rng_override: Option<f64>,
}

impl BanditServiceImpl {
    pub fn new() -> Self {
        Self {
            experiments: Arc::new(Mutex::new(HashMap::new())),
            rng_override: None,
        }
    }

    /// Override the RNG value used for epsilon-greedy selection.
    /// Useful in tests to force deterministic explore/exploit behaviour.
    /// Value must be in [0, 1).
    pub fn with_fixed_rng(mut self, rng: f64) -> Self {
        self.rng_override = Some(rng);
        self
    }

    fn rng(&self) -> f64 {
        self.rng_override.unwrap_or_else(|| {
            // Poor man's entropy from timestamp bits — good enough for non-crypto use.
            // In production you'd use `rand` crate.
            let ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            (ns as f64) / 1_000_000_000.0
        })
    }

    /// Create a new experiment. Returns `Err` if the ID already exists or the
    /// config is invalid.
    pub fn create_experiment(
        &self,
        req: CreateExperimentRequest,
    ) -> Result<BanditExperiment, BanditError> {
        if req.arms.is_empty() {
            return Err(BanditError::NoArms(req.experiment_id));
        }
        if req.epsilon < 0.0 || req.epsilon > 1.0 {
            return Err(BanditError::InvalidConfig(format!(
                "epsilon must be in [0, 1], got {}",
                req.epsilon
            )));
        }
        let mut lock = self.experiments.lock().unwrap_or_else(|e| e.into_inner());
        if lock.contains_key(&req.experiment_id) {
            return Err(BanditError::InvalidConfig(format!(
                "experiment '{}' already exists",
                req.experiment_id
            )));
        }
        let arms = req
            .arms
            .into_iter()
            .map(|(arm_id, release_id)| BanditArm::new(arm_id, release_id))
            .collect();
        let exp = BanditExperiment {
            experiment_id: req.experiment_id.clone(),
            tenant_id: req.tenant_id,
            arms,
            strategy: req.strategy,
            epsilon: req.epsilon,
            created_at_ms: req.created_at_ms,
        };
        lock.insert(req.experiment_id, exp.clone());
        Ok(exp)
    }

    /// Select an arm for the given experiment using the configured strategy.
    ///
    /// The arm's `pulls` counter is **not** incremented here — the caller
    /// should call `record_reward` after observing the outcome, which
    /// increments pulls and updates `reward_sum`.
    pub fn select_arm(&self, experiment_id: &str) -> Result<SelectedArm, BanditError> {
        let lock = self.experiments.lock().unwrap_or_else(|e| e.into_inner());
        let exp = lock
            .get(experiment_id)
            .ok_or_else(|| BanditError::ExperimentNotFound(experiment_id.to_owned()))?;

        if exp.arms.is_empty() {
            return Err(BanditError::NoArms(experiment_id.to_owned()));
        }

        let arm = match &exp.strategy {
            BanditStrategy::EpsilonGreedy { epsilon } => {
                let rng = self.rng();
                epsilon_greedy_select(&exp.arms, *epsilon, rng)
            }
            BanditStrategy::Ucb1 => {
                let total = exp.total_pulls();
                ucb1_select(&exp.arms, total)
            }
        };

        Ok(SelectedArm {
            arm_id: arm.arm_id.clone(),
            prompt_release_id: arm.prompt_release_id.clone(),
        })
    }

    /// Record the reward observed after pulling `arm_id` in `experiment_id`.
    ///
    /// Increments `pulls` and adds `reward` to `reward_sum`.
    /// `reward` is typically in [0.0, 1.0] but the service accepts any f64.
    pub fn record_reward(
        &self,
        experiment_id: &str,
        arm_id: &str,
        reward: f64,
    ) -> Result<(), BanditError> {
        let mut lock = self.experiments.lock().unwrap_or_else(|e| e.into_inner());
        let exp = lock
            .get_mut(experiment_id)
            .ok_or_else(|| BanditError::ExperimentNotFound(experiment_id.to_owned()))?;
        let arm = exp.arm_mut(arm_id).ok_or_else(|| BanditError::ArmNotFound {
            experiment_id: experiment_id.to_owned(),
            arm_id: arm_id.to_owned(),
        })?;
        arm.pulls += 1;
        arm.reward_sum += reward;
        Ok(())
    }

    /// Get a snapshot of the experiment (cloned).
    pub fn get_experiment(&self, experiment_id: &str) -> Option<BanditExperiment> {
        self.experiments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(experiment_id)
            .cloned()
    }

    /// List all experiments for a tenant (cloned snapshots).
    pub fn list_by_tenant(&self, tenant_id: &TenantId) -> Vec<BanditExperiment> {
        self.experiments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|e| e.tenant_id == *tenant_id)
            .cloned()
            .collect()
    }
}

impl Default for BanditServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::bandit::BanditStrategy;
    use cairn_domain::ids::{PromptReleaseId, TenantId};

    fn release(id: &str) -> PromptReleaseId {
        PromptReleaseId::new(id)
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant_1")
    }

    fn two_arm_experiment(svc: &BanditServiceImpl, epsilon: f64) -> BanditExperiment {
        svc.create_experiment(CreateExperimentRequest {
            experiment_id: "exp_1".to_owned(),
            tenant_id: tenant(),
            arms: vec![
                ("arm_a".to_owned(), release("pr_a")),
                ("arm_b".to_owned(), release("pr_b")),
            ],
            strategy: BanditStrategy::EpsilonGreedy { epsilon },
            epsilon,
            created_at_ms: 1000,
        })
        .unwrap()
    }

    // ── Creation ─────────────────────────────────────────────────────────

    #[test]
    fn bandit_create_experiment_ok() {
        let svc = BanditServiceImpl::new();
        let exp = two_arm_experiment(&svc, 0.1);
        assert_eq!(exp.experiment_id, "exp_1");
        assert_eq!(exp.arms.len(), 2);
        assert_eq!(exp.epsilon, 0.1);
    }

    #[test]
    fn bandit_create_experiment_no_arms_is_error() {
        let svc = BanditServiceImpl::new();
        let result = svc.create_experiment(CreateExperimentRequest {
            experiment_id: "e".to_owned(),
            tenant_id: tenant(),
            arms: vec![],
            strategy: BanditStrategy::EpsilonGreedy { epsilon: 0.1 },
            epsilon: 0.1,
            created_at_ms: 0,
        });
        assert!(matches!(result, Err(BanditError::NoArms(_))));
    }

    #[test]
    fn bandit_create_experiment_duplicate_id_is_error() {
        let svc = BanditServiceImpl::new();
        two_arm_experiment(&svc, 0.1);
        let result = svc.create_experiment(CreateExperimentRequest {
            experiment_id: "exp_1".to_owned(),
            tenant_id: tenant(),
            arms: vec![("a".to_owned(), release("pr_a"))],
            strategy: BanditStrategy::EpsilonGreedy { epsilon: 0.1 },
            epsilon: 0.1,
            created_at_ms: 0,
        });
        assert!(matches!(result, Err(BanditError::InvalidConfig(_))));
    }

    #[test]
    fn bandit_create_experiment_epsilon_out_of_range() {
        let svc = BanditServiceImpl::new();
        let result = svc.create_experiment(CreateExperimentRequest {
            experiment_id: "e2".to_owned(),
            tenant_id: tenant(),
            arms: vec![("a".to_owned(), release("pr_a"))],
            strategy: BanditStrategy::EpsilonGreedy { epsilon: 1.5 },
            epsilon: 1.5,
            created_at_ms: 0,
        });
        assert!(matches!(result, Err(BanditError::InvalidConfig(_))));
    }

    // ── Reward recording ──────────────────────────────────────────────────

    #[test]
    fn bandit_record_reward_increments_pulls_and_sum() {
        let svc = BanditServiceImpl::new();
        two_arm_experiment(&svc, 0.1);
        svc.record_reward("exp_1", "arm_a", 1.0).unwrap();
        svc.record_reward("exp_1", "arm_a", 0.5).unwrap();
        let exp = svc.get_experiment("exp_1").unwrap();
        let arm_a = exp.arms.iter().find(|a| a.arm_id == "arm_a").unwrap();
        assert_eq!(arm_a.pulls, 2);
        assert_eq!(arm_a.reward_sum, 1.5);
    }

    #[test]
    fn bandit_record_reward_unknown_arm_is_error() {
        let svc = BanditServiceImpl::new();
        two_arm_experiment(&svc, 0.1);
        let result = svc.record_reward("exp_1", "arm_ghost", 1.0);
        assert!(matches!(result, Err(BanditError::ArmNotFound { .. })));
    }

    #[test]
    fn bandit_record_reward_unknown_experiment_is_error() {
        let svc = BanditServiceImpl::new();
        let result = svc.record_reward("no_such_exp", "arm_a", 1.0);
        assert!(matches!(result, Err(BanditError::ExperimentNotFound(_))));
    }

    // ── EpsilonGreedy selection ───────────────────────────────────────────

    /// Core test: epsilon=0 → always exploit → arm_a (higher reward) always selected.
    #[test]
    fn bandit_epsilon_zero_always_exploits_best_arm() {
        let svc = BanditServiceImpl::new().with_fixed_rng(0.99); // > epsilon=0 → always exploit
        two_arm_experiment(&svc, 0.0);

        // Give arm_a many reward=1.0 and arm_b reward=0.0.
        for _ in 0..20 {
            svc.record_reward("exp_1", "arm_a", 1.0).unwrap();
        }
        for _ in 0..20 {
            svc.record_reward("exp_1", "arm_b", 0.0).unwrap();
        }

        // With epsilon=0, exploitation always picks arm_a.
        for _ in 0..10 {
            let selected = svc.select_arm("exp_1").unwrap();
            assert_eq!(
                selected.arm_id, "arm_a",
                "epsilon=0 must always pick the highest-reward arm"
            );
            assert_eq!(selected.prompt_release_id, release("pr_a"));
        }
    }

    /// Forced exploration: rng < epsilon → picks random arm.
    #[test]
    fn bandit_epsilon_one_always_explores() {
        // rng = 0.001, epsilon = 1.0 → rng < epsilon → always random
        // With arms sorted as [arm_a, arm_b], idx = floor(0.001/1.0 * 2) = 0 → arm_a
        let svc = BanditServiceImpl::new().with_fixed_rng(0.001);
        two_arm_experiment(&svc, 1.0);
        // Even if arm_b has better history, with rng near 0 we always pick first arm.
        svc.record_reward("exp_1", "arm_b", 1.0).unwrap();
        svc.record_reward("exp_1", "arm_b", 1.0).unwrap();
        let selected = svc.select_arm("exp_1").unwrap();
        // rng_scaled = 0.001/1.0 = 0.001 → idx = 0 → arm_a
        assert_eq!(selected.arm_id, "arm_a", "forced explore with rng near 0 selects index 0");
    }

    // ── UCB1 selection ────────────────────────────────────────────────────

    #[test]
    fn bandit_ucb1_unpulled_arm_selected_first() {
        let svc = BanditServiceImpl::new();
        svc.create_experiment(CreateExperimentRequest {
            experiment_id: "ucb_exp".to_owned(),
            tenant_id: tenant(),
            arms: vec![
                ("arm_x".to_owned(), release("pr_x")),
                ("arm_y".to_owned(), release("pr_y")),
            ],
            strategy: BanditStrategy::Ucb1,
            epsilon: 0.0,
            created_at_ms: 0,
        })
        .unwrap();

        // Pull arm_x once to give it a history.
        svc.record_reward("ucb_exp", "arm_x", 0.9).unwrap();

        // arm_y has 0 pulls → UCB1 score = MAX → must be selected.
        let selected = svc.select_arm("ucb_exp").unwrap();
        assert_eq!(
            selected.arm_id, "arm_y",
            "UCB1 must prefer unpulled arm"
        );
    }

    #[test]
    fn bandit_ucb1_selects_better_arm_with_data() {
        let svc = BanditServiceImpl::new();
        svc.create_experiment(CreateExperimentRequest {
            experiment_id: "ucb2".to_owned(),
            tenant_id: tenant(),
            arms: vec![
                ("best".to_owned(), release("pr_best")),
                ("worst".to_owned(), release("pr_worst")),
            ],
            strategy: BanditStrategy::Ucb1,
            epsilon: 0.0,
            created_at_ms: 0,
        })
        .unwrap();

        // Give both arms many pulls so confidence bounds tighten.
        for _ in 0..50 {
            svc.record_reward("ucb2", "best", 1.0).unwrap();
        }
        for _ in 0..50 {
            svc.record_reward("ucb2", "worst", 0.0).unwrap();
        }

        let selected = svc.select_arm("ucb2").unwrap();
        assert_eq!(selected.arm_id, "best", "UCB1 must prefer high-reward arm with many pulls");
    }

    // ── List / get ────────────────────────────────────────────────────────

    #[test]
    fn bandit_get_experiment_returns_snapshot() {
        let svc = BanditServiceImpl::new();
        two_arm_experiment(&svc, 0.1);
        let snap = svc.get_experiment("exp_1").unwrap();
        assert_eq!(snap.arms.len(), 2);
    }

    #[test]
    fn bandit_list_by_tenant_filters_correctly() {
        let svc = BanditServiceImpl::new();
        two_arm_experiment(&svc, 0.1);
        svc.create_experiment(CreateExperimentRequest {
            experiment_id: "other_exp".to_owned(),
            tenant_id: TenantId::new("other_tenant"),
            arms: vec![("a".to_owned(), release("pr_a"))],
            strategy: BanditStrategy::Ucb1,
            epsilon: 0.0,
            created_at_ms: 0,
        })
        .unwrap();

        let t1_exps = svc.list_by_tenant(&tenant());
        assert_eq!(t1_exps.len(), 1);
        assert_eq!(t1_exps[0].experiment_id, "exp_1");
    }

    // ── selector unit tests ───────────────────────────────────────────────

    #[test]
    fn epsilon_greedy_selector_exploit_picks_best_mean() {
        let arms = vec![
            { let mut a = BanditArm::new("low", release("pr_1")); a.pulls = 10; a.reward_sum = 2.0; a },
            { let mut a = BanditArm::new("high", release("pr_2")); a.pulls = 10; a.reward_sum = 8.0; a },
        ];
        // rng = 0.5, epsilon = 0.1 → 0.5 >= 0.1 → exploit → picks "high"
        let selected = epsilon_greedy_select(&arms, 0.1, 0.5);
        assert_eq!(selected.arm_id, "high");
    }

    #[test]
    fn epsilon_greedy_selector_explore_picks_by_rng() {
        let arms = vec![
            BanditArm::new("arm0", release("pr_0")),
            BanditArm::new("arm1", release("pr_1")),
        ];
        // rng = 0.05, epsilon = 0.1 → explore, idx = floor(0.05/0.1 * 2) = 1 → "arm1"
        let selected = epsilon_greedy_select(&arms, 0.1, 0.05);
        assert_eq!(selected.arm_id, "arm1");
    }

    #[test]
    fn ucb1_selector_picks_unpulled() {
        let arms = vec![
            { let mut a = BanditArm::new("pulled", release("pr_1")); a.pulls = 5; a.reward_sum = 4.5; a },
            BanditArm::new("fresh", release("pr_2")),
        ];
        let selected = ucb1_select(&arms, 5);
        assert_eq!(selected.arm_id, "fresh");
    }
}
