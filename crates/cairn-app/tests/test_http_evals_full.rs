//! Issue #138 — full eval-run create contract.
//!
//! The UI's `EvalsPage` "New Eval Run" form collects dataset / rubric /
//! baseline / prompt-release references and submits them to
//! `POST /v1/evals/runs`. Before this fix the endpoint accepted only
//! `{eval_run_id, subject_kind, evaluator_type}` — the linked artifacts were
//! silently ignored and the run created was a no-op stub.
//!
//! This test locks the real contract:
//!   1. GET /v1/evals/datasets + rubrics + baselines return the list shape
//!      the UI consumes (`{items, has_more}`).
//!   2. POST /v1/evals/runs validates dataset_id / rubric_id / baseline_id
//!      exist in tenant state — a dangling id must 404, not silently ignore.
//!   3. The created run round-trips: GET /v1/evals/runs/:id finds it.
//!   4. GET /v1/evals/compare?run_ids=… returns backend-authoritative metric
//!      rows (used by the Results link on each run).
//!
//! If any of these break, the EvalsPage form regresses to the stub state.

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn eval_run_full_contract_roundtrip() {
    let h = LiveHarness::setup().await;
    let base = &h.base_url;
    let tenant = &h.tenant;

    // ── 1. Create dataset + rubric + baseline so the pickers have real ids.

    let res = h
        .client()
        .post(format!("{base}/v1/evals/datasets"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "name":         "issue-138 dataset",
            "subject_kind": "prompt_release",
        }))
        .send()
        .await
        .expect("create dataset reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "create dataset: {}",
        res.text().await.unwrap_or_default()
    );
    let dataset: Value = res.json().await.expect("dataset json");
    let dataset_id = dataset
        .get("dataset_id")
        .and_then(|v| v.as_str())
        .expect("dataset_id in response")
        .to_owned();

    let res = h
        .client()
        .post(format!("{base}/v1/evals/rubrics"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":  tenant,
            "name":       "issue-138 rubric",
            "dimensions": [],
        }))
        .send()
        .await
        .expect("create rubric reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "create rubric: {}",
        res.text().await.unwrap_or_default()
    );
    let rubric: Value = res.json().await.expect("rubric json");
    let rubric_id = rubric
        .get("rubric_id")
        .and_then(|v| v.as_str())
        .expect("rubric_id in response")
        .to_owned();

    let res = h
        .client()
        .post(format!("{base}/v1/evals/baselines"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":       tenant,
            "name":            "issue-138 baseline",
            "prompt_asset_id": "asset-issue-138",
            "metrics":         {},
        }))
        .send()
        .await
        .expect("create baseline reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "create baseline: {}",
        res.text().await.unwrap_or_default()
    );
    let baseline: Value = res.json().await.expect("baseline json");
    let baseline_id = baseline
        .get("baseline_id")
        .and_then(|v| v.as_str())
        .expect("baseline_id in response")
        .to_owned();

    // ── 2. Lock the list contract — the UI's pickers consume these shapes.

    for path in ["datasets", "rubrics", "baselines"] {
        let url = format!("{base}/v1/evals/{path}?tenant_id={tenant}");
        let res = h
            .client()
            .get(&url)
            .bearer_auth(&h.admin_token)
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {url} reaches server: {e}"));
        assert_eq!(res.status().as_u16(), 200, "GET {url}");
        let body: Value = res.json().await.expect("list json");
        assert!(
            body.get("items").and_then(|v| v.as_array()).is_some(),
            "GET {url} missing items[] array: {body}",
        );
        assert!(
            body.get("has_more").is_some() || body.get("hasMore").is_some(),
            "GET {url} missing has_more/hasMore field: {body}",
        );
    }

    // ── 3. Dangling id must 404 — not silently ignore.

    let eval_run_id = "eval_issue138_dangling";
    let res = h
        .client()
        .post(format!("{base}/v1/evals/runs"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":      tenant,
            "workspace_id":   h.workspace,
            "project_id":     h.project,
            "eval_run_id":    eval_run_id,
            "subject_kind":   "prompt_release",
            "evaluator_type": "accuracy",
            "rubric_id":      "rubric_does_not_exist",
        }))
        .send()
        .await
        .expect("create run w/ dangling rubric reaches server");
    assert_eq!(
        res.status().as_u16(),
        404,
        "dangling rubric_id must 404, got {}: {}",
        res.status(),
        res.text().await.unwrap_or_default(),
    );

    // ── 4. Real eval-run create with all linked artifacts.

    let eval_run_id = "eval_issue138_ok";
    let res = h
        .client()
        .post(format!("{base}/v1/evals/runs"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":      tenant,
            "workspace_id":   h.workspace,
            "project_id":     h.project,
            "eval_run_id":    eval_run_id,
            "subject_kind":   "prompt_release",
            "evaluator_type": "accuracy",
            "dataset_id":     dataset_id,
            "rubric_id":      rubric_id,
            "baseline_id":    baseline_id,
        }))
        .send()
        .await
        .expect("create run reaches server");
    assert_eq!(
        res.status().as_u16(),
        201,
        "create run: {}",
        res.text().await.unwrap_or_default(),
    );
    let run: Value = res.json().await.expect("run json");
    assert_eq!(
        run.get("eval_run_id").and_then(|v| v.as_str()),
        Some(eval_run_id),
    );
    // dataset_id / rubric_id / baseline_id are all persisted on the EvalRun
    // record (see cairn-evals EvalRun + issue #223).
    assert_eq!(
        run.get("dataset_id").and_then(|v| v.as_str()),
        Some(dataset_id.as_str()),
        "created run must echo dataset_id: {run}",
    );
    assert_eq!(
        run.get("rubric_id").and_then(|v| v.as_str()),
        Some(rubric_id.as_str()),
        "created run must echo rubric_id: {run}",
    );
    assert_eq!(
        run.get("baseline_id").and_then(|v| v.as_str()),
        Some(baseline_id.as_str()),
        "created run must echo baseline_id: {run}",
    );

    // ── 5. Round-trip via GET /v1/evals/runs/:id.

    let res = h
        .client()
        .get(format!("{base}/v1/evals/runs/{eval_run_id}"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("get run reaches server");
    assert_eq!(res.status().as_u16(), 200, "GET run");
    let got: Value = res.json().await.expect("get run json");
    assert_eq!(
        got.get("eval_run_id").and_then(|v| v.as_str()),
        Some(eval_run_id),
    );
    // GET must echo all three linkage ids (issue #223).
    assert_eq!(
        got.get("dataset_id").and_then(|v| v.as_str()),
        Some(dataset_id.as_str()),
        "GET run dataset_id: {got}",
    );
    assert_eq!(
        got.get("rubric_id").and_then(|v| v.as_str()),
        Some(rubric_id.as_str()),
        "GET run rubric_id: {got}",
    );
    assert_eq!(
        got.get("baseline_id").and_then(|v| v.as_str()),
        Some(baseline_id.as_str()),
        "GET run baseline_id: {got}",
    );

    // ── 6. Compare endpoint (Results link backend) returns metric rows.

    let res = h
        .client()
        .get(format!("{base}/v1/evals/compare?run_ids={eval_run_id}"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("compare reaches server");
    assert_eq!(
        res.status().as_u16(),
        200,
        "compare: {}",
        res.text().await.unwrap_or_default()
    );
    let compare: Value = res.json().await.expect("compare json");
    let run_ids = compare
        .get("run_ids")
        .and_then(|v| v.as_array())
        .expect("compare.run_ids[]");
    assert_eq!(run_ids.len(), 1);
    assert!(
        compare.get("rows").and_then(|v| v.as_array()).is_some(),
        "compare.rows[] present: {compare}",
    );
}

/// Issue #220 — dataset_id must survive a process restart via the event log.
///
/// Before this fix, `POST /v1/evals/runs` stored the dataset binding only in
/// the in-memory `EvalsService`. `replay_evals` reconstructed runs from
/// `EvalRunStarted` events, which did not carry `dataset_id`, so the
/// dataset/run linkage silently disappeared on reboot.
///
/// Contract: the dataset_id is persisted on `EvalRunStarted` (serde-default
/// for backward compatibility with pre-#220 event logs) and restored on
/// replay. After a sigkill+restart, `GET /v1/evals/runs/:id` must still echo
/// the dataset_id that was submitted at create time.
#[tokio::test]
async fn eval_dataset_id_survives_restart() {
    let mut h = LiveHarness::setup_with_sqlite().await;
    let tenant = h.tenant.clone();
    let workspace = h.workspace.clone();
    let project = h.project.clone();

    // Create a dataset so the run has a real binding to attach.
    let base = h.base_url.clone();
    let res = h
        .client()
        .post(format!("{base}/v1/evals/datasets"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    tenant,
            "name":         "issue-220 dataset",
            "subject_kind": "prompt_release",
        }))
        .send()
        .await
        .expect("create dataset reaches server");
    assert_eq!(res.status().as_u16(), 201, "create dataset");
    let dataset: Value = res.json().await.expect("dataset json");
    let dataset_id = dataset
        .get("dataset_id")
        .and_then(|v| v.as_str())
        .expect("dataset_id")
        .to_owned();

    // Create an eval run bound to the dataset.
    let eval_run_id = "eval_issue220_persist";
    let res = h
        .client()
        .post(format!("{base}/v1/evals/runs"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":      tenant,
            "workspace_id":   workspace,
            "project_id":     project,
            "eval_run_id":    eval_run_id,
            "subject_kind":   "prompt_release",
            "evaluator_type": "accuracy",
            "dataset_id":     dataset_id,
        }))
        .send()
        .await
        .expect("create run reaches server");
    assert_eq!(res.status().as_u16(), 201, "create run");

    // Sanity: pre-restart echoes dataset_id.
    let got: Value = h
        .client()
        .get(format!("{base}/v1/evals/runs/{eval_run_id}"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("pre-restart get run")
        .json()
        .await
        .expect("pre-restart get run json");
    assert_eq!(
        got.get("dataset_id").and_then(|v| v.as_str()),
        Some(dataset_id.as_str()),
        "pre-restart: dataset_id must be attached: {got}",
    );

    // Sigkill the subprocess and bring up a fresh one against the same
    // event log. replay_evals() must restore the dataset binding.
    h.sigkill_and_restart()
        .await
        .expect("sigkill+restart succeeds");
    let base = h.base_url.clone();

    let got: Value = h
        .client()
        .get(format!("{base}/v1/evals/runs/{eval_run_id}"))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("post-restart get run")
        .json()
        .await
        .expect("post-restart get run json");
    assert_eq!(
        got.get("dataset_id").and_then(|v| v.as_str()),
        Some(dataset_id.as_str()),
        "post-restart: dataset_id must survive replay, got: {got}",
    );
}

/// Issue #223 — `rubric_id` + `baseline_id` (in addition to `dataset_id`)
/// must survive a process restart via the event log. Superset of
/// `eval_dataset_id_survives_restart`; asserts all three linkages at once.
#[tokio::test]
async fn eval_run_linkage_survives_restart() {
    let mut h = LiveHarness::setup_with_sqlite().await;
    let base = h.base_url.clone();
    let tenant = h.tenant.clone();

    // Seed a dataset + rubric + baseline.
    let ds: Value = h
        .client()
        .post(format!("{base}/v1/evals/datasets"))
        .bearer_auth(&h.admin_token)
        .json(&json!({"tenant_id": &tenant, "name": "ds-223", "subject_kind": "prompt_release"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dataset_id = ds["dataset_id"].as_str().unwrap().to_owned();

    let ru: Value = h
        .client()
        .post(format!("{base}/v1/evals/rubrics"))
        .bearer_auth(&h.admin_token)
        .json(&json!({"tenant_id": &tenant, "name": "ru-223", "dimensions": []}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let rubric_id = ru["rubric_id"].as_str().unwrap().to_owned();

    let bl: Value = h
        .client()
        .post(format!("{base}/v1/evals/baselines"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": &tenant,
            "name": "bl-223",
            "prompt_asset_id": "asset-223",
            "metrics": {},
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let baseline_id = bl["baseline_id"].as_str().unwrap().to_owned();

    let eval_run_id = "eval_223_replay";
    let res = h
        .client()
        .post(format!("{base}/v1/evals/runs"))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":      &tenant,
            "workspace_id":   &h.workspace,
            "project_id":     &h.project,
            "eval_run_id":    eval_run_id,
            "subject_kind":   "prompt_release",
            "evaluator_type": "accuracy",
            "dataset_id":     &dataset_id,
            "rubric_id":      &rubric_id,
            "baseline_id":    &baseline_id,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 201);

    // Restart — event-log replay is the only path back to EvalRun state.
    h.sigkill_and_restart()
        .await
        .expect("sigkill+restart must succeed");

    let res = h
        .client()
        .get(format!("{}/v1/evals/runs/{eval_run_id}", h.base_url))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status().as_u16(),
        200,
        "GET run after restart: {}",
        res.text().await.unwrap_or_default()
    );
    let got: Value = res.json().await.unwrap();
    assert_eq!(
        got.get("dataset_id").and_then(|v| v.as_str()),
        Some(dataset_id.as_str()),
        "dataset_id must survive restart: {got}",
    );
    assert_eq!(
        got.get("rubric_id").and_then(|v| v.as_str()),
        Some(rubric_id.as_str()),
        "rubric_id must survive restart: {got}",
    );
    assert_eq!(
        got.get("baseline_id").and_then(|v| v.as_str()),
        Some(baseline_id.as_str()),
        "baseline_id must survive restart: {got}",
    );
}
