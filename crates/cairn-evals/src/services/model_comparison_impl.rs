//! Model comparison service — RFC 004.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static COMPARISON_COUNTER: AtomicU64 = AtomicU64::new(1);

use cairn_domain::evals::{EvalMetrics, ModelComparisonRun, ModelComparisonStatus};
use cairn_domain::TenantId;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug)]
pub enum ModelComparisonError {
    NotFound(String),
    AlreadyCompleted(String),
    UnknownBinding(String),
}

impl std::fmt::Display for ModelComparisonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "comparison not found: {id}"),
            Self::AlreadyCompleted(id) => write!(f, "comparison already completed: {id}"),
            Self::UnknownBinding(id) => write!(f, "unknown binding id: {id}"),
        }
    }
}

impl std::error::Error for ModelComparisonError {}

pub struct ModelComparisonServiceImpl {
    comparisons: Mutex<HashMap<String, ModelComparisonRun>>,
}

impl ModelComparisonServiceImpl {
    pub fn new() -> Self {
        Self {
            comparisons: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new pending comparison between two model bindings.
    pub fn create(
        &self,
        tenant_id: TenantId,
        dataset_id: String,
        model_a_binding_id: String,
        model_b_binding_id: String,
    ) -> ModelComparisonRun {
        let seq = COMPARISON_COUNTER.fetch_add(1, Ordering::SeqCst);
        let comparison_id = format!("cmp_{}_{}_{}", tenant_id.as_str(), now_ms(), seq);
        let run = ModelComparisonRun {
            comparison_id: comparison_id.clone(),
            tenant_id,
            dataset_id,
            model_a_binding_id,
            model_b_binding_id,
            status: ModelComparisonStatus::Pending,
            results_a: None,
            results_b: None,
            winner: None,
            created_at_ms: now_ms(),
        };
        self.comparisons
            .lock()
            .unwrap()
            .insert(comparison_id, run.clone());
        run
    }

    /// Submit evaluation results for one binding in the comparison.
    ///
    /// Transitions to Running after the first result and Completed once both
    /// results are in; automatically determines the winner on completion.
    pub fn submit_result(
        &self,
        comparison_id: &str,
        binding_id: &str,
        metrics: EvalMetrics,
    ) -> Result<ModelComparisonRun, ModelComparisonError> {
        let mut map = self.comparisons.lock().unwrap();
        let run = map
            .get_mut(comparison_id)
            .ok_or_else(|| ModelComparisonError::NotFound(comparison_id.to_owned()))?;

        if run.status == ModelComparisonStatus::Completed {
            return Err(ModelComparisonError::AlreadyCompleted(
                comparison_id.to_owned(),
            ));
        }

        if binding_id == run.model_a_binding_id {
            run.results_a = Some(metrics);
        } else if binding_id == run.model_b_binding_id {
            run.results_b = Some(metrics);
        } else {
            return Err(ModelComparisonError::UnknownBinding(binding_id.to_owned()));
        }

        if run.results_a.is_some() && run.results_b.is_some() {
            run.status = ModelComparisonStatus::Completed;
            run.winner = Self::pick_winner(run);
        } else {
            run.status = ModelComparisonStatus::Running;
        }

        Ok(run.clone())
    }

    /// Return the binding_id with the higher task_success_rate, or None if
    /// both results are absent / equal.
    pub fn determine_winner(
        &self,
        comparison_id: &str,
    ) -> Result<Option<String>, ModelComparisonError> {
        let map = self.comparisons.lock().unwrap();
        let run = map
            .get(comparison_id)
            .ok_or_else(|| ModelComparisonError::NotFound(comparison_id.to_owned()))?;
        Ok(Self::pick_winner(run))
    }

    /// Retrieve a comparison by id.
    pub fn get(&self, comparison_id: &str) -> Option<ModelComparisonRun> {
        self.comparisons.lock().unwrap().get(comparison_id).cloned()
    }

    /// List all comparisons for a tenant.
    pub fn list_by_tenant(&self, tenant_id: &TenantId) -> Vec<ModelComparisonRun> {
        let map = self.comparisons.lock().unwrap();
        let mut items: Vec<_> = map
            .values()
            .filter(|c| c.tenant_id == *tenant_id)
            .cloned()
            .collect();
        items.sort_by_key(|c| c.created_at_ms);
        items
    }

    fn pick_winner(run: &ModelComparisonRun) -> Option<String> {
        let rate_a = run
            .results_a
            .as_ref()
            .and_then(|m| m.task_success_rate)
            .unwrap_or(0.0);
        let rate_b = run
            .results_b
            .as_ref()
            .and_then(|m| m.task_success_rate)
            .unwrap_or(0.0);

        if rate_a > rate_b {
            Some(run.model_a_binding_id.clone())
        } else if rate_b > rate_a {
            Some(run.model_b_binding_id.clone())
        } else {
            None // tie
        }
    }
}

impl Default for ModelComparisonServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::evals::EvalMetrics;

    #[test]
    fn model_comparison_winner_is_higher_task_success_rate() {
        let svc = ModelComparisonServiceImpl::new();
        let tenant_id = TenantId::new("tenant_mc");

        let comparison = svc.create(
            tenant_id.clone(),
            "dataset_1".to_owned(),
            "binding_a".to_owned(),
            "binding_b".to_owned(),
        );

        assert_eq!(comparison.status, ModelComparisonStatus::Pending);
        assert!(comparison.winner.is_none());

        // Submit result for model A (0.7)
        let metrics_a = EvalMetrics {
            task_success_rate: Some(0.7),
            ..EvalMetrics::default()
        };
        let after_a = svc
            .submit_result(&comparison.comparison_id, "binding_a", metrics_a)
            .unwrap();
        assert_eq!(after_a.status, ModelComparisonStatus::Running);
        assert!(after_a.winner.is_none());

        // Submit result for model B (0.9)
        let metrics_b = EvalMetrics {
            task_success_rate: Some(0.9),
            ..EvalMetrics::default()
        };
        let after_b = svc
            .submit_result(&comparison.comparison_id, "binding_b", metrics_b)
            .unwrap();
        assert_eq!(after_b.status, ModelComparisonStatus::Completed);
        assert_eq!(after_b.winner, Some("binding_b".to_owned()));

        // determine_winner returns binding_b
        let winner = svc.determine_winner(&comparison.comparison_id).unwrap();
        assert_eq!(winner, Some("binding_b".to_owned()));

        // GET returns correct comparison
        let fetched = svc.get(&comparison.comparison_id).unwrap();
        assert_eq!(fetched.winner, Some("binding_b".to_owned()));
        assert_eq!(fetched.status, ModelComparisonStatus::Completed);
        assert_eq!(
            fetched.results_a.as_ref().and_then(|m| m.task_success_rate),
            Some(0.7)
        );
        assert_eq!(
            fetched.results_b.as_ref().and_then(|m| m.task_success_rate),
            Some(0.9)
        );
    }

    #[test]
    fn model_comparison_list_by_tenant_filters_correctly() {
        let svc = ModelComparisonServiceImpl::new();
        let t1 = TenantId::new("tenant_1");
        let t2 = TenantId::new("tenant_2");

        svc.create(t1.clone(), "ds".to_owned(), "a".to_owned(), "b".to_owned());
        svc.create(t1.clone(), "ds".to_owned(), "c".to_owned(), "d".to_owned());
        svc.create(t2.clone(), "ds".to_owned(), "e".to_owned(), "f".to_owned());

        assert_eq!(svc.list_by_tenant(&t1).len(), 2);
        assert_eq!(svc.list_by_tenant(&t2).len(), 1);
    }
}
