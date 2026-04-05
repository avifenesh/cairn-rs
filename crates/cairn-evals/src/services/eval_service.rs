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
    pub fn set_dataset_id(&self, eval_run_id: &EvalRunId, dataset_id: String) -> Result<(), EvalError> {
        let mut state = self.state.lock().unwrap();
        let run = state.runs.get_mut(eval_run_id.as_str())
            .ok_or_else(|| EvalError::NotFound(eval_run_id.to_string()))?;
        run.dataset_id = Some(dataset_id);
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
            b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
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
    pub fn export_runs(
        &self,
        project_id: &ProjectId,
        limit: usize,
    ) -> Vec<EvalRun> {
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
                r.prompt_asset_id.as_ref() == Some(asset_id)
                    && r.status == EvalRunStatus::Completed
            })
            .collect();
        runs.sort_by_key(|r| r.created_at);
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
                r.prompt_asset_id.as_ref() == Some(asset_id)
                    && r.status == EvalRunStatus::Completed
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
        let best_run_id = runs.first().map(|r| r.eval_run_id.as_str().to_owned()).unwrap_or_default();
        let worst_run_id = runs.last().map(|r| r.eval_run_id.as_str().to_owned()).unwrap_or_default();

        // Compute trend direction from chronologically sorted scores
        let mut by_time = state
            .runs
            .values()
            .filter(|r| {
                r.prompt_asset_id.as_ref() == Some(asset_id)
                    && r.status == EvalRunStatus::Completed
            })
            .collect::<Vec<_>>();
        by_time.sort_by_key(|r| r.created_at);
        let scores: Vec<f64> = by_time
            .iter()
            .filter_map(|r| r.metrics.task_success_rate)
            .collect();
        let trend_direction = if scores.len() >= 2 {
            let first = scores[0];
            let last = *scores.last().unwrap();
            if last > first { "improving" }
            else if last < first { "declining" }
            else { "stable" }
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
}
