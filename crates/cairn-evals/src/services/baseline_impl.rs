use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cairn_domain::{
    BaselineComparison, EvalBaseline, EvalMetrics, EvalRunId, PromptAssetId, TenantId,
};

use crate::services::EvalRunService;

#[derive(Debug)]
pub enum EvalBaselineError {
    BaselineNotFound(String),
    EvalRunNotFound(String),
    MissingPromptAssetId(String),
}

impl std::fmt::Display for EvalBaselineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalBaselineError::BaselineNotFound(id) => write!(f, "eval baseline not found: {id}"),
            EvalBaselineError::EvalRunNotFound(id) => write!(f, "eval run not found: {id}"),
            EvalBaselineError::MissingPromptAssetId(id) => {
                write!(f, "eval run has no prompt_asset_id: {id}")
            }
        }
    }
}

impl std::error::Error for EvalBaselineError {}

struct BaselineState {
    baselines: HashMap<String, EvalBaseline>,
}

pub struct EvalBaselineServiceImpl {
    state: Mutex<BaselineState>,
    eval_runs: Arc<EvalRunService>,
}

impl EvalBaselineServiceImpl {
    pub fn new(eval_runs: Arc<EvalRunService>) -> Self {
        Self {
            state: Mutex::new(BaselineState {
                baselines: HashMap::new(),
            }),
            eval_runs,
        }
    }

    pub fn set_baseline(
        &self,
        tenant_id: TenantId,
        name: String,
        prompt_asset_id: PromptAssetId,
        metrics: EvalMetrics,
    ) -> EvalBaseline {
        let baseline_id = format!("baseline_{}", now_millis());
        let baseline = EvalBaseline {
            baseline_id: baseline_id.clone(),
            tenant_id,
            name,
            prompt_asset_id,
            metrics,
            created_at_ms: now_millis(),
            locked: false,
        };
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .baselines
            .insert(baseline_id, baseline.clone());
        baseline
    }

    pub fn lock(&self, baseline_id: &str) -> Result<EvalBaseline, EvalBaselineError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let baseline = state
            .baselines
            .get_mut(baseline_id)
            .ok_or_else(|| EvalBaselineError::BaselineNotFound(baseline_id.to_owned()))?;
        baseline.locked = true;
        Ok(baseline.clone())
    }

    pub fn get(&self, baseline_id: &str) -> Option<EvalBaseline> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .baselines
            .get(baseline_id)
            .cloned()
    }

    pub fn list(&self, tenant_id: &TenantId) -> Vec<EvalBaseline> {
        let mut baselines = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .baselines
            .values()
            .filter(|baseline| baseline.tenant_id == *tenant_id)
            .cloned()
            .collect::<Vec<_>>();
        baselines.sort_by_key(|baseline| (baseline.created_at_ms, baseline.baseline_id.clone()));
        baselines
    }

    pub fn compare_to_baseline(
        &self,
        eval_run_id: &EvalRunId,
    ) -> Result<BaselineComparison, EvalBaselineError> {
        let run = self
            .eval_runs
            .get(eval_run_id)
            .ok_or_else(|| EvalBaselineError::EvalRunNotFound(eval_run_id.to_string()))?;
        let prompt_asset_id = run
            .prompt_asset_id
            .clone()
            .ok_or_else(|| EvalBaselineError::MissingPromptAssetId(eval_run_id.to_string()))?;
        let baseline = self
            .select_baseline(&prompt_asset_id)
            .ok_or_else(|| EvalBaselineError::BaselineNotFound(prompt_asset_id.to_string()))?;

        let mut regressions = Vec::new();
        let mut improvements = Vec::new();

        compare_metric(
            "task_success_rate",
            run.metrics.task_success_rate,
            baseline.metrics.task_success_rate,
            false,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "policy_pass_rate",
            run.metrics.policy_pass_rate,
            baseline.metrics.policy_pass_rate,
            false,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "retrieval_hit_at_k",
            run.metrics.retrieval_hit_at_k,
            baseline.metrics.retrieval_hit_at_k,
            false,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "citation_coverage",
            run.metrics.citation_coverage,
            baseline.metrics.citation_coverage,
            false,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "source_diversity",
            run.metrics.source_diversity,
            baseline.metrics.source_diversity,
            false,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "latency_p50_ms",
            run.metrics.latency_p50_ms.map(|v| v as f64),
            baseline.metrics.latency_p50_ms.map(|v| v as f64),
            true,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "latency_p99_ms",
            run.metrics.latency_p99_ms.map(|v| v as f64),
            baseline.metrics.latency_p99_ms.map(|v| v as f64),
            true,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "retrieval_latency_ms",
            run.metrics.retrieval_latency_ms.map(|v| v as f64),
            baseline.metrics.retrieval_latency_ms.map(|v| v as f64),
            true,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "cost_per_run",
            run.metrics.cost_per_run,
            baseline.metrics.cost_per_run,
            true,
            &mut regressions,
            &mut improvements,
        );
        compare_metric(
            "retrieval_cost",
            run.metrics.retrieval_cost,
            baseline.metrics.retrieval_cost,
            true,
            &mut regressions,
            &mut improvements,
        );

        let passed = regressions.is_empty();

        Ok(BaselineComparison {
            run_id: run.eval_run_id.to_string(),
            baseline_id: baseline.baseline_id.clone(),
            run_metrics: run.metrics.clone(),
            baseline_metrics: baseline.metrics.clone(),
            regressions,
            improvements,
            passed,
        })
    }

    fn select_baseline(&self, prompt_asset_id: &PromptAssetId) -> Option<EvalBaseline> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut matches = state
            .baselines
            .values()
            .filter(|baseline| baseline.prompt_asset_id == *prompt_asset_id)
            .cloned()
            .collect::<Vec<_>>();
        matches.sort_by_key(|baseline| (baseline.created_at_ms, baseline.baseline_id.clone()));
        matches
            .iter()
            .rev()
            .find(|baseline| baseline.locked)
            .cloned()
            .or_else(|| matches.into_iter().last())
    }
}

impl Default for EvalBaselineServiceImpl {
    fn default() -> Self {
        Self::new(Arc::new(EvalRunService::new()))
    }
}

fn compare_metric(
    name: &str,
    run: Option<f64>,
    baseline: Option<f64>,
    lower_is_better: bool,
    regressions: &mut Vec<String>,
    improvements: &mut Vec<String>,
) {
    let (Some(run), Some(baseline)) = (run, baseline) else {
        return;
    };
    if baseline <= 0.0 {
        return;
    }

    let delta_ratio = if lower_is_better {
        (baseline - run) / baseline
    } else {
        (run - baseline) / baseline
    };

    if delta_ratio <= -0.05 {
        regressions.push(name.to_owned());
    } else if delta_ratio >= 0.05 {
        improvements.push(name.to_owned());
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
