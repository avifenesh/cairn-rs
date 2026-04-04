use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cairn_domain::{
    EvalDataset, EvalRubric, EvalRunId, RubricDimension, RubricScoreResult, RubricScoringFn,
    TenantId,
};

use crate::scorecards::EvalRun;
use crate::services::{EvalDatasetServiceImpl, EvalRunService};

#[derive(Debug)]
pub enum EvalRubricError {
    DatasetNotFound(String),
    EvalRunNotFound(String),
    MissingDatasetId(String),
    RubricNotFound(String),
    OutputCountMismatch { expected: usize, actual: usize },
    MissingPluginId(String),
    PluginScorerNotConfigured(String),
    PluginScoreFailed(String),
}

impl std::fmt::Display for EvalRubricError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalRubricError::DatasetNotFound(id) => write!(f, "eval dataset not found: {id}"),
            EvalRubricError::EvalRunNotFound(id) => write!(f, "eval run not found: {id}"),
            EvalRubricError::MissingDatasetId(id) => {
                write!(f, "eval run has no dataset_id: {id}")
            }
            EvalRubricError::RubricNotFound(id) => write!(f, "eval rubric not found: {id}"),
            EvalRubricError::OutputCountMismatch { expected, actual } => {
                write!(
                    f,
                    "actual output count mismatch: expected {expected}, got {actual}"
                )
            }
            EvalRubricError::MissingPluginId(name) => {
                write!(f, "rubric dimension missing plugin_id: {name}")
            }
            EvalRubricError::PluginScorerNotConfigured(name) => {
                write!(f, "plugin scorer not configured for dimension: {name}")
            }
            EvalRubricError::PluginScoreFailed(message) => {
                write!(f, "plugin scoring failed: {message}")
            }
        }
    }
}

impl std::error::Error for EvalRubricError {}

struct RubricState {
    rubrics: HashMap<String, EvalRubric>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginDimensionScore {
    pub score: f64,
    pub passed: bool,
    pub feedback: Option<String>,
}

#[async_trait]
pub trait PluginRubricScorer: Send + Sync {
    async fn score(
        &self,
        plugin_id: &str,
        input: &serde_json::Value,
        expected_output: Option<&serde_json::Value>,
        actual_output: &serde_json::Value,
    ) -> Result<PluginDimensionScore, EvalRubricError>;
}

pub struct EvalRubricServiceImpl {
    state: Mutex<RubricState>,
    eval_runs: Arc<EvalRunService>,
    datasets: Arc<EvalDatasetServiceImpl>,
    plugin_scorer: Option<Arc<dyn PluginRubricScorer>>,
}

impl EvalRubricServiceImpl {
    pub fn new(eval_runs: Arc<EvalRunService>, datasets: Arc<EvalDatasetServiceImpl>) -> Self {
        Self {
            state: Mutex::new(RubricState {
                rubrics: HashMap::new(),
            }),
            eval_runs,
            datasets,
            plugin_scorer: None,
        }
    }

    pub fn with_plugin_scorer(
        eval_runs: Arc<EvalRunService>,
        datasets: Arc<EvalDatasetServiceImpl>,
        plugin_scorer: Arc<dyn PluginRubricScorer>,
    ) -> Self {
        Self {
            state: Mutex::new(RubricState {
                rubrics: HashMap::new(),
            }),
            eval_runs,
            datasets,
            plugin_scorer: Some(plugin_scorer),
        }
    }

    pub fn create(
        &self,
        tenant_id: TenantId,
        name: String,
        dimensions: Vec<RubricDimension>,
    ) -> EvalRubric {
        let rubric_id = format!("rubric_{}", now_millis());
        let rubric = EvalRubric {
            rubric_id: rubric_id.clone(),
            tenant_id,
            name,
            dimensions,
            created_at_ms: now_millis(),
        };
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rubrics
            .insert(rubric_id, rubric.clone());
        rubric
    }

    pub fn get(&self, rubric_id: &str) -> Option<EvalRubric> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rubrics
            .get(rubric_id)
            .cloned()
    }

    pub fn list(&self, tenant_id: &TenantId) -> Vec<EvalRubric> {
        let mut rubrics = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rubrics
            .values()
            .filter(|rubric| rubric.tenant_id == *tenant_id)
            .cloned()
            .collect::<Vec<_>>();
        rubrics.sort_by_key(|rubric| (rubric.created_at_ms, rubric.rubric_id.clone()));
        rubrics
    }

    pub async fn score_against_rubric(
        &self,
        eval_run_id: &EvalRunId,
        rubric_id: &str,
        actual_outputs: &[serde_json::Value],
    ) -> Result<RubricScoreResult, EvalRubricError> {
        let run = self
            .eval_runs
            .get(eval_run_id)
            .ok_or_else(|| EvalRubricError::EvalRunNotFound(eval_run_id.to_string()))?;
        let dataset_id = run
            .dataset_id
            .clone()
            .ok_or_else(|| EvalRubricError::MissingDatasetId(eval_run_id.to_string()))?;
        let dataset = self
            .datasets
            .get(&dataset_id)
            .ok_or_else(|| EvalRubricError::DatasetNotFound(dataset_id.clone()))?;
        let rubric = self
            .get(rubric_id)
            .ok_or_else(|| EvalRubricError::RubricNotFound(rubric_id.to_owned()))?;

        if dataset.entries.len() != actual_outputs.len() {
            return Err(EvalRubricError::OutputCountMismatch {
                expected: dataset.entries.len(),
                actual: actual_outputs.len(),
            });
        }

        score_dataset_against_rubric(
            &run,
            &dataset,
            &rubric,
            actual_outputs,
            self.plugin_scorer.as_deref(),
        )
        .await
    }
}

impl Default for EvalRubricServiceImpl {
    fn default() -> Self {
        Self::new(
            Arc::new(EvalRunService::new()),
            Arc::new(EvalDatasetServiceImpl::new()),
        )
    }
}

async fn score_dataset_against_rubric(
    run: &EvalRun,
    dataset: &EvalDataset,
    rubric: &EvalRubric,
    actual_outputs: &[serde_json::Value],
    plugin_scorer: Option<&dyn PluginRubricScorer>,
) -> Result<RubricScoreResult, EvalRubricError> {
    let mut weighted_sum = 0.0;
    let mut total_weight = 0.0;
    let mut dimension_scores = Vec::new();
    for dimension in &rubric.dimensions {
        let score = score_dimension(dimension, dataset, actual_outputs, plugin_scorer).await?;
        if dimension.weight > 0.0 {
            weighted_sum += score * dimension.weight;
            total_weight += dimension.weight;
        }
        dimension_scores.push((dimension.name.clone(), score));
    }

    let overall = if total_weight > 0.0 {
        weighted_sum / total_weight
    } else if dimension_scores.is_empty() {
        0.0
    } else {
        dimension_scores
            .iter()
            .map(|(_, score)| *score)
            .sum::<f64>()
            / dimension_scores.len() as f64
    };

    Ok(RubricScoreResult {
        run_id: run.eval_run_id.to_string(),
        rubric_id: rubric.rubric_id.clone(),
        dimension_scores,
        overall: overall.clamp(0.0, 1.0),
    })
}

async fn score_dimension(
    dimension: &RubricDimension,
    dataset: &EvalDataset,
    actual_outputs: &[serde_json::Value],
    plugin_scorer: Option<&dyn PluginRubricScorer>,
) -> Result<f64, EvalRubricError> {
    if dataset.entries.is_empty() {
        return Ok(0.0);
    }

    let mut total = 0.0;
    for (entry, actual) in dataset.entries.iter().zip(actual_outputs.iter()) {
        total += score_expected_vs_actual(
            dimension,
            &entry.input,
            entry.expected_output.as_ref(),
            actual,
            plugin_scorer,
        )
        .await?;
    }

    Ok((total / dataset.entries.len() as f64).clamp(0.0, 1.0))
}

async fn score_expected_vs_actual(
    dimension: &RubricDimension,
    input: &serde_json::Value,
    expected: Option<&serde_json::Value>,
    actual: &serde_json::Value,
    plugin_scorer: Option<&dyn PluginRubricScorer>,
) -> Result<f64, EvalRubricError> {
    let score = match &dimension.scoring_fn {
        RubricScoringFn::ExactMatch => {
            if expected == Some(actual) {
                1.0
            } else {
                0.0
            }
        }
        RubricScoringFn::Contains => contains_score(expected, actual),
        RubricScoringFn::Similarity => similarity_score(expected, actual),
        RubricScoringFn::Plugin => {
            let plugin_id = dimension
                .plugin_id
                .as_deref()
                .ok_or_else(|| EvalRubricError::MissingPluginId(dimension.name.clone()))?;
            let plugin_scorer = plugin_scorer.ok_or_else(|| {
                EvalRubricError::PluginScorerNotConfigured(dimension.name.clone())
            })?;
            plugin_scorer
                .score(plugin_id, input, expected, actual)
                .await?
                .score
        }
        RubricScoringFn::Custom => 0.0,
    };

    Ok(match dimension.threshold {
        Some(threshold) if score < threshold => 0.0,
        _ => score,
    })
}

fn contains_score(expected: Option<&serde_json::Value>, actual: &serde_json::Value) -> f64 {
    let Some(expected) = expected else {
        return 0.0;
    };
    match (expected, actual) {
        (serde_json::Value::String(expected), serde_json::Value::String(actual)) => {
            if actual.to_lowercase().contains(&expected.to_lowercase()) {
                1.0
            } else {
                0.0
            }
        }
        _ => {
            if expected == actual {
                1.0
            } else {
                0.0
            }
        }
    }
}

fn similarity_score(expected: Option<&serde_json::Value>, actual: &serde_json::Value) -> f64 {
    let Some(expected) = expected else {
        return 0.0;
    };
    match (expected, actual) {
        (serde_json::Value::String(expected), serde_json::Value::String(actual)) => {
            let expected_tokens = tokenize(expected);
            let actual_tokens = tokenize(actual);
            if expected_tokens.is_empty() && actual_tokens.is_empty() {
                return 1.0;
            }
            let overlap = expected_tokens
                .iter()
                .filter(|token| actual_tokens.contains(token))
                .count();
            let union = expected_tokens.len() + actual_tokens.len() - overlap;
            if union == 0 {
                0.0
            } else {
                overlap as f64 / union as f64
            }
        }
        _ => {
            if expected == actual {
                1.0
            } else {
                0.0
            }
        }
    }
}

fn tokenize(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
