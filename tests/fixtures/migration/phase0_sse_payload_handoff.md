# Phase 0 SSE Payload Handoff

Status: generated  
Purpose: give Worker 8 a concrete event-by-event handoff from current runtime publisher inputs to the preserved SSE payload shapes Worker 1 is guarding.

Current implementation note:

- `cairn-api/src/sse_publisher.rs` still serializes `stored.envelope.payload` directly via `serde_json::to_value(&stored.envelope.payload)` for mapped runtime events.
- that is enough for event-name coverage, but not enough for preserved frontend payload compatibility.
- this report makes the missing payload-builder work explicit without requiring Worker 1 to edit Worker 8 ownership code.

## Event Handoff Table

| Event | Current Runtime Source | Expected Preserved Payload Shape | Current Status | Suggested Builder Direction |
|---|---|---|---|---|
| `ready` | `build_ready_frame()` | `{ clientId }` | `covered` | `keep_existing_ready_builder` |
| `task_update` | `TaskCreated | TaskStateChanged | TaskLeaseClaimed | TaskLeaseHeartbeated` | `{ task: { id, type, status, title, description, progress, createdAt, updatedAt } }` | `mapped_name_but_payload_builder_missing` | `add_build_task_update_payload(&RuntimeEvent)` |
| `approval_required` | `ApprovalRequested` | `{ approval: { id, type, status, title, description, context, createdAt } }` | `mapped_name_but_payload_builder_missing` | `add_build_approval_required_payload(&RuntimeEvent)` |
| `assistant_tool_call` | `ToolInvocationStarted | ToolInvocationCompleted | ToolInvocationFailed` | `{ taskId, toolName, phase, args?, result? }` | `mapped_name_but_payload_builder_missing` | `add_build_assistant_tool_call_payload(&RuntimeEvent)` |
| `agent_progress` | `ExternalWorkerReported | SubagentSpawned` | `{ agentId, message }` | `mapped_name_but_payload_builder_missing` | `add_build_agent_progress_payload(&RuntimeEvent)` |
| `feed_update` | `no_runtime_mapping_yet` | `{ item: { id, source, kind, title, body, url, author, avatarUrl, repoFullName, isRead, isArchived, groupKey, createdAt } }` | `owner_and_builder_missing` | `decide_non_runtime_or_signal_owner_then_add_builder` |
| `poll_completed` | `no_runtime_mapping_yet` | `{ source, newCount }` | `owner_and_builder_missing` | `decide_non_runtime_or_signal_owner_then_add_builder` |
| `assistant_delta` | `no_runtime_mapping_yet` | `{ taskId, deltaText }` | `owner_and_builder_missing` | `decide_agent_stream_owner_then_add_builder` |
| `assistant_end` | `no_runtime_mapping_yet` | `{ taskId, messageText }` | `owner_and_builder_missing` | `decide_agent_stream_owner_then_add_builder` |
| `assistant_reasoning` | `no_runtime_mapping_yet` | `{ taskId, round, thought }` | `owner_and_builder_missing` | `decide_agent_stream_owner_then_add_builder` |
| `memory_proposed` | `no_runtime_mapping_yet` | `{ memory: { id, category, status, content, source, confidence, createdAt } }` | `owner_and_builder_missing` | `decide_memory_owner_then_add_builder` |
