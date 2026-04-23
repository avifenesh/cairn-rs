//! Eval run service for creating runs, recording metrics, and building scorecards.

use cairn_domain::{
    EvalRunId, OperatorId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId,
};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::matrices::EvalMetrics;
use crate::scorecards::{EvalRun, EvalRunStatus, EvalSubjectKind, Scorecard, ScorecardEntry};

/// Eval service error.
#[derive(Debug)]
pub enum EvalError {
    NotFound(String),
    InvalidTransition {
        from: EvalRunStatus,
        to: EvalRunStatus,
    },
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::NotFound(id) => write!(f, "eval run not found: {id}"),
            EvalError::InvalidTransition { from, to } => {
                write!(f, "invalid eval transition: {from:?} -> {to:?}")
            }
        }
    }
}

impl std::error::Error for EvalError {}

struct EvalState {
    runs: HashMap<String, EvalRun>,
}

/// In-memory eval run service.
pub struct EvalRunService {
    state: Mutex<EvalState>,
}

impl EvalRunService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(EvalState {
                runs: HashMap::new(),
            }),
        }
    }

    /// Builder: attach graph integration + event log. Stub — ignored in in-memory impl.
    pub fn with_graph_and_event_log<G, S>(
        _graph_integration: std::sync::Arc<G>,
        _store: std::sync::Arc<S>,
    ) -> Self
    where
        G: Send + Sync + 'static,
        S: Send + Sync + 'static,
    {
        Self::new()
    }

    /// Builder: attach memory diagnostics. Stub — ignored in in-memory impl.
    pub fn with_memory_diagnostics<D>(self, _diagnostics: std::sync::Arc<D>) -> Self
    where
        D: Send + Sync + 'static,
    {
        self
    }

    /// Create a new eval run.
    pub fn create_run(
        &self,
        eval_run_id: EvalRunId,
        project_id: ProjectId,
        subject_kind: EvalSubjectKind,
        evaluator_type: String,
        prompt_asset_id: Option<PromptAssetId>,
        prompt_version_id: Option<PromptVersionId>,
        prompt_release_id: Option<PromptReleaseId>,
        created_by: Option<OperatorId>,
    ) -> EvalRun {
        let now = now_millis();
        let run = EvalRun {
            eval_run_id: eval_run_id.clone(),
            project_id,
            subject_kind,
            status: EvalRunStatus::Pending,
            prompt_asset_id,
            prompt_version_id,
            prompt_release_id,
            evaluator_type,
            dataset_id: None,
            dataset_source: None,
            rubric_id: None,
            baseline_id: None,
            metrics: EvalMetrics::default(),
            plugin_metrics: Vec::new(),
            cost: None,
            created_by,
            created_at: now,
            completed_at: None,
        };

        let mut state = self.state.lock().unwrap();
        state
            .runs
            .insert(eval_run_id.as_str().to_owned(), run.clone());
        run
    }

    /// Start an eval run (Pending -> Running).
    pub fn start_run(&self, eval_run_id: &EvalRunId) -> Result<EvalRun, EvalError> {
        let mut state = self.state.lock().unwrap();
        let run = state
            .runs
            .get_mut(eval_run_id.as_str())
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;

        if run.status != EvalRunStatus::Pending {
            return Err(EvalError::InvalidTransition {
                from: run.status,
                to: EvalRunStatus::Running,
            });
        }

        run.status = EvalRunStatus::Running;
        Ok(run.clone())
    }

    /// Complete an eval run with final metrics.
    pub fn complete_run(
        &self,
        eval_run_id: &EvalRunId,
        metrics: EvalMetrics,
        cost: Option<f64>,
    ) -> Result<EvalRun, EvalError> {
        let mut state = self.state.lock().unwrap();
        let run = state
            .runs
            .get_mut(eval_run_id.as_str())
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;

        if run.status != EvalRunStatus::Running {
            return Err(EvalError::InvalidTransition {
                from: run.status,
                to: EvalRunStatus::Completed,
            });
        }

        run.status = EvalRunStatus::Completed;
        run.metrics = metrics;
        run.cost = cost;
        run.completed_at = Some(now_millis());
        Ok(run.clone())
    }

    /// Link a dataset to an existing eval run.
    pub fn set_dataset_id(
        &self,
        eval_run_id: &EvalRunId,
        dataset_id: String,
    ) -> Result<(), EvalError> {
        let mut state = self.state.lock().unwrap();
        let run = state
            .runs
            .get_mut(eval_run_id.as_str())
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;
        run.dataset_id = Some(dataset_id);
        Ok(())
    }

    /// Link a rubric to an existing eval run (issue #223).
    ///
    /// Uses the poison-tolerant lock pattern (`unwrap_or_else(into_inner)`)
    /// so a panic on another thread while holding the mutex cannot take down
    /// the whole eval service. Matches the sibling `EvalBaselineService` /
    /// `EvalDatasetService` convention.
    pub fn set_rubric_id(
        &self,
        eval_run_id: &EvalRunId,
        rubric_id: String,
    ) -> Result<(), EvalError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let run = state
            .runs
            .get_mut(eval_run_id.as_str())
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;
        run.rubric_id = Some(rubric_id);
        Ok(())
    }

    /// Link a baseline to an existing eval run (issue #223).
    ///
    /// See `set_rubric_id` for the poison-tolerant locking rationale.
    pub fn set_baseline_id(
        &self,
        eval_run_id: &EvalRunId,
        baseline_id: String,
    ) -> Result<(), EvalError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let run = state
            .runs
            .get_mut(eval_run_id.as_str())
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;
        run.baseline_id = Some(baseline_id);
        Ok(())
    }

    /// Get an eval run by ID.
    pub fn get(&self, eval_run_id: &EvalRunId) -> Option<EvalRun> {
        let state = self.state.lock().unwrap();
        state.runs.get(eval_run_id.as_str()).cloned()
    }

    /// Build a scorecard for a prompt asset, comparing eval results across releases.
    pub fn build_scorecard(
        &self,
        project_id: &ProjectId,
        prompt_asset_id: &PromptAssetId,
    ) -> Scorecard {
        let state = self.state.lock().unwrap();

        let mut entries: Vec<ScorecardEntry> = state
            .runs
            .values()
            .filter(|r| {
                r.project_id == *project_id
                    && r.prompt_asset_id.as_ref() == Some(prompt_asset_id)
                    && r.status == EvalRunStatus::Completed
            })
            .filter_map(|r| {
                Some(ScorecardEntry {
                    prompt_release_id: r.prompt_release_id.clone()?,
                    prompt_version_id: r.prompt_version_id.clone()?,
                    eval_run_id: r.eval_run_id.clone(),
                    metrics: r.metrics.clone(),
                })
            })
            .collect();

        // Sort by task_success_rate descending so the best run is first.
        entries.sort_by(|a, b| {
            let a_score = a.metrics.task_success_rate.unwrap_or(0.0);
            let b_score = b.metrics.task_success_rate.unwrap_or(0.0);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Scorecard {
            project_id: project_id.clone(),
            prompt_asset_id: prompt_asset_id.clone(),
            entries,
        }
    }

    /// List all eval runs for a project.
    pub fn list_by_project(&self, project_id: &ProjectId) -> Vec<EvalRun> {
        let state = self.state.lock().unwrap();
        state
            .runs
            .values()
            .filter(|r| r.project_id == *project_id)
            .cloned()
            .collect()
    }

    /// Build a prompt comparison matrix for a prompt asset.
    pub fn build_prompt_comparison_matrix(
        &self,
        _project_id: &ProjectId,
        prompt_asset_id: &PromptAssetId,
    ) -> crate::matrices::PromptComparisonMatrix {
        let state = self.state.lock().unwrap();
        let mut rows: Vec<crate::matrices::PromptComparisonRow> = state
            .runs
            .values()
            .filter(|r| {
                r.prompt_asset_id.as_ref() == Some(prompt_asset_id)
                    && r.status == EvalRunStatus::Completed
            })
            .map(|r| crate::matrices::PromptComparisonRow {
                project_id: r.project_id.clone(),
                prompt_release_id: r
                    .prompt_release_id
                    .clone()
                    .unwrap_or_else(|| cairn_domain::PromptReleaseId::new("")),
                prompt_asset_id: r
                    .prompt_asset_id
                    .clone()
                    .unwrap_or_else(|| prompt_asset_id.clone()),
                prompt_version_id: r
                    .prompt_version_id
                    .clone()
                    .unwrap_or_else(|| cairn_domain::PromptVersionId::new("")),
                provider_binding_id: None,
                eval_run_id: r.eval_run_id.clone(),
                metrics: r.metrics.clone(),
            })
            .collect();
        rows.sort_by_key(|r| r.eval_run_id.as_str().to_owned());
        crate::matrices::PromptComparisonMatrix { rows }
    }

    /// Stub: build a permission matrix.
    pub async fn build_permission_matrix(
        &self,
        _tenant_id: &cairn_domain::TenantId,
    ) -> Result<crate::matrices::PermissionMatrix, EvalError> {
        Ok(crate::matrices::PermissionMatrix { rows: vec![] })
    }

    /// Stub: build a skill health matrix.
    pub async fn build_skill_health_matrix(
        &self,
        _tenant_id: &cairn_domain::TenantId,
    ) -> Result<crate::matrices::SkillHealthMatrix, EvalError> {
        Ok(crate::matrices::SkillHealthMatrix { rows: vec![] })
    }

    /// Stub: build a guardrail matrix.
    pub async fn build_guardrail_matrix(
        &self,
        _tenant_id: &cairn_domain::TenantId,
    ) -> Result<crate::matrices::GuardrailMatrix, EvalError> {
        Ok(crate::matrices::GuardrailMatrix { rows: vec![] })
    }

    /// Stub: build a memory source quality matrix.
    pub async fn build_memory_quality_matrix(
        &self,
        _project: &cairn_domain::ProjectKey,
    ) -> Result<crate::matrices::MemorySourceQualityMatrix, EvalError> {
        Ok(crate::matrices::MemorySourceQualityMatrix { rows: vec![] })
    }

    /// Stub: export runs to a JSON-serialisable list.
    pub fn export_runs(&self, project_id: &ProjectId, limit: usize) -> Vec<EvalRun> {
        self.list_by_project(project_id)
            .into_iter()
            .take(limit)
            .collect()
    }

    /// Record a score for a run without completing it.
    pub fn record_score(
        &self,
        eval_run_id: &EvalRunId,
        metrics: crate::matrices::EvalMetrics,
    ) -> Result<EvalRun, EvalError> {
        let mut state = self.state.lock().unwrap();
        let run = state
            .runs
            .get_mut(eval_run_id.as_str())
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;
        if run.status != crate::EvalRunStatus::Running {
            return Err(EvalError::InvalidTransition {
                from: run.status,
                to: crate::EvalRunStatus::Running,
            });
        }
        // Update metrics only — do not change status.
        run.metrics = metrics;
        Ok(run.clone())
    }

    // ── RFC 004 Gap 2: Rubric scoring ──────────────────────────────────────

    /// Score an eval run against a rubric definition.
    ///
    /// The rubric is a list of `{ dimension, weight, criteria }`. Each
    /// dimension is scored by matching the criteria against the run's metrics.
    /// Returns per-dimension scores and a weighted overall score.
    pub fn score_with_rubric(
        &self,
        eval_run_id: &EvalRunId,
        rubric: &[crate::matrices::RubricDimensionDef],
    ) -> Result<crate::matrices::RubricScoringResult, EvalError> {
        let run = self
            .get(eval_run_id)
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;

        let mut dimensions = Vec::new();
        let mut weighted_sum = 0.0;
        let mut total_weight = 0.0;

        for dim in rubric {
            let raw_score = eval_metric_by_name(&run.metrics, &dim.criteria);
            let score = raw_score.unwrap_or(0.0).clamp(0.0, 1.0);
            let weighted = score * dim.weight as f64;

            dimensions.push(crate::matrices::DimensionScore {
                dimension: dim.dimension.clone(),
                weight: dim.weight as f64,
                score,
                weighted_score: weighted,
            });

            if dim.weight > 0.0 {
                weighted_sum += weighted;
                total_weight += dim.weight as f64;
            }
        }

        let overall = if total_weight > 0.0 {
            (weighted_sum / total_weight).clamp(0.0, 1.0)
        } else {
            0.0
        };

        Ok(crate::matrices::RubricScoringResult {
            eval_run_id: eval_run_id.as_str().to_owned(),
            dimensions,
            overall_score: overall,
        })
    }

    // ── RFC 004 Gap 3: Run-to-run comparison ─────────────────────────────

    /// Compare two eval runs, computing per-metric deltas and flagging regressions.
    ///
    /// `threshold` is the minimum absolute delta ratio to flag as regression
    /// (default 0.05 = 5%). A metric is a regression when the candidate is
    /// worse than baseline by more than the threshold.
    pub fn compare_runs(
        &self,
        baseline_run_id: &EvalRunId,
        candidate_run_id: &EvalRunId,
        threshold: Option<f64>,
    ) -> Result<crate::matrices::RunComparison, EvalError> {
        let baseline = self
            .get(baseline_run_id)
            .ok_or_else(|| EvalError::NotFound(baseline_run_id.to_string()))?;
        let candidate = self
            .get(candidate_run_id)
            .ok_or_else(|| EvalError::NotFound(candidate_run_id.to_string()))?;

        let thresh = threshold.unwrap_or(0.05);
        let mut deltas = Vec::new();
        let mut regressions = Vec::new();
        let mut improvements = Vec::new();

        // Higher is better
        for (name, lower_is_better) in [
            ("task_success_rate", false),
            ("policy_pass_rate", false),
            ("retrieval_hit_at_k", false),
            ("citation_coverage", false),
            ("source_diversity", false),
            ("latency_p50_ms", true),
            ("latency_p99_ms", true),
            ("retrieval_latency_ms", true),
            ("cost_per_run", true),
            ("retrieval_cost", true),
        ] {
            let bv = eval_metric_by_name(&baseline.metrics, name);
            let cv = eval_metric_by_name(&candidate.metrics, name);

            if let (Some(b), Some(c)) = (bv, cv) {
                if b.abs() < f64::EPSILON {
                    continue;
                }
                let delta_ratio = if lower_is_better {
                    (b - c) / b.abs()
                } else {
                    (c - b) / b.abs()
                };

                let is_regression = delta_ratio < -thresh;
                let is_improvement = delta_ratio > thresh;

                deltas.push(crate::matrices::MetricDelta {
                    metric: name.to_owned(),
                    baseline_value: b,
                    candidate_value: c,
                    delta: delta_ratio,
                    is_regression,
                });

                if is_regression {
                    regressions.push(name.to_owned());
                } else if is_improvement {
                    improvements.push(name.to_owned());
                }
            }
        }

        let passed = regressions.is_empty();

        Ok(crate::matrices::RunComparison {
            baseline_run_id: baseline_run_id.as_str().to_owned(),
            candidate_run_id: candidate_run_id.as_str().to_owned(),
            deltas,
            regressions,
            improvements,
            passed,
        })
    }

    // ── RFC 004 Gap 4: Model × eval suite matrix ─────────────────────────

    /// Build a model × eval_suite matrix for a project.
    ///
    /// Groups completed eval runs by `evaluator_type` (as eval suite) and
    /// provider binding / model association. Returns a matrix that makes it
    /// easy to compare models across evaluation suites.
    pub fn build_model_eval_matrix(
        &self,
        project_id: &ProjectId,
    ) -> crate::matrices::ModelEvalMatrix {
        let state = self.state.lock().unwrap();

        let completed_runs: Vec<&EvalRun> = state
            .runs
            .values()
            .filter(|r| r.project_id == *project_id && r.status == EvalRunStatus::Completed)
            .collect();

        let mut model_ids_set = std::collections::BTreeSet::new();
        let mut eval_suites_set = std::collections::BTreeSet::new();
        let mut cells = Vec::new();

        for run in &completed_runs {
            // Use prompt_version_id as model identifier if available,
            // otherwise fall back to evaluator_type as model proxy.
            let model_id = run
                .prompt_version_id
                .as_ref()
                .map(|v| v.as_str().to_owned())
                .unwrap_or_else(|| run.evaluator_type.clone());
            let eval_suite = run.evaluator_type.clone();

            model_ids_set.insert(model_id.clone());
            eval_suites_set.insert(eval_suite.clone());

            cells.push(crate::matrices::ModelEvalCell {
                model_id,
                eval_suite,
                eval_run_id: run.eval_run_id.as_str().to_owned(),
                metrics: run.metrics.clone(),
            });
        }

        crate::matrices::ModelEvalMatrix {
            model_ids: model_ids_set.into_iter().collect(),
            eval_suites: eval_suites_set.into_iter().collect(),
            cells,
        }
    }

    /// Stub: returns an async provider routing matrix.
    pub async fn build_provider_routing_matrix(
        &self,
        _tenant_id: &cairn_domain::TenantId,
    ) -> Result<crate::matrices::ProviderRoutingMatrix, EvalError> {
        Ok(crate::matrices::ProviderRoutingMatrix { rows: vec![] })
    }

    /// Returns trend points for a prompt asset metric, ordered by creation time.
    pub fn get_trend(
        &self,
        _tenant_id: &str,
        asset_id: &cairn_domain::PromptAssetId,
        metric: String,
        _days: u32,
    ) -> Result<Vec<EvalTrendPoint>, EvalError> {
        let state = self.state.lock().unwrap();
        let mut runs: Vec<&EvalRun> = state
            .runs
            .values()
            .filter(|r| {
                r.prompt_asset_id.as_ref() == Some(asset_id) && r.status == EvalRunStatus::Completed
            })
            .collect();
        // Sort by created_at, then by eval_run_id as a stable tiebreaker
        // (HashMap iteration order is non-deterministic).
        runs.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.eval_run_id.as_str().cmp(b.eval_run_id.as_str()))
        });
        let points = runs
            .into_iter()
            .map(|r| {
                let value = match metric.as_str() {
                    "task_success_rate" => r.metrics.task_success_rate.unwrap_or(0.0),
                    "latency_p50_ms" => r.metrics.latency_p50_ms.map(|v| v as f64).unwrap_or(0.0),
                    "cost_per_run" => r.metrics.cost_per_run.unwrap_or(0.0),
                    _ => 0.0,
                };
                EvalTrendPoint {
                    eval_run_id: r.eval_run_id.as_str().to_owned(),
                    day: r.created_at.to_string(),
                    value,
                }
            })
            .collect();
        Ok(points)
    }

    /// Generates an eval summary report for a prompt asset.
    pub fn generate_report(
        &self,
        _tenant_id: &str,
        asset_id: &cairn_domain::PromptAssetId,
    ) -> EvalReport {
        let state = self.state.lock().unwrap();
        let mut runs: Vec<&EvalRun> = state
            .runs
            .values()
            .filter(|r| {
                r.prompt_asset_id.as_ref() == Some(asset_id) && r.status == EvalRunStatus::Completed
            })
            .collect();
        if runs.is_empty() {
            return EvalReport {
                asset_id: asset_id.as_str().to_owned(),
                total_runs: 0,
                best_run_id: String::new(),
                worst_run_id: String::new(),
                trend_direction: "no_data".to_owned(),
                summary: String::new(),
            };
        }
        // Sort by task_success_rate to find best/worst
        runs.sort_by(|a, b| {
            let a_s = a.metrics.task_success_rate.unwrap_or(0.0);
            let b_s = b.metrics.task_success_rate.unwrap_or(0.0);
            b_s.partial_cmp(&a_s).unwrap_or(std::cmp::Ordering::Equal)
        });
        let best_run_id = runs
            .first()
            .map(|r| r.eval_run_id.as_str().to_owned())
            .unwrap_or_default();
        let worst_run_id = runs
            .last()
            .map(|r| r.eval_run_id.as_str().to_owned())
            .unwrap_or_default();

        // Compute trend direction from chronologically sorted scores
        let mut by_time = state
            .runs
            .values()
            .filter(|r| {
                r.prompt_asset_id.as_ref() == Some(asset_id) && r.status == EvalRunStatus::Completed
            })
            .collect::<Vec<_>>();
        by_time.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.eval_run_id.as_str().cmp(b.eval_run_id.as_str()))
        });
        let scores: Vec<f64> = by_time
            .iter()
            .filter_map(|r| r.metrics.task_success_rate)
            .collect();
        let trend_direction = if scores.len() >= 2 {
            let first = scores[0];
            let last = *scores.last().unwrap();
            if last > first {
                "improving"
            } else if last < first {
                "declining"
            } else {
                "stable"
            }
        } else {
            "stable"
        };

        EvalReport {
            asset_id: asset_id.as_str().to_owned(),
            total_runs: by_time.len(),
            best_run_id,
            worst_run_id,
            trend_direction: trend_direction.to_owned(),
            summary: format!("{} completed eval runs", by_time.len()),
        }
    }
}

/// A single data point in a trend series.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EvalTrendPoint {
    pub eval_run_id: String,
    pub day: String,
    pub value: f64,
}

/// Summary report for an eval asset.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EvalReport {
    pub asset_id: String,
    pub total_runs: usize,
    pub best_run_id: String,
    pub worst_run_id: String,
    pub trend_direction: String,
    pub summary: String,
}

impl Default for EvalRunService {
    fn default() -> Self {
        Self::new()
    }
}

// ── Memory diagnostics source contract ────────────────────────────────────

/// Snapshot of source quality metrics for one knowledge source.
#[derive(Clone, Debug)]
pub struct SourceQualitySnapshot {
    pub source_id: cairn_domain::SourceId,
    pub total_chunks: u64,
    pub credibility_score: Option<f64>,
    pub retrieval_count: u64,
    pub query_hit_rate: f64,
    pub error_rate: f64,
    pub last_ingested_at: Option<u64>,
}

/// Trait for adapting a memory diagnostics backend to the eval service.
#[async_trait::async_trait]
pub trait MemoryDiagnosticsSource: Send + Sync {
    async fn list_source_quality(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
    ) -> Result<Vec<SourceQualitySnapshot>, String>;
}

/// Extract a named metric value from EvalMetrics.
fn eval_metric_by_name(m: &EvalMetrics, name: &str) -> Option<f64> {
    match name {
        "task_success_rate" => m.task_success_rate,
        "policy_pass_rate" => m.policy_pass_rate,
        "retrieval_hit_at_k" => m.retrieval_hit_at_k,
        "citation_coverage" => m.citation_coverage,
        "source_diversity" => m.source_diversity,
        "latency_p50_ms" => m.latency_p50_ms.map(|v| v as f64),
        "latency_p99_ms" => m.latency_p99_ms.map(|v| v as f64),
        "retrieval_latency_ms" => m.retrieval_latency_ms.map(|v| v as f64),
        "cost_per_run" => m.cost_per_run,
        "retrieval_cost" => m.retrieval_cost,
        _ => None,
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_run_lifecycle() {
        let svc = EvalRunService::new();

        let run = svc.create_run(
            EvalRunId::new("eval_1"),
            ProjectId::new("proj_1"),
            EvalSubjectKind::PromptRelease,
            "auto_scorer".to_owned(),
            Some(PromptAssetId::new("prompt_planner")),
            Some(PromptVersionId::new("pv_1")),
            Some(PromptReleaseId::new("rel_1")),
            None,
        );
        assert_eq!(run.status, EvalRunStatus::Pending);

        let run = svc.start_run(&EvalRunId::new("eval_1")).unwrap();
        assert_eq!(run.status, EvalRunStatus::Running);

        let metrics = EvalMetrics {
            task_success_rate: Some(0.92),
            latency_p50_ms: Some(150),
            cost_per_run: Some(0.003),
            ..Default::default()
        };

        let run = svc
            .complete_run(&EvalRunId::new("eval_1"), metrics, Some(0.15))
            .unwrap();
        assert_eq!(run.status, EvalRunStatus::Completed);
        assert_eq!(run.metrics.task_success_rate, Some(0.92));
        assert!(run.completed_at.is_some());
    }

    #[test]
    fn scorecard_aggregates_completed_runs() {
        let svc = EvalRunService::new();
        let project_id = ProjectId::new("proj_1");
        let asset_id = PromptAssetId::new("prompt_planner");

        // Run for release A
        svc.create_run(
            EvalRunId::new("eval_a"),
            project_id.clone(),
            EvalSubjectKind::PromptRelease,
            "auto".to_owned(),
            Some(asset_id.clone()),
            Some(PromptVersionId::new("pv_1")),
            Some(PromptReleaseId::new("rel_a")),
            None,
        );
        svc.start_run(&EvalRunId::new("eval_a")).unwrap();
        svc.complete_run(
            &EvalRunId::new("eval_a"),
            EvalMetrics {
                task_success_rate: Some(0.85),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        // Run for release B
        svc.create_run(
            EvalRunId::new("eval_b"),
            project_id.clone(),
            EvalSubjectKind::PromptRelease,
            "auto".to_owned(),
            Some(asset_id.clone()),
            Some(PromptVersionId::new("pv_2")),
            Some(PromptReleaseId::new("rel_b")),
            None,
        );
        svc.start_run(&EvalRunId::new("eval_b")).unwrap();
        svc.complete_run(
            &EvalRunId::new("eval_b"),
            EvalMetrics {
                task_success_rate: Some(0.93),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        let scorecard = svc.build_scorecard(&project_id, &asset_id);
        assert_eq!(scorecard.entries.len(), 2);

        // Find the better release
        let best = scorecard
            .entries
            .iter()
            .max_by(|a, b| {
                a.metrics
                    .task_success_rate
                    .partial_cmp(&b.metrics.task_success_rate)
                    .unwrap()
            })
            .unwrap();
        assert_eq!(best.prompt_release_id, PromptReleaseId::new("rel_b"));
    }

    #[test]
    fn cannot_complete_pending_run() {
        let svc = EvalRunService::new();
        svc.create_run(
            EvalRunId::new("eval_1"),
            ProjectId::new("proj_1"),
            EvalSubjectKind::PromptRelease,
            "auto".to_owned(),
            None,
            None,
            None,
            None,
        );

        let result = svc.complete_run(&EvalRunId::new("eval_1"), EvalMetrics::default(), None);
        assert!(result.is_err());
    }

    // ── RFC 004 Gap 2: score_with_rubric tests ──────────────────────────

    #[test]
    fn score_with_rubric_computes_weighted_scores() {
        let svc = EvalRunService::new();
        svc.create_run(
            EvalRunId::new("r1"),
            ProjectId::new("p1"),
            EvalSubjectKind::PromptRelease,
            "auto".to_owned(),
            None,
            None,
            None,
            None,
        );
        svc.start_run(&EvalRunId::new("r1")).unwrap();
        svc.complete_run(
            &EvalRunId::new("r1"),
            EvalMetrics {
                task_success_rate: Some(0.9),
                policy_pass_rate: Some(0.8),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        let rubric = vec![
            crate::matrices::RubricDimensionDef {
                dimension: "accuracy".into(),
                weight: 0.7,
                criteria: "task_success_rate".into(),
            },
            crate::matrices::RubricDimensionDef {
                dimension: "compliance".into(),
                weight: 0.3,
                criteria: "policy_pass_rate".into(),
            },
        ];

        let result = svc
            .score_with_rubric(&EvalRunId::new("r1"), &rubric)
            .unwrap();
        assert_eq!(result.dimensions.len(), 2);
        assert!((result.dimensions[0].score - 0.9).abs() < 0.001);
        assert!((result.dimensions[1].score - 0.8).abs() < 0.001);
        // Weighted: (0.9*0.7 + 0.8*0.3) / (0.7+0.3) = 0.63+0.24 = 0.87
        assert!((result.overall_score - 0.87).abs() < 0.001);
    }

    #[test]
    fn score_with_rubric_missing_metric_scores_zero() {
        let svc = EvalRunService::new();
        svc.create_run(
            EvalRunId::new("r1"),
            ProjectId::new("p1"),
            EvalSubjectKind::PromptRelease,
            "auto".to_owned(),
            None,
            None,
            None,
            None,
        );
        svc.start_run(&EvalRunId::new("r1")).unwrap();
        svc.complete_run(&EvalRunId::new("r1"), EvalMetrics::default(), None)
            .unwrap();

        let rubric = vec![crate::matrices::RubricDimensionDef {
            dimension: "accuracy".into(),
            weight: 1.0,
            criteria: "task_success_rate".into(),
        }];

        let result = svc
            .score_with_rubric(&EvalRunId::new("r1"), &rubric)
            .unwrap();
        assert!((result.overall_score - 0.0).abs() < 0.001);
    }

    // ── RFC 004 Gap 3: compare_runs tests ────────────────────────────────

    fn create_completed_run(svc: &EvalRunService, id: &str, metrics: EvalMetrics) {
        svc.create_run(
            EvalRunId::new(id),
            ProjectId::new("p1"),
            EvalSubjectKind::PromptRelease,
            "auto".to_owned(),
            None,
            None,
            None,
            None,
        );
        svc.start_run(&EvalRunId::new(id)).unwrap();
        svc.complete_run(&EvalRunId::new(id), metrics, None)
            .unwrap();
    }

    #[test]
    fn compare_runs_detects_regression() {
        let svc = EvalRunService::new();
        create_completed_run(
            &svc,
            "baseline",
            EvalMetrics {
                task_success_rate: Some(0.9),
                latency_p50_ms: Some(100),
                ..Default::default()
            },
        );
        create_completed_run(
            &svc,
            "candidate",
            EvalMetrics {
                task_success_rate: Some(0.7), // regression: -22%
                latency_p50_ms: Some(150),    // regression: +50% latency
                ..Default::default()
            },
        );

        let cmp = svc
            .compare_runs(
                &EvalRunId::new("baseline"),
                &EvalRunId::new("candidate"),
                Some(0.05),
            )
            .unwrap();

        assert!(!cmp.passed);
        assert!(cmp.regressions.contains(&"task_success_rate".to_owned()));
        assert!(cmp.regressions.contains(&"latency_p50_ms".to_owned()));
    }

    #[test]
    fn compare_runs_detects_improvement() {
        let svc = EvalRunService::new();
        create_completed_run(
            &svc,
            "baseline",
            EvalMetrics {
                task_success_rate: Some(0.7),
                ..Default::default()
            },
        );
        create_completed_run(
            &svc,
            "candidate",
            EvalMetrics {
                task_success_rate: Some(0.9), // improvement: +28%
                ..Default::default()
            },
        );

        let cmp = svc
            .compare_runs(
                &EvalRunId::new("baseline"),
                &EvalRunId::new("candidate"),
                None,
            )
            .unwrap();

        assert!(cmp.passed);
        assert!(cmp.improvements.contains(&"task_success_rate".to_owned()));
        assert!(cmp.regressions.is_empty());
    }

    #[test]
    fn compare_runs_equal_metrics_passes() {
        let svc = EvalRunService::new();
        let metrics = EvalMetrics {
            task_success_rate: Some(0.85),
            ..Default::default()
        };
        create_completed_run(&svc, "a", metrics.clone());
        create_completed_run(&svc, "b", metrics);

        let cmp = svc
            .compare_runs(&EvalRunId::new("a"), &EvalRunId::new("b"), None)
            .unwrap();
        assert!(cmp.passed);
        assert!(cmp.regressions.is_empty());
        assert!(cmp.improvements.is_empty());
    }

    // ── RFC 004 Gap 4: model eval matrix tests ──────────────────────────

    #[test]
    fn build_model_eval_matrix_groups_by_model_and_suite() {
        let svc = EvalRunService::new();
        let project = ProjectId::new("p1");

        // Model A evaluated by suite "accuracy"
        svc.create_run(
            EvalRunId::new("r1"),
            project.clone(),
            EvalSubjectKind::PromptRelease,
            "accuracy".to_owned(),
            None,
            Some(PromptVersionId::new("model_a")),
            None,
            None,
        );
        svc.start_run(&EvalRunId::new("r1")).unwrap();
        svc.complete_run(
            &EvalRunId::new("r1"),
            EvalMetrics {
                task_success_rate: Some(0.9),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        // Model B evaluated by suite "accuracy"
        svc.create_run(
            EvalRunId::new("r2"),
            project.clone(),
            EvalSubjectKind::PromptRelease,
            "accuracy".to_owned(),
            None,
            Some(PromptVersionId::new("model_b")),
            None,
            None,
        );
        svc.start_run(&EvalRunId::new("r2")).unwrap();
        svc.complete_run(
            &EvalRunId::new("r2"),
            EvalMetrics {
                task_success_rate: Some(0.75),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        // Model A evaluated by suite "latency"
        svc.create_run(
            EvalRunId::new("r3"),
            project.clone(),
            EvalSubjectKind::PromptRelease,
            "latency".to_owned(),
            None,
            Some(PromptVersionId::new("model_a")),
            None,
            None,
        );
        svc.start_run(&EvalRunId::new("r3")).unwrap();
        svc.complete_run(
            &EvalRunId::new("r3"),
            EvalMetrics {
                latency_p50_ms: Some(50),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        let matrix = svc.build_model_eval_matrix(&project);
        assert_eq!(matrix.model_ids, vec!["model_a", "model_b"]);
        assert_eq!(matrix.eval_suites, vec!["accuracy", "latency"]);
        assert_eq!(matrix.cells.len(), 3);

        // Look up model_a / accuracy cell
        let cell = matrix.cell("model_a", "accuracy").unwrap();
        assert_eq!(cell.metrics.task_success_rate, Some(0.9));

        // model_b / latency should be absent
        assert!(matrix.cell("model_b", "latency").is_none());
    }

    #[test]
    fn model_eval_matrix_empty_project() {
        let svc = EvalRunService::new();
        let matrix = svc.build_model_eval_matrix(&ProjectId::new("empty"));
        assert!(matrix.model_ids.is_empty());
        assert!(matrix.cells.is_empty());
    }
}
