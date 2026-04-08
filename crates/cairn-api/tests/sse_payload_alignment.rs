//! Executable tests verifying SSE payload shapes match preserved fixtures.
//!
//! For builder-owned families that are expected to be exact today, these
//! assertions compare the serialized JSON directly to the preserved fixture
//! payload. For families that are still intentionally thinner, the tests stay
//! explicit about the narrower guarantee so the compatibility reports remain
//! truthful.

use std::collections::HashSet;

fn load_fixture_payload(fixture_name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../tests/fixtures/sse/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        fixture_name
    );
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to load fixture {path}: {e}"));
    let fixture: serde_json::Value = serde_json::from_str(&content).unwrap();
    fixture["payload"].clone()
}

fn top_level_keys(value: &serde_json::Value) -> HashSet<String> {
    match value.as_object() {
        Some(map) => map.keys().cloned().collect(),
        None => HashSet::new(),
    }
}

fn assert_json_matches_fixture(our_json: serde_json::Value, fixture: &serde_json::Value) {
    assert_eq!(
        our_json, *fixture,
        "serialized payload did not match preserved fixture example"
    );
}

#[test]
fn task_update_payload_has_fixture_keys() {
    let fixture = load_fixture_payload("task_update__running_task");
    let fixture_keys = top_level_keys(&fixture);

    // Our payload must have at least the "task" wrapper
    assert!(fixture_keys.contains("task"), "fixture has 'task' wrapper");

    // Verify our struct produces "task" wrapper
    let payload = cairn_api::sse_payloads::TaskUpdatePayload {
        task: cairn_api::sse_payloads::TaskUpdateInner {
            id: "task_001".to_owned(),
            task_type: Some("agent".to_owned()),
            status: Some("running".to_owned()),
            title: Some("Draft weekly digest".to_owned()),
            description: Some("Collect updates and prepare digest for review.".to_owned()),
            progress: Some(42),
            created_at: Some("2026-04-03T09:00:00Z".to_owned()),
            updated_at: Some("2026-04-03T09:32:00Z".to_owned()),
        },
    };
    let our_json = serde_json::to_value(&payload).unwrap();
    assert_json_matches_fixture(our_json, &fixture);
}

#[test]
fn task_update_runtime_mapping_falls_back_without_current_state() {
    use cairn_domain::events::{RuntimeEvent, TaskCreated};
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::TaskId;

    let event = RuntimeEvent::TaskCreated(TaskCreated {
        project: ProjectKey::new("t", "w", "p"),
        task_id: TaskId::new("task_001"),
        parent_run_id: None,
        parent_task_id: None,
        prompt_release_id: None,
    });
    let payload = cairn_api::sse_payloads::shape_event_payload(&event).unwrap();

    assert_eq!(payload["task"]["id"], "task_001");
    assert_eq!(payload["task"]["status"], "queued");
    assert!(
        payload["task"]["type"].is_null()
            && payload["task"]["title"].is_null()
            && payload["task"]["description"].is_null()
            && payload["task"]["progress"].is_null()
            && payload["task"]["createdAt"].is_null()
            && payload["task"]["updatedAt"].is_null(),
        "runtime task_update mapping is still the thin fallback when no current-state record is supplied",
    );
}

#[test]
fn task_update_current_state_helper_uses_store_record() {
    use cairn_domain::lifecycle::TaskState;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::TaskId;
    use cairn_store::projections::TaskRecord;

    let event =
        cairn_domain::events::RuntimeEvent::TaskCreated(cairn_domain::events::TaskCreated {
            project: ProjectKey::new("t", "w", "p"),
            task_id: TaskId::new("task_001"),
            parent_run_id: None,
            parent_task_id: None,
            prompt_release_id: None,
        });
    let record = TaskRecord {
        task_id: TaskId::new("task_001"),
        project: ProjectKey::new("t", "w", "p"),
        parent_run_id: None,
        parent_task_id: None,
        state: TaskState::Running,
        prompt_release_id: None,
        failure_class: None,
        pause_reason: None,
        resume_trigger: None,
        retry_count: 0,
        lease_owner: None,
        lease_expires_at: None,
        title: Some("Draft weekly digest".to_owned()),
        description: Some("Collect updates and prepare digest.".to_owned()),
        version: 2,
        created_at: 1000,
        updated_at: 1500,
    };

    let payload =
        cairn_api::sse_payloads::shape_event_payload_with_records(&event, Some(&record), None)
            .unwrap();

    assert_eq!(payload["task"]["id"], "task_001");
    assert_eq!(payload["task"]["status"], "running");
    assert_eq!(payload["task"]["title"], "Draft weekly digest");
    assert_eq!(
        payload["task"]["description"],
        "Collect updates and prepare digest."
    );
    assert_eq!(payload["task"]["createdAt"], "1000");
    assert_eq!(payload["task"]["updatedAt"], "1500");
}

#[test]
fn approval_required_runtime_mapping_falls_back_without_current_state() {
    let fixture = load_fixture_payload("approval_required__pending");
    let fixture_keys = top_level_keys(&fixture);
    assert!(fixture_keys.contains("approval"));

    let payload = cairn_api::sse_payloads::ApprovalRequiredPayload {
        approval: cairn_api::sse_payloads::ApprovalInner {
            id: "approval_001".to_owned(),
            approval_type: Some("tool_execution".to_owned()),
            status: "pending".to_owned(),
            title: Some("Approve GitHub write action".to_owned()),
            description: Some("Agent wants to create a draft pull request.".to_owned()),
            context: Some(
                serde_json::json!({"repo": "avife/cairn", "action": "create_pull_request"}),
            ),
            created_at: Some("2026-04-03T09:20:00Z".to_owned()),
        },
    };
    let our_json = serde_json::to_value(&payload).unwrap();
    assert_json_matches_fixture(our_json, &fixture);
}

#[test]
fn approval_required_runtime_mapping_gap_is_still_explicit() {
    use cairn_domain::events::{ApprovalRequested, RuntimeEvent};
    use cairn_domain::policy::ApprovalRequirement;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::{ApprovalId, TaskId};

    let event = RuntimeEvent::ApprovalRequested(ApprovalRequested {
        project: ProjectKey::new("t", "w", "p"),
        approval_id: ApprovalId::new("approval_001"),
        run_id: None,
        task_id: Some(TaskId::new("task_001")),
        requirement: ApprovalRequirement::Required,
    });
    let payload = cairn_api::sse_payloads::shape_event_payload(&event).unwrap();

    assert_eq!(payload["approval"]["id"], "approval_001");
    assert_eq!(payload["approval"]["status"], "pending");
    assert!(
        payload["approval"]["type"].is_null()
            && payload["approval"]["title"].is_null()
            && payload["approval"]["description"].is_null()
            && payload["approval"]["context"].is_null()
            && payload["approval"]["createdAt"].is_null(),
        "runtime approval_required mapping is still the thin fallback when no current-state record is supplied",
    );
}

#[test]
fn approval_required_current_state_helper_uses_store_record() {
    use cairn_domain::policy::ApprovalRequirement;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::{ApprovalId, TaskId};
    use cairn_store::projections::ApprovalRecord;

    let event = cairn_domain::events::RuntimeEvent::ApprovalRequested(
        cairn_domain::events::ApprovalRequested {
            project: ProjectKey::new("t", "w", "p"),
            approval_id: ApprovalId::new("approval_001"),
            run_id: None,
            task_id: Some(TaskId::new("task_001")),
            requirement: ApprovalRequirement::Required,
        },
    );
    let record = ApprovalRecord {
        approval_id: ApprovalId::new("approval_001"),
        project: ProjectKey::new("t", "w", "p"),
        run_id: None,
        task_id: Some(TaskId::new("task_001")),
        requirement: ApprovalRequirement::Required,
        decision: None,
        title: Some("Approve GitHub write action".to_owned()),
        description: Some("Agent wants to create a PR.".to_owned()),
        version: 1,
        created_at: 2000,
        updated_at: 2000,
    };

    let payload =
        cairn_api::sse_payloads::shape_event_payload_with_records(&event, None, Some(&record))
            .unwrap();

    assert_eq!(payload["approval"]["id"], "approval_001");
    assert_eq!(payload["approval"]["status"], "pending");
    assert_eq!(payload["approval"]["title"], "Approve GitHub write action");
    assert_eq!(
        payload["approval"]["description"],
        "Agent wants to create a PR."
    );
    assert_eq!(payload["approval"]["createdAt"], "2000");
}

#[test]
fn assistant_tool_call_payload_has_fixture_keys() {
    let fixture = load_fixture_payload("assistant_tool_call__start");

    let payload = cairn_api::sse_payloads::AssistantToolCallPayload {
        task_id: Some("task_assistant_001".to_owned()),
        tool_name: "list_approvals".to_owned(),
        phase: "start",
        args: Some(serde_json::json!({"status": "pending"})),
    };
    let our_json = serde_json::to_value(&payload).unwrap();
    assert_json_matches_fixture(our_json, &fixture);
}

#[test]
fn assistant_tool_call_completed_and_failed_now_preserve_tool_name_and_task_id() {
    use cairn_domain::events::{RuntimeEvent, ToolInvocationCompleted, ToolInvocationFailed};
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::{TaskId, ToolInvocationId, ToolInvocationOutcomeKind};

    let completed = RuntimeEvent::ToolInvocationCompleted(ToolInvocationCompleted {
        project: ProjectKey::new("t", "w", "p"),
        invocation_id: ToolInvocationId::new("inv_1"),
        task_id: Some(TaskId::new("task_assistant_001")),
        tool_name: "list_approvals".to_owned(),
        finished_at_ms: 200,
        outcome: ToolInvocationOutcomeKind::Success,
    });
    let failed = RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
        project: ProjectKey::new("t", "w", "p"),
        invocation_id: ToolInvocationId::new("inv_2"),
        task_id: Some(TaskId::new("task_assistant_001")),
        tool_name: "list_approvals".to_owned(),
        finished_at_ms: 201,
        outcome: ToolInvocationOutcomeKind::PermanentFailure,
        error_message: Some("bad input".to_owned()),
    });

    let completed_json = cairn_api::sse_payloads::shape_event_payload(&completed).unwrap();
    let failed_json = cairn_api::sse_payloads::shape_event_payload(&failed).unwrap();

    assert_eq!(completed_json["taskId"], "task_assistant_001");
    assert_eq!(completed_json["toolName"], "list_approvals");
    assert_eq!(completed_json["phase"], "completed");

    assert_eq!(failed_json["taskId"], "task_assistant_001");
    assert_eq!(failed_json["toolName"], "list_approvals");
    assert_eq!(failed_json["phase"], "failed");
    assert!(
        failed_json.get("args").is_none() || failed_json["args"].is_null(),
        "failed tool-call payload still lacks richer result/error shaping today",
    );
}

#[test]
fn agent_progress_payload_has_fixture_keys() {
    let fixture = load_fixture_payload("agent_progress__message");

    let payload = cairn_api::sse_payloads::AgentProgressPayload {
        agent_id: "agent_001".to_owned(),
        message: "Waiting for approval before continuing deployment workflow.".to_owned(),
    };
    let our_json = serde_json::to_value(&payload).unwrap();
    assert_json_matches_fixture(our_json, &fixture);
}

#[test]
fn agent_progress_runtime_mapping_matches_current_fixture_contract() {
    use cairn_domain::events::{ExternalWorkerReported, RuntimeEvent};
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::workers::{ExternalWorkerProgress, ExternalWorkerReport};
    use cairn_domain::{TaskId, WorkerId};

    let event = RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
        report: ExternalWorkerReport {
            project: ProjectKey::new("t", "w", "p"),
            worker_id: WorkerId::new("agent_001"),
            run_id: None,
            task_id: TaskId::new("task_001"),
            lease_token: 7,
            reported_at_ms: 100,
            progress: Some(ExternalWorkerProgress {
                message: Some(
                    "Waiting for approval before continuing deployment workflow.".to_owned(),
                ),
                percent_milli: Some(500),
            }),
            outcome: None,
        },
    });
    let payload = cairn_api::sse_payloads::shape_event_payload(&event).unwrap();
    let fixture = load_fixture_payload("agent_progress__message");

    assert_json_matches_fixture(payload, &fixture);
}

#[test]
fn poll_completed_payload_matches_fixture_exactly() {
    let fixture = load_fixture_payload("poll_completed__source_done");

    let frame = cairn_api::sse_payloads::build_poll_completed_frame("slack", 3, None);
    assert_json_matches_fixture(frame.data, &fixture);
}

#[test]
fn feed_update_payload_still_has_item_wrapper() {
    let fixture = load_fixture_payload("feed_update__single_item");
    let fixture_keys = top_level_keys(&fixture);
    assert!(fixture_keys.contains("item"), "fixture has 'item' wrapper");

    let item = cairn_api::feed::FeedItem {
        id: "101".to_owned(),
        source: "slack".to_owned(),
        kind: Some("message".to_owned()),
        title: Some("Build pipeline needs approval".to_owned()),
        body: Some("Deploy is waiting on approval from ops.".to_owned()),
        url: Some("https://example.test/slack/101".to_owned()),
        author: Some("ops-bot".to_owned()),
        avatar_url: Some("https://example.test/avatar/ops-bot.png".to_owned()),
        repo_full_name: Some("avife/cairn".to_owned()),
        is_read: false,
        is_archived: false,
        group_key: Some("slack:deploy".to_owned()),
        created_at: "2026-04-03T09:30:00Z".to_owned(),
    };
    let frame = cairn_api::sse_payloads::build_feed_update_frame(item, None);
    assert!(
        frame.data.get("item").is_some(),
        "our payload has 'item' wrapper"
    );
}

#[test]
fn feed_update_payload_matches_fixture_exactly() {
    let fixture = load_fixture_payload("feed_update__single_item");
    let item = cairn_api::feed::FeedItem {
        id: "101".to_owned(),
        source: "slack".to_owned(),
        kind: Some("message".to_owned()),
        title: Some("Build pipeline needs approval".to_owned()),
        body: Some("Deploy is waiting on approval from ops.".to_owned()),
        url: Some("https://example.test/slack/101".to_owned()),
        author: Some("ops-bot".to_owned()),
        avatar_url: Some("https://example.test/avatar/ops-bot.png".to_owned()),
        repo_full_name: Some("avife/cairn".to_owned()),
        is_read: false,
        is_archived: false,
        group_key: Some("slack:deploy".to_owned()),
        created_at: "2026-04-03T09:30:00Z".to_owned(),
    };
    let frame = cairn_api::sse_payloads::build_feed_update_frame(item, None);
    assert_json_matches_fixture(frame.data, &fixture);
}

#[test]
fn assistant_delta_payload_matches_fixture_exactly() {
    let fixture = load_fixture_payload("assistant_delta__incremental_reply");

    let payload = cairn_api::sse_payloads::AssistantDeltaPayload {
        task_id: "task_assistant_001".to_owned(),
        delta_text: "The current deploy is blocked by".to_owned(),
    };
    let our_json = serde_json::to_value(&payload).unwrap();
    assert_json_matches_fixture(our_json, &fixture);
}

#[test]
fn assistant_end_payload_matches_fixture_exactly() {
    let fixture = load_fixture_payload("assistant_end__complete_reply");

    let payload = cairn_api::sse_payloads::AssistantEndPayload {
        task_id: "task_assistant_001".to_owned(),
        message_text: "The current deploy is blocked by a pending approval from ops.".to_owned(),
    };
    let our_json = serde_json::to_value(&payload).unwrap();
    assert_json_matches_fixture(our_json, &fixture);
}

#[test]
fn assistant_end_streaming_builder_still_requires_caller_assembled_text() {
    use cairn_agent::streaming::{AssistantEnd, StopReason, StreamingOutput};
    use cairn_domain::{RunId, SessionId};

    let output = StreamingOutput::AssistantEnd(AssistantEnd {
        session_id: SessionId::new("sess_1"),
        run_id: RunId::new("run_1"),
        stop_reason: StopReason::EndTurn,
    });

    // build_streaming_sse_frame returns None for AssistantEnd —
    // callers must use build_enriched_assistant_end_frame with assembled text.
    let frame =
        cairn_api::sse_payloads::build_streaming_sse_frame(&output, "task_assistant_001", None);
    assert!(
        frame.is_none(),
        "AssistantEnd requires caller-assembled text via enriched builder"
    );

    // The real path:
    let enriched = cairn_api::sse_payloads::build_enriched_assistant_end_frame(
        "task_assistant_001",
        "The assembled reply text.",
        None,
    );
    assert_eq!(enriched.event.as_str(), "assistant_end");
    assert_eq!(enriched.data["taskId"], "task_assistant_001");
    assert_eq!(enriched.data["messageText"], "The assembled reply text.");
}

#[test]
fn assistant_reasoning_payload_matches_fixture_exactly() {
    let fixture = load_fixture_payload("assistant_reasoning__round_1");

    let payload = cairn_api::sse_payloads::AssistantReasoningPayload {
        task_id: "task_assistant_001".to_owned(),
        round: 1,
        thought:
            "I should inspect the current approvals and running tasks before summarizing blockers."
                .to_owned(),
    };
    let our_json = serde_json::to_value(&payload).unwrap();
    assert_json_matches_fixture(our_json, &fixture);
}
