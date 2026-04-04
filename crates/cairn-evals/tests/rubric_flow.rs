use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{EvalRunId, ProjectId, RubricDimension, RubricScoringFn, TenantId};
use cairn_evals::{
    EvalDatasetServiceImpl, EvalRubricServiceImpl, EvalRunService, EvalSubjectKind,
    PluginDimensionScore, PluginRubricScorer,
};

use cairn_evals::services::rubric_impl::EvalRubricError;

#[tokio::test]
async fn rubric_exact_match_scores_eval_run() {
    let datasets = Arc::new(EvalDatasetServiceImpl::new());
    let runs = Arc::new(EvalRunService::new());
    let rubrics = EvalRubricServiceImpl::new(runs.clone(), datasets.clone());

    let dataset = datasets.create(
        TenantId::new("tenant_rubric"),
        "Rubric Dataset".to_owned(),
        EvalSubjectKind::PromptRelease,
    );
    datasets
        .add_entry(
            &dataset.dataset_id,
            serde_json::json!({ "prompt": "alpha" }),
            Some(serde_json::json!({ "answer": "one" })),
            vec!["smoke".to_owned()],
        )
        .unwrap();
    datasets
        .add_entry(
            &dataset.dataset_id,
            serde_json::json!({ "prompt": "beta" }),
            Some(serde_json::json!({ "answer": "two" })),
            vec!["smoke".to_owned()],
        )
        .unwrap();

    let run = runs.create_run(
        EvalRunId::new("eval_rubric_1"),
        ProjectId::new("project_rubric"),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        None,
        None,
        None,
        None,
        Some(dataset.dataset_id.clone()),
    );

    let rubric = rubrics.create(
        TenantId::new("tenant_rubric"),
        "Exact Match".to_owned(),
        vec![RubricDimension {
            name: "answer_exact".to_owned(),
            weight: 1.0,
            scoring_fn: RubricScoringFn::ExactMatch,
            threshold: None,
            plugin_id: None,
        }],
    );

    let result = rubrics
        .score_against_rubric(
            &run.eval_run_id,
            &rubric.rubric_id,
            &[
                serde_json::json!({ "answer": "one" }),
                serde_json::json!({ "answer": "wrong" }),
            ],
        )
        .await
        .unwrap();

    assert_eq!(run.dataset_id.as_deref(), Some(dataset.dataset_id.as_str()));
    assert_eq!(result.run_id, run.eval_run_id.to_string());
    assert_eq!(result.rubric_id, rubric.rubric_id);
    assert_eq!(result.dimension_scores.len(), 1);
    assert_eq!(result.dimension_scores[0].0, "answer_exact");
    assert!((result.dimension_scores[0].1 - 0.5).abs() < f64::EPSILON);
    assert!((result.overall - 0.5).abs() < f64::EPSILON);
}

struct ExactMatchPluginScorer;

#[async_trait]
impl PluginRubricScorer for ExactMatchPluginScorer {
    async fn score(
        &self,
        _plugin_id: &str,
        _input: &serde_json::Value,
        expected_output: Option<&serde_json::Value>,
        actual_output: &serde_json::Value,
    ) -> Result<PluginDimensionScore, EvalRubricError> {
        let score = if expected_output == Some(actual_output) {
            1.0
        } else {
            0.0
        };
        Ok(PluginDimensionScore {
            score,
            passed: score >= 1.0,
            feedback: Some("mock plugin scorer".to_owned()),
        })
    }
}

#[tokio::test]
async fn rubric_plugin_scoring_scores_exact_match() {
    let datasets = Arc::new(EvalDatasetServiceImpl::new());
    let runs = Arc::new(EvalRunService::new());
    let rubrics = EvalRubricServiceImpl::with_plugin_scorer(
        runs.clone(),
        datasets.clone(),
        Arc::new(ExactMatchPluginScorer),
    );

    let dataset = datasets.create(
        TenantId::new("tenant_rubric_plugin"),
        "Plugin Rubric Dataset".to_owned(),
        EvalSubjectKind::PromptRelease,
    );
    datasets
        .add_entry(
            &dataset.dataset_id,
            serde_json::json!({ "prompt": "exact" }),
            Some(serde_json::json!({ "answer": "match" })),
            vec!["plugin".to_owned()],
        )
        .unwrap();

    let run = runs.create_run(
        EvalRunId::new("eval_rubric_plugin_1"),
        ProjectId::new("project_rubric_plugin"),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        None,
        None,
        None,
        None,
        Some(dataset.dataset_id.clone()),
    );

    let rubric = rubrics.create(
        TenantId::new("tenant_rubric_plugin"),
        "Plugin Score".to_owned(),
        vec![RubricDimension {
            name: "plugin_match".to_owned(),
            weight: 1.0,
            scoring_fn: RubricScoringFn::Plugin,
            threshold: None,
            plugin_id: Some("com.test.eval-scorer".to_owned()),
        }],
    );

    let result = rubrics
        .score_against_rubric(
            &run.eval_run_id,
            &rubric.rubric_id,
            &[serde_json::json!({ "answer": "match" })],
        )
        .await
        .unwrap();

    assert_eq!(result.dimension_scores.len(), 1);
    assert_eq!(result.dimension_scores[0].0, "plugin_match");
    assert!((result.dimension_scores[0].1 - 1.0).abs() < f64::EPSILON);
    assert!((result.overall - 1.0).abs() < f64::EPSILON);
}
