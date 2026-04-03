use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative)
}

fn read_json(relative: &str) -> Value {
    let path = repo_file(relative);
    let contents = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&contents)
        .unwrap_or_else(|err| panic!("failed to parse {} as json: {err}", path.display()))
}

fn assert_string(value: &Value, pointer: &str) {
    let found = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing json pointer `{pointer}`"));
    assert!(
        found.is_string(),
        "expected string at `{pointer}`, got {found:?}"
    );
}

fn assert_bool(value: &Value, pointer: &str) {
    let found = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing json pointer `{pointer}`"));
    assert!(
        found.is_boolean(),
        "expected bool at `{pointer}`, got {found:?}"
    );
}

fn assert_object(value: &Value, pointer: &str) {
    let found = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing json pointer `{pointer}`"));
    assert!(
        found.is_object(),
        "expected object at `{pointer}`, got {found:?}"
    );
}

fn assert_array(value: &Value, pointer: &str) {
    let found = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing json pointer `{pointer}`"));
    assert!(
        found.is_array(),
        "expected array at `{pointer}`, got {found:?}"
    );
}

fn assert_number(value: &Value, pointer: &str) {
    let found = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing json pointer `{pointer}`"));
    assert!(
        found.is_number(),
        "expected number at `{pointer}`, got {found:?}"
    );
}

fn assert_provenance(value: &Value) {
    assert_array(value, "/provenance");
}

#[test]
fn phase0_http_fixtures_match_minimum_contracts() {
    let feed = read_json("tests/fixtures/http/GET__v1_feed__limit20_unread_true.json");
    assert_provenance(&feed);
    assert_string(&feed, "/request/method");
    assert_string(&feed, "/request/path");
    assert_array(&feed, "/response/items");
    assert_bool(&feed, "/response/hasMore");
    assert_object(&feed, "/response/items/0");
    assert_string(&feed, "/response/items/0/id");

    let tasks = read_json("tests/fixtures/http/GET__v1_tasks__status_running_type_agent.json");
    assert_provenance(&tasks);
    assert_array(&tasks, "/response/items");
    assert_bool(&tasks, "/response/hasMore");
    assert_string(&tasks, "/response/items/0/id");
    assert_string(&tasks, "/response/items/0/status");

    let approvals = read_json("tests/fixtures/http/GET__v1_approvals__status_pending.json");
    assert_provenance(&approvals);
    assert_array(&approvals, "/response/items");
    assert_bool(&approvals, "/response/hasMore");
    assert_string(&approvals, "/response/items/0/id");
    assert_string(&approvals, "/response/items/0/status");

    let memories = read_json("tests/fixtures/http/GET__v1_memories_search__q_test_limit_10.json");
    assert_provenance(&memories);
    assert_array(&memories, "/response/items");
    assert_string(&memories, "/response/items/0/id");
    assert_string(&memories, "/response/items/0/content");

    let assistant_with_session =
        read_json("tests/fixtures/http/POST__v1_assistant_message__with_session.json");
    assert_provenance(&assistant_with_session);
    assert_string(&assistant_with_session, "/request/body/message");
    assert_string(&assistant_with_session, "/request/body/mode");
    assert_string(&assistant_with_session, "/request/body/sessionId");
    assert_string(&assistant_with_session, "/response/taskId");

    let assistant_without_session =
        read_json("tests/fixtures/http/POST__v1_assistant_message__without_session.json");
    assert_provenance(&assistant_without_session);
    assert_string(&assistant_without_session, "/request/body/message");
    assert_string(&assistant_without_session, "/request/body/mode");
    assert!(
        assistant_without_session
            .pointer("/request/body/sessionId")
            .is_none(),
        "sessionId should be absent in the no-session fixture"
    );
    assert_string(&assistant_without_session, "/response/taskId");

    let stream = read_json("tests/fixtures/http/GET__v1_stream__replay_from_last_event_id.json");
    assert_provenance(&stream);
    assert_string(&stream, "/request/query/lastEventId");
    assert_string(&stream, "/response_contract/transport");
    assert_array(&stream, "/response_contract/expected_first_event_names");
    assert_eq!(
        stream
            .pointer("/response_contract/transport")
            .and_then(Value::as_str),
        Some("sse")
    );
    assert_eq!(
        stream
            .pointer("/response_contract/expected_first_event_names/0")
            .and_then(Value::as_str),
        Some("ready")
    );
}

#[test]
fn phase0_sse_fixtures_match_minimum_contracts() {
    let ready = read_json("tests/fixtures/sse/ready__connected.json");
    assert_provenance(&ready);
    assert_eq!(
        ready.pointer("/event").and_then(Value::as_str),
        Some("ready")
    );
    assert_string(&ready, "/payload/clientId");

    let feed_update = read_json("tests/fixtures/sse/feed_update__single_item.json");
    assert_eq!(
        feed_update.pointer("/event").and_then(Value::as_str),
        Some("feed_update")
    );
    assert_object(&feed_update, "/payload/item");

    let poll_completed = read_json("tests/fixtures/sse/poll_completed__source_done.json");
    assert_eq!(
        poll_completed.pointer("/event").and_then(Value::as_str),
        Some("poll_completed")
    );
    assert_string(&poll_completed, "/payload/source");
    assert_number(&poll_completed, "/payload/newCount");

    let task_update = read_json("tests/fixtures/sse/task_update__running_task.json");
    assert_eq!(
        task_update.pointer("/event").and_then(Value::as_str),
        Some("task_update")
    );
    assert_object(&task_update, "/payload/task");
    assert_string(&task_update, "/payload/task/id");

    let approval_required = read_json("tests/fixtures/sse/approval_required__pending.json");
    assert_eq!(
        approval_required.pointer("/event").and_then(Value::as_str),
        Some("approval_required")
    );
    assert_object(&approval_required, "/payload/approval");
    assert_string(&approval_required, "/payload/approval/id");

    let assistant_delta = read_json("tests/fixtures/sse/assistant_delta__incremental_reply.json");
    assert_eq!(
        assistant_delta.pointer("/event").and_then(Value::as_str),
        Some("assistant_delta")
    );
    assert_string(&assistant_delta, "/payload/taskId");
    assert_string(&assistant_delta, "/payload/deltaText");

    let assistant_end = read_json("tests/fixtures/sse/assistant_end__complete_reply.json");
    assert_eq!(
        assistant_end.pointer("/event").and_then(Value::as_str),
        Some("assistant_end")
    );
    assert_string(&assistant_end, "/payload/taskId");
    assert_string(&assistant_end, "/payload/messageText");

    let assistant_reasoning = read_json("tests/fixtures/sse/assistant_reasoning__round_1.json");
    assert_eq!(
        assistant_reasoning
            .pointer("/event")
            .and_then(Value::as_str),
        Some("assistant_reasoning")
    );
    assert_string(&assistant_reasoning, "/payload/taskId");
    assert_number(&assistant_reasoning, "/payload/round");
    assert_string(&assistant_reasoning, "/payload/thought");

    let assistant_tool_call = read_json("tests/fixtures/sse/assistant_tool_call__start.json");
    assert_eq!(
        assistant_tool_call
            .pointer("/event")
            .and_then(Value::as_str),
        Some("assistant_tool_call")
    );
    assert_string(&assistant_tool_call, "/payload/taskId");
    assert_string(&assistant_tool_call, "/payload/toolName");
    assert_string(&assistant_tool_call, "/payload/phase");

    let memory_proposed = read_json("tests/fixtures/sse/memory_proposed__proposal.json");
    assert_eq!(
        memory_proposed.pointer("/event").and_then(Value::as_str),
        Some("memory_proposed")
    );
    assert_object(&memory_proposed, "/payload/memory");
    assert_string(&memory_proposed, "/payload/memory/content");

    let agent_progress = read_json("tests/fixtures/sse/agent_progress__message.json");
    assert_eq!(
        agent_progress.pointer("/event").and_then(Value::as_str),
        Some("agent_progress")
    );
    assert_string(&agent_progress, "/payload/agentId");
    assert_string(&agent_progress, "/payload/message");
}
