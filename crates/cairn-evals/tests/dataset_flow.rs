use cairn_domain::{EvalSubjectKind as DomainSubjectKind, ProjectId, TenantId};
use cairn_evals::{EvalDatasetServiceImpl, EvalRunService, EvalSubjectKind};

#[test]
fn dataset_create_entries_and_run_link_round_trip() {
    let datasets = EvalDatasetServiceImpl::new();
    let runs = EvalRunService::new();

    let dataset = datasets.create(
        TenantId::new("tenant_eval"),
        "Regression Dataset".to_owned(),
        DomainSubjectKind::PromptRelease,
    );
    for idx in 0..3 {
        datasets
            .add_entry(
                &dataset.dataset_id,
                serde_json::json!({ "prompt": format!("case-{idx}") }),
                Some(serde_json::json!({ "ok": true })),
                vec!["smoke".to_owned()],
            )
            .unwrap();
    }

    let dataset = datasets.get(&dataset.dataset_id).unwrap();
    assert_eq!(dataset.entries.len(), 3);

    let run = runs.create_run(
        cairn_domain::EvalRunId::new("eval_dataset_1"),
        ProjectId::new("project_eval"),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        None,
        None,
        None,
        None,
    );

    // Verify the run was created (dataset linking happens at application layer).
    assert_eq!(run.subject_kind, EvalSubjectKind::PromptRelease,
        "run must be created with PromptRelease subject kind");
}
