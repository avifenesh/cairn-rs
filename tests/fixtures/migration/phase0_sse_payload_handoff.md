# Phase 0 SSE Payload Handoff

Status: generated  
Purpose: give Worker 8 a concrete event-by-event handoff from current runtime publisher inputs to the preserved SSE payload shapes Worker 1 is guarding.

Current implementation note:

- `cairn-api/src/sse_publisher.rs` now routes mapped runtime events through `crate::sse_payloads::shape_event_payload(&stored.envelope.payload)`.
- that is a real compatibility step forward: event names and wrapper families now exist for the mapped runtime surfaces.
- the remaining work is field-level alignment with the preserved frontend fixtures, not raw-event serialization removal.

## Event Handoff Table

| Event | Current Runtime Source | Expected Preserved Payload Shape | Current Status | Suggested Builder Direction |
|---|---|---|---|---|
| `ready` | `build_ready_frame()` | `{ clientId }` | `covered` | `keep_existing_ready_builder` |
| `task_update` | `TaskCreated | TaskStateChanged | TaskLeaseClaimed | TaskLeaseHeartbeated` | `{ task: { id, type, status, title, description, progress, createdAt, updatedAt } }` | `shaped_builder_present_fixture_alignment_remaining` | `expand_existing_task_update_shaper_fields` |
| `approval_required` | `ApprovalRequested` | `{ approval: { id, type, status, title, description, context, createdAt } }` | `shaped_builder_present_fixture_alignment_remaining` | `expand_existing_approval_required_shaper_fields` |
| `assistant_tool_call` | `ToolInvocationStarted | ToolInvocationCompleted | ToolInvocationFailed` | `{ taskId, toolName, phase, args?, result? }` | `shaped_builder_present_fixture_alignment_remaining` | `expand_existing_assistant_tool_call_shaper_fields` |
| `agent_progress` | `ExternalWorkerReported | SubagentSpawned` | `{ agentId, message }` | `shaped_builder_present_fixture_alignment_remaining` | `tighten_existing_agent_progress_shaper_fields` |
| `feed_update` | `no_runtime_mapping_yet` | `{ item: { id, source, kind, title, body, url, author, avatarUrl, repoFullName, isRead, isArchived, groupKey, createdAt } }` | `owner_and_builder_missing` | `decide_non_runtime_or_signal_owner_then_add_builder` |
| `poll_completed` | `no_runtime_mapping_yet` | `{ source, newCount }` | `owner_and_builder_missing` | `decide_non_runtime_or_signal_owner_then_add_builder` |
| `assistant_delta` | `no_runtime_mapping_yet` | `{ taskId, deltaText }` | `owner_and_builder_missing` | `decide_agent_stream_owner_then_add_builder` |
| `assistant_end` | `no_runtime_mapping_yet` | `{ taskId, messageText }` | `owner_and_builder_missing` | `decide_agent_stream_owner_then_add_builder` |
| `assistant_reasoning` | `no_runtime_mapping_yet` | `{ taskId, round, thought }` | `owner_and_builder_missing` | `decide_agent_stream_owner_then_add_builder` |
| `memory_proposed` | `no_runtime_mapping_yet` | `{ memory: { id, category, status, content, source, confidence, createdAt } }` | `owner_and_builder_missing` | `decide_memory_owner_then_add_builder` |
