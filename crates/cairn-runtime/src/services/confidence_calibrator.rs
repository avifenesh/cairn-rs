//! Confidence calibrator — port of Go PR #1231.
//!
//! Queries the last 7 days of outcome records for a project, groups by
//! `agent_type`, and computes how much each agent type over- or under-estimates
//! its own confidence.  The returned `CalibrationAdjustment` tells the caller
//! how to scale predicted confidence before acting on it.
//!
//! Usage:
//! ```ignore
//! let calibrator = ConfidenceCalibrator::new(store.clone());
//! let adjustments = calibrator.calibrate(&project).await?;
//! if let Some(adj) = adjustments.get("code_review") {
//!     let calibrated = adj.apply(raw_confidence);
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::events::ActualOutcome;
use cairn_domain::ProjectKey;
use cairn_store::error::StoreError;
use cairn_store::projections::OutcomeReadModel;

/// How much to adjust raw predicted confidence for a given agent type.
///
/// `adjustment = actual_success_rate - mean_predicted_confidence`
///
/// A positive value means the agent is under-confident (predicts lower than
/// reality); a negative value means it is over-confident.
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationAdjustment {
    pub agent_type: String,
    /// Number of outcomes in the window used to compute this adjustment.
    pub sample_count: usize,
    /// Fraction of outcomes that were `Success` [0.0, 1.0].
    pub actual_success_rate: f64,
    /// Mean predicted confidence across outcomes [0.0, 1.0].
    pub mean_predicted_confidence: f64,
    /// Signed delta: positive → agent under-estimates; negative → over-estimates.
    pub adjustment: f64,
}

impl CalibrationAdjustment {
    /// Apply this adjustment to a raw predicted confidence value, clamping to
    /// [0.0, 1.0].
    pub fn apply(&self, raw: f64) -> f64 {
        (raw + self.adjustment).clamp(0.0, 1.0)
    }
}

/// Queries outcome records and returns per-agent-type calibration adjustments.
pub struct ConfidenceCalibrator<S> {
    store: Arc<S>,
}

impl<S> ConfidenceCalibrator<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

/// 7 days in milliseconds.
const WINDOW_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

/// Maximum outcomes to fetch per project (prevents unbounded reads).
const MAX_OUTCOMES: usize = 10_000;

impl<S> ConfidenceCalibrator<S>
where
    S: OutcomeReadModel,
{
    /// Compute calibration adjustments for every agent type that has at least
    /// one outcome in the last 7 days.
    pub async fn calibrate(
        &self,
        project: &ProjectKey,
    ) -> Result<HashMap<String, CalibrationAdjustment>, StoreError> {
        let cutoff_ms = now_millis().saturating_sub(WINDOW_MS);

        let all = OutcomeReadModel::list_by_project(self.store.as_ref(), project, MAX_OUTCOMES, 0)
            .await?;

        // Filter to the 7-day window.
        let recent: Vec<_> = all
            .into_iter()
            .filter(|r| r.recorded_at >= cutoff_ms)
            .collect();

        // Accumulate per agent_type: (success_count, total, sum_predicted).
        let mut buckets: HashMap<String, (usize, usize, f64)> = HashMap::new();
        for record in &recent {
            let entry = buckets.entry(record.agent_type.clone()).or_default();
            entry.1 += 1;
            entry.2 += record.predicted_confidence;
            if record.actual_outcome == ActualOutcome::Success {
                entry.0 += 1;
            }
        }

        let adjustments = buckets
            .into_iter()
            .map(|(agent_type, (successes, total, sum_pred))| {
                let actual_success_rate = successes as f64 / total as f64;
                let mean_predicted_confidence = sum_pred / total as f64;
                let adjustment = actual_success_rate - mean_predicted_confidence;
                let adj = CalibrationAdjustment {
                    agent_type: agent_type.clone(),
                    sample_count: total,
                    actual_success_rate,
                    mean_predicted_confidence,
                    adjustment,
                };
                (agent_type, adj)
            })
            .collect();

        Ok(adjustments)
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cairn_domain::{OutcomeId, RunId};
    use cairn_store::error::StoreError;
    use cairn_store::projections::OutcomeRecord;

    fn project() -> ProjectKey {
        ProjectKey::new("tenant", "workspace", "project")
    }

    fn run_id(s: &str) -> RunId {
        RunId::new(s)
    }

    fn outcome_id(s: &str) -> OutcomeId {
        OutcomeId::new(s)
    }

    fn record(
        agent_type: &str,
        predicted: f64,
        outcome: ActualOutcome,
        age_ms: u64,
    ) -> OutcomeRecord {
        let now = now_millis();
        OutcomeRecord {
            outcome_id: outcome_id(agent_type),
            run_id: run_id(agent_type),
            project: project(),
            agent_type: agent_type.to_owned(),
            predicted_confidence: predicted,
            actual_outcome: outcome,
            recorded_at: now.saturating_sub(age_ms),
        }
    }

    // Minimal in-memory stub for tests.
    struct StubStore {
        records: Vec<OutcomeRecord>,
    }

    #[async_trait]
    impl OutcomeReadModel for StubStore {
        async fn get(&self, id: &OutcomeId) -> Result<Option<OutcomeRecord>, StoreError> {
            Ok(self.records.iter().find(|r| r.outcome_id == *id).cloned())
        }

        async fn list_by_run(
            &self,
            run_id: &RunId,
            limit: usize,
        ) -> Result<Vec<OutcomeRecord>, StoreError> {
            let mut results: Vec<_> = self
                .records
                .iter()
                .filter(|r| r.run_id == *run_id)
                .cloned()
                .collect();
            results.truncate(limit);
            Ok(results)
        }

        async fn list_by_project(
            &self,
            project: &ProjectKey,
            limit: usize,
            offset: usize,
        ) -> Result<Vec<OutcomeRecord>, StoreError> {
            let mut results: Vec<_> = self
                .records
                .iter()
                .filter(|r| r.project == *project)
                .cloned()
                .collect();
            results.sort_by_key(|r| r.recorded_at);
            let results: Vec<_> = results.into_iter().skip(offset).take(limit).collect();
            Ok(results)
        }
    }

    #[tokio::test]
    async fn no_outcomes_returns_empty_map() {
        let cal = ConfidenceCalibrator::new(Arc::new(StubStore { records: vec![] }));
        let result = cal.calibrate(&project()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn perfect_calibration_gives_zero_adjustment() {
        // Agent predicts 0.8 confidence and succeeds 80% of the time.
        let records = vec![
            record("research", 0.8, ActualOutcome::Success, 1_000),
            record("research", 0.8, ActualOutcome::Success, 2_000),
            record("research", 0.8, ActualOutcome::Success, 3_000),
            record("research", 0.8, ActualOutcome::Success, 4_000),
            record("research", 0.8, ActualOutcome::Failure, 5_000),
        ];
        let cal = ConfidenceCalibrator::new(Arc::new(StubStore { records }));
        let result = cal.calibrate(&project()).await.unwrap();
        let adj = result.get("research").unwrap();
        assert_eq!(adj.sample_count, 5);
        assert!((adj.actual_success_rate - 0.8).abs() < 1e-9);
        assert!((adj.mean_predicted_confidence - 0.8).abs() < 1e-9);
        assert!(adj.adjustment.abs() < 1e-9);
    }

    #[tokio::test]
    async fn over_confident_agent_gets_negative_adjustment() {
        // Agent always predicts 0.9 but only succeeds 50% of the time.
        let records = vec![
            record("code_review", 0.9, ActualOutcome::Success, 1_000),
            record("code_review", 0.9, ActualOutcome::Failure, 2_000),
        ];
        let cal = ConfidenceCalibrator::new(Arc::new(StubStore { records }));
        let result = cal.calibrate(&project()).await.unwrap();
        let adj = result.get("code_review").unwrap();
        assert_eq!(adj.sample_count, 2);
        assert!((adj.actual_success_rate - 0.5).abs() < 1e-9);
        assert!((adj.mean_predicted_confidence - 0.9).abs() < 1e-9);
        // adjustment = 0.5 - 0.9 = -0.4
        assert!((adj.adjustment - (-0.4)).abs() < 1e-9);
        // Applying to a 0.9 raw prediction should give 0.5
        assert!((adj.apply(0.9) - 0.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn under_confident_agent_gets_positive_adjustment() {
        // Agent always predicts 0.3 but succeeds 70% of the time.
        let records = vec![
            record("planner", 0.3, ActualOutcome::Success, 1_000),
            record("planner", 0.3, ActualOutcome::Success, 2_000),
            record("planner", 0.3, ActualOutcome::Success, 3_000),
            record("planner", 0.3, ActualOutcome::Failure, 4_000),
        ];
        let cal = ConfidenceCalibrator::new(Arc::new(StubStore { records }));
        let result = cal.calibrate(&project()).await.unwrap();
        let adj = result.get("planner").unwrap();
        // adjustment = 0.75 - 0.3 = 0.45
        assert!((adj.adjustment - 0.45).abs() < 1e-9);
        assert!((adj.apply(0.3) - 0.75).abs() < 1e-9);
    }

    #[tokio::test]
    async fn outcomes_outside_window_are_excluded() {
        let eight_days_ms = 8 * 24 * 60 * 60 * 1_000;
        let records = vec![
            record("stale_agent", 0.9, ActualOutcome::Success, eight_days_ms),
            record("fresh_agent", 0.5, ActualOutcome::Failure, 1_000),
        ];
        let cal = ConfidenceCalibrator::new(Arc::new(StubStore { records }));
        let result = cal.calibrate(&project()).await.unwrap();
        // stale_agent's outcome is outside the 7-day window.
        assert!(
            !result.contains_key("stale_agent"),
            "stale record should be excluded"
        );
        assert!(result.contains_key("fresh_agent"));
    }

    #[tokio::test]
    async fn multiple_agent_types_are_independent() {
        let records = vec![
            record("type_a", 0.8, ActualOutcome::Success, 100),
            record("type_b", 0.6, ActualOutcome::Failure, 200),
        ];
        let cal = ConfidenceCalibrator::new(Arc::new(StubStore { records }));
        let result = cal.calibrate(&project()).await.unwrap();
        assert_eq!(result.len(), 2);
        let a = result.get("type_a").unwrap();
        let b = result.get("type_b").unwrap();
        assert!((a.adjustment - 0.2).abs() < 1e-9); // 1.0 - 0.8 = 0.2
        assert!((b.adjustment - (-0.6)).abs() < 1e-9); // 0.0 - 0.6 = -0.6
    }

    #[tokio::test]
    async fn apply_clamps_to_unit_interval() {
        let adj = CalibrationAdjustment {
            agent_type: "x".into(),
            sample_count: 1,
            actual_success_rate: 1.0,
            mean_predicted_confidence: 0.1,
            adjustment: 0.9,
        };
        // 0.8 + 0.9 = 1.7 → clamped to 1.0
        assert!((adj.apply(0.8) - 1.0).abs() < 1e-9);
        let adj_neg = CalibrationAdjustment {
            adjustment: -0.9,
            ..adj
        };
        // 0.1 - 0.9 = -0.8 → clamped to 0.0
        assert!((adj_neg.apply(0.1) - 0.0).abs() < 1e-9);
    }
}
