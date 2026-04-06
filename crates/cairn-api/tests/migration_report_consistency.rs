use std::fs;
use std::path::PathBuf;

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative)
}

fn read_report(relative: &str) -> String {
    let path = repo_file(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn markdown_row<'a>(contents: &'a str, key: &str) -> &'a str {
    contents
        .lines()
        .find(|line| line.starts_with(&format!("| `{key}` |")))
        .unwrap_or_else(|| panic!("missing markdown row for `{key}`"))
}

fn markdown_row_prefix<'a>(contents: &'a str, prefix: &str) -> &'a str {
    contents
        .lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("missing markdown row starting with `{prefix}`"))
}

#[test]
fn migration_readme_lists_current_generated_phase0_reports() {
    let readme = read_report("tests/fixtures/migration/README.md");

    for artifact in [
        "phase0_mismatch_report.md",
        "phase0_upstream_contract_report.md",
        "phase0_upstream_source_pointers.md",
        "phase0_http_endpoint_gap_report.md",
        "phase0_sse_publisher_gap_report.md",
        "phase0_sse_payload_handoff.md",
        "phase0_owner_map.md",
    ] {
        assert!(
            readme.contains(&format!("`{artifact}`")),
            "migration README is missing `{artifact}`",
        );
    }
}

#[test]
fn owner_map_and_sse_gap_report_agree_on_builder_backed_streaming_events() {
    let owner_map = read_report("tests/fixtures/migration/phase0_owner_map.md");
    let sse_gap_report = read_report("tests/fixtures/migration/phase0_sse_publisher_gap_report.md");

    let assistant_delta_owner = markdown_row(&owner_map, "assistant_delta");
    let assistant_delta_gap = markdown_row(&sse_gap_report, "assistant_delta");
    assert!(
        assistant_delta_owner.contains("dedicated assistant-streaming builder present"),
        "owner map should treat assistant_delta as builder-backed",
    );
    assert!(
        assistant_delta_gap.contains("`supported_via_dedicated_builder`"),
        "SSE gap report should treat assistant_delta as builder-backed",
    );

    let assistant_reasoning_owner = markdown_row(&owner_map, "assistant_reasoning");
    let assistant_reasoning_gap = markdown_row(&sse_gap_report, "assistant_reasoning");
    assert!(
        assistant_reasoning_owner.contains("dedicated assistant-streaming builder present"),
        "owner map should treat assistant_reasoning as builder-backed",
    );
    assert!(
        assistant_reasoning_gap.contains("`supported_via_dedicated_builder`"),
        "SSE gap report should treat assistant_reasoning as builder-backed",
    );

    let assistant_end_owner = markdown_row(&owner_map, "assistant_end");
    let assistant_end_gap = markdown_row(&sse_gap_report, "assistant_end");
    assert!(
        assistant_end_owner
            .contains("dedicated assistant-streaming builder present; assembled final text handoff still open"),
        "owner map should keep the assistant_end handoff gap explicit",
    );
    assert!(
        assistant_end_gap.contains("`supported_via_dedicated_builder_followup_remaining`"),
        "SSE gap report should keep the assistant_end follow-up explicit",
    );
}

#[test]
fn sse_reports_distinguish_exact_builders_from_runtime_mapping_gaps() {
    let owner_map = read_report("tests/fixtures/migration/phase0_owner_map.md");
    let sse_gap_report = read_report("tests/fixtures/migration/phase0_sse_publisher_gap_report.md");
    let payload_handoff = read_report("tests/fixtures/migration/phase0_sse_payload_handoff.md");

    let task_owner = markdown_row(&owner_map, "task_update");
    let task_gap = markdown_row(&sse_gap_report, "task_update");
    let task_handoff = markdown_row(&payload_handoff, "task_update");
    assert!(
        task_owner.contains("exact dedicated builder present"),
        "owner map should acknowledge the exact task_update builder",
    );
    assert!(
        task_owner.contains("current-state helper available"),
        "owner map should acknowledge the current-state task_update helper",
    );
    assert!(
        task_gap.contains("`runtime_mapping_followup_remaining_exact_dedicated_builder_present`"),
        "SSE gap report should distinguish the exact task_update builder from the thinner runtime mapping",
    );
    assert!(
        task_gap.contains("build_sse_frame_with_current_state(...)"),
        "SSE gap report should mention the current-state helper now available for task_update",
    );
    assert!(
        task_handoff.contains("`exact_builder_present_runtime_mapping_followup_remaining`"),
        "payload handoff should keep the task_update distinction explicit",
    );
    assert!(
        task_handoff.contains("build_sse_frame_with_current_state(...)"),
        "payload handoff should mention the current-state helper for task_update",
    );

    let approval_owner = markdown_row(&owner_map, "approval_required");
    let approval_gap = markdown_row(&sse_gap_report, "approval_required");
    let approval_handoff = markdown_row(&payload_handoff, "approval_required");
    assert!(
        approval_owner.contains("exact dedicated builder present"),
        "owner map should acknowledge the exact approval_required builder",
    );
    assert!(
        approval_owner.contains("current-state helper available"),
        "owner map should acknowledge the current-state approval_required helper",
    );
    assert!(
        approval_gap
            .contains("`runtime_mapping_followup_remaining_exact_dedicated_builder_present`"),
        "SSE gap report should distinguish the exact approval_required builder from the thinner runtime mapping",
    );
    assert!(
        approval_gap.contains("build_sse_frame_with_current_state(...)"),
        "SSE gap report should mention the current-state helper now available for approval_required",
    );
    assert!(
        approval_handoff.contains("`exact_builder_present_runtime_mapping_followup_remaining`"),
        "payload handoff should keep the approval_required distinction explicit",
    );
    assert!(
        approval_handoff.contains("build_sse_frame_with_current_state(...)"),
        "payload handoff should mention the current-state helper for approval_required",
    );

    let tool_owner = markdown_row(&owner_map, "assistant_tool_call");
    let tool_gap = markdown_row(&sse_gap_report, "assistant_tool_call");
    let tool_handoff = markdown_row(&payload_handoff, "assistant_tool_call");
    assert!(
        tool_owner.contains("exact start-phase payload plus enriched builder present"),
        "owner map should acknowledge the richer assistant_tool_call builder path",
    );
    assert!(
        tool_owner.contains("completed/failed runtime identity is preserved"),
        "owner map should state that completed/failed assistant_tool_call now preserves runtime identity",
    );
    assert!(
        tool_gap.contains("`runtime_mapping_followup_remaining_enriched_builder_present`"),
        "SSE gap report should keep the assistant_tool_call runtime-phase gap explicit",
    );
    assert!(
        tool_gap.contains("preserve `taskId`, `toolName`, and `phase`"),
        "SSE gap report should state that completed/failed assistant_tool_call now preserves task/tool identity and phase",
    );
    assert!(
        tool_handoff.contains("`start_fixture_exact_runtime_phase_followup_remaining`"),
        "payload handoff should keep the assistant_tool_call start-vs-runtime distinction explicit",
    );
    assert!(
        tool_handoff.contains("now-stable completed/failed taskId/toolName/phase semantics"),
        "payload handoff should keep the assistant_tool_call follow-up narrowed to richer result/error detail",
    );

    let progress_owner = markdown_row(&owner_map, "agent_progress");
    let progress_gap = markdown_row(&sse_gap_report, "agent_progress");
    let progress_handoff = markdown_row(&payload_handoff, "agent_progress");
    assert!(
        progress_owner.contains("current minimal fixture contract is exact"),
        "owner map should treat the current agent_progress contract as exact",
    );
    assert!(
        progress_gap.contains("`mapped_with_shaped_payload_exact_current_contract`"),
        "SSE gap report should treat the current agent_progress contract as exact",
    );
    assert!(
        progress_handoff.contains("`covered_for_current_fixture_contract`"),
        "payload handoff should treat the current agent_progress contract as covered",
    );
}

#[test]
fn owner_map_and_sse_gap_report_agree_on_feed_poll_and_memory_gap_states() {
    let owner_map = read_report("tests/fixtures/migration/phase0_owner_map.md");
    let sse_gap_report = read_report("tests/fixtures/migration/phase0_sse_publisher_gap_report.md");
    let payload_handoff = read_report("tests/fixtures/migration/phase0_sse_payload_handoff.md");

    let feed_owner = markdown_row(&owner_map, "feed_update");
    let feed_gap = markdown_row(&sse_gap_report, "feed_update");
    let feed_handoff = markdown_row(&payload_handoff, "feed_update");
    assert!(
        feed_owner.contains("dedicated non-runtime builder present"),
        "owner map should treat feed_update as builder-backed",
    );
    assert!(
        feed_gap.contains("`supported_via_dedicated_builder`"),
        "SSE gap report should treat feed_update as builder-backed",
    );
    assert!(
        !feed_owner.contains("FeedItem.id"),
        "owner map should no longer describe a FeedItem.id mismatch",
    );
    assert!(
        !feed_gap.contains("FeedItem.id"),
        "SSE gap report should no longer describe a FeedItem.id mismatch",
    );
    assert!(
        feed_handoff.contains("`covered`") && feed_handoff.contains("`none`"),
        "payload handoff should now treat feed_update as covered",
    );

    let poll_owner = markdown_row(&owner_map, "poll_completed");
    let poll_gap = markdown_row(&sse_gap_report, "poll_completed");
    assert!(
        poll_owner.contains("dedicated non-runtime builder present"),
        "owner map should treat poll_completed as builder-backed",
    );
    assert!(
        poll_gap.contains("`supported_via_dedicated_builder`"),
        "SSE gap report should treat poll_completed as builder-backed",
    );

    let memory_owner = markdown_row(&owner_map, "memory_proposed");
    let memory_gap = markdown_row(&sse_gap_report, "memory_proposed");
    assert!(
        memory_owner.contains("dedicated non-runtime builder present"),
        "owner map should reflect the memory_proposed builder wired to SSE broadcast",
    );
    assert!(
        memory_gap.contains("`supported_via_dedicated_builder`"),
        "SSE gap report should treat memory_proposed as builder-backed",
    );
}

#[test]
fn sse_payload_handoff_reflects_memory_proposed_builder_wired() {
    let payload_handoff = read_report("tests/fixtures/migration/phase0_sse_payload_handoff.md");
    let memory_handoff = markdown_row(&payload_handoff, "memory_proposed");

    assert!(
        memory_handoff.contains("`build_memory_proposed_frame(item, eventId)`"),
        "payload handoff should show the memory_proposed builder as the runtime source",
    );
    assert!(
        memory_handoff.contains("`covered`"),
        "payload handoff should treat memory_proposed as covered",
    );
    assert!(
        memory_handoff.contains("`none`"),
        "payload handoff should show no remaining follow-up for memory_proposed",
    );
}

#[test]
fn owner_map_and_http_gap_report_agree_on_explicit_route_seams() {
    let owner_map = read_report("tests/fixtures/migration/phase0_owner_map.md");
    let http_gap_report =
        read_report("tests/fixtures/migration/phase0_http_endpoint_gap_report.md");

    let feed_owner = markdown_row(&owner_map, "GET /v1/feed?limit=20&unread=true");
    let feed_gap = markdown_row(&http_gap_report, "GET /v1/feed?limit=20&unread=true");
    assert!(
        feed_owner.contains("`FeedEndpoints` trait present in `feed.rs`"),
        "owner map should reflect the explicit feed API seam",
    );
    assert!(
        feed_gap.contains("`dedicated_endpoint_trait_present`"),
        "HTTP gap report should now treat the feed response shape as explicit",
    );

    let memory_owner = markdown_row(&owner_map, "GET /v1/memories/search?q=test&limit=10");
    let memory_gap = markdown_row(&http_gap_report, "GET /v1/memories/search?q=test&limit=10");
    assert!(
        memory_owner.contains("`MemoryEndpoints::search` present in `memory_api.rs`"),
        "owner map should reflect the explicit memory search API seam",
    );
    assert!(
        memory_gap.contains("`dedicated_endpoint_trait_present_followup_remaining`"),
        "HTTP gap report should keep the memory response follow-up explicit",
    );

    let assistant_owner = markdown_row_prefix(
        &owner_map,
        "| `POST /v1/assistant/message` (with session) |",
    );
    let assistant_gap = markdown_row(
        &http_gap_report,
        "POST /v1/assistant/message body={message,mode?,sessionId?}",
    );
    assert!(
        assistant_owner.contains("`AssistantEndpoints::send_message` present in `assistant.rs`"),
        "owner map should reflect the explicit assistant API seam",
    );
    assert!(
        assistant_gap.contains("`dedicated_endpoint_trait_present`"),
        "HTTP gap report should treat the assistant command boundary as explicit",
    );
}
