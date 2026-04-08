//! eval_score — record an evaluation metric against a run.
//!
//! Writes an `OutcomeRecorded` event into the event log so the
//! evaluator-optimizer feedback loop can track per-run metrics and
//! calibrate agent confidence over time.
//!
//! ## Parameters
//! ```json
//! {
//!   "run_id":      "run_abc123",     // required
//!   "metric_name": "answer_quality", // required; used as agent_type label
//!   "score":       0.85,             // required; 0.0 – 1.0
//!   "passed":      true              // optional; default: score >= 0.5
//! }
//! ```
//!
//! ## Output
//! ```json
//! { "run_id": "run_abc123", "metric_name": "answer_quality", "score": 0.85, "recorded": true }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{
    events::{ActualOutcome, OutcomeRecorded},
    policy::ExecutionClass,
    EventEnvelope, EventId, EventSource, OutcomeId, ProjectKey, RunId, RuntimeEvent,
};
use cairn_store::EventLog;
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

/// Eval-score tool — appends an OutcomeRecorded event for a run.
pub struct EvalScoreTool {
    event_log: Arc<dyn EventLog + Send + Sync>,
}

impl EvalScoreTool {
    pub fn new(event_log: Arc<dyn EventLog + Send + Sync>) -> Self {
        Self { event_log }
    }
}

#[async_trait]
impl ToolHandler for EvalScoreTool {
    fn name(&self) -> &str {
        "eval_score"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    fn description(&self) -> &str {
        "Record an evaluation metric (score) against a run for quality tracking and confidence calibration."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["run_id", "metric_name", "score"],
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "The run to score."
                },
                "metric_name": {
                    "type": "string",
                    "description": "Name of the metric being recorded (e.g. 'answer_quality')."
                },
                "score": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "description": "Metric score in [0.0, 1.0]."
                },
                "passed": {
                    "type": "boolean",
                    "description": "Explicit pass/fail. Defaults to score >= 0.5 when omitted."
                }
            }
        })
    }

    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        // ── Validate ──────────────────────────────────────────────────────────
        let run_id_str =
            args.get("run_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "run_id".into(),
                    message: "required".into(),
                })?;
        if run_id_str.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "run_id".into(),
                message: "must not be empty".into(),
            });
        }

        let metric_name = args
            .get("metric_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "metric_name".into(),
                message: "required".into(),
            })?
            .to_owned();
        if metric_name.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "metric_name".into(),
                message: "must not be empty".into(),
            });
        }

        let score =
            args.get("score")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "score".into(),
                    message: "required numeric value 0.0–1.0".into(),
                })?;
        if !(0.0..=1.0).contains(&score) {
            return Err(ToolError::InvalidArgs {
                field: "score".into(),
                message: format!("must be in [0.0, 1.0], got {score}"),
            });
        }

        let passed = args
            .get("passed")
            .and_then(|v| v.as_bool())
            .unwrap_or(score >= 0.5);

        // ── Write OutcomeRecorded event ────────────────────────────────────────
        let now_ms = now_millis();
        let outcome_id = OutcomeId::new(format!("oc_{}_{}", run_id_str, now_ms));
        let event = EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_eval_{}", now_ms)),
            EventSource::Runtime,
            RuntimeEvent::OutcomeRecorded(OutcomeRecorded {
                project: project.clone(),
                outcome_id: outcome_id.clone(),
                run_id: RunId::new(run_id_str),
                agent_type: metric_name.clone(),
                predicted_confidence: score,
                actual_outcome: if passed {
                    ActualOutcome::Success
                } else {
                    ActualOutcome::Failure
                },
                recorded_at: now_ms,
            }),
        );

        self.event_log
            .append(&[event])
            .await
            .map_err(|e| ToolError::Transient(format!("event log write failed: {e}")))?;

        Ok(ToolResult::ok(serde_json::json!({
            "run_id":      run_id_str,
            "metric_name": metric_name,
            "score":       score,
            "passed":      passed,
            "recorded":    true,
        })))
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_store::InMemoryStore;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn make_tool() -> EvalScoreTool {
        EvalScoreTool::new(Arc::new(InMemoryStore::new()))
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn name_tier_class() {
        let t = make_tool();
        assert_eq!(t.name(), "eval_score");
        assert_eq!(t.tier(), ToolTier::Registered);
        assert_eq!(t.execution_class(), ExecutionClass::SupervisedProcess);
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_run_id_is_invalid() {
        let err = make_tool()
            .execute(
                &project(),
                serde_json::json!({"metric_name":"q","score":0.8}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "run_id"));
    }

    #[tokio::test]
    async fn missing_metric_name_is_invalid() {
        let err = make_tool()
            .execute(&project(), serde_json::json!({"run_id":"r1","score":0.8}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "metric_name"));
    }

    #[tokio::test]
    async fn missing_score_is_invalid() {
        let err = make_tool()
            .execute(
                &project(),
                serde_json::json!({"run_id":"r1","metric_name":"q"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "score"));
    }

    #[tokio::test]
    async fn score_out_of_range_is_invalid() {
        let err = make_tool()
            .execute(
                &project(),
                serde_json::json!({"run_id":"r1","metric_name":"q","score":1.5}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "score"));
    }

    // ── Happy paths ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn records_metric_successfully() {
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({
                    "run_id": "run_test_1", "metric_name": "answer_quality", "score": 0.85
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["run_id"], "run_test_1");
        assert_eq!(result.output["metric_name"], "answer_quality");
        assert_eq!(result.output["recorded"], true);
        assert!((result.output["score"].as_f64().unwrap() - 0.85).abs() < 1e-9);
    }

    #[tokio::test]
    async fn score_above_half_defaults_to_passed() {
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({
                    "run_id": "r1", "metric_name": "m", "score": 0.7
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["passed"], true);
    }

    #[tokio::test]
    async fn score_below_half_defaults_to_failed() {
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({
                    "run_id": "r1", "metric_name": "m", "score": 0.3
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["passed"], false);
    }

    #[tokio::test]
    async fn explicit_passed_overrides_default() {
        // score 0.3 would default to failed, but explicit passed=true overrides
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({
                    "run_id": "r1", "metric_name": "m", "score": 0.3, "passed": true
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["passed"], true);
    }

    #[tokio::test]
    async fn event_written_to_store() {
        let store = Arc::new(InMemoryStore::new());
        let tool = EvalScoreTool::new(store.clone());
        tool.execute(
            &project(),
            serde_json::json!({
                "run_id": "r1", "metric_name": "quality", "score": 0.9
            }),
        )
        .await
        .unwrap();

        // Verify an OutcomeRecorded event was written.
        use cairn_store::EventLog;
        let events = store.read_stream(None, 100).await.unwrap();
        assert_eq!(events.len(), 1, "one event must be written");
        assert!(matches!(
            &events[0].envelope.payload,
            RuntimeEvent::OutcomeRecorded(ev) if ev.agent_type == "quality"
        ));
    }

    #[tokio::test]
    async fn boundary_score_zero_is_valid() {
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({"run_id":"r","metric_name":"m","score":0.0}),
            )
            .await
            .unwrap();
        assert_eq!(result.output["recorded"], true);
        assert_eq!(result.output["passed"], false); // 0.0 < 0.5
    }

    #[tokio::test]
    async fn boundary_score_one_is_valid() {
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({"run_id":"r","metric_name":"m","score":1.0}),
            )
            .await
            .unwrap();
        assert_eq!(result.output["recorded"], true);
        assert_eq!(result.output["passed"], true); // 1.0 >= 0.5
    }
}
