# Phase 0 SSE Payload Handoff

Status: generated  
Purpose: give Worker 8 a concrete event-by-event handoff from current runtime publisher inputs to the preserved SSE payload shapes Worker 1 is guarding.

Current implementation note:

- `cairn-api/src/sse_publisher.rs` now routes mapped runtime events through `crate::sse_payloads::shape_event_payload(&stored.envelope.payload)`.
- that is a real compatibility step forward: event names and wrapper families now exist for the mapped runtime surfaces.
- the remaining work is field-level alignment with the preserved frontend fixtures, not raw-event serialization removal.

## Event Handoff Table

| Event | Current Runtime Source | Expected Preserved Payload Shape | Current Status | Suggested Builder Direction | Exact Follow-up |
|---|---|---|---|---|---|
| `ready` | `build_ready_frame()` | `{ clientId }` | `covered` | `keep_existing_ready_builder` | `none` |
| `task_update` | `TaskCreated | TaskStateChanged | TaskLeaseClaimed | TaskLeaseHeartbeated` | `{ task: { id, type, status, title, description, progress, createdAt, updatedAt } }` | `exact_builder_present_runtime_mapping_followup_remaining` | `prefer_exact_task_update_builder_or_backfill_runtime_mapping` | `either prefer the existing exact enriched builder or use build_sse_frame_with_current_state(...) to populate task.type/title/description/progress/createdAt/updatedAt on the generic runtime path from task metadata/read models` |
| `approval_required` | `ApprovalRequested` | `{ approval: { id, type, status, title, description, context, createdAt } }` | `exact_builder_present_runtime_mapping_followup_remaining` | `prefer_exact_approval_builder_or_backfill_runtime_mapping` | `either prefer the existing exact enriched builder or use build_sse_frame_with_current_state(...) to populate approval.type/title/description/context/createdAt on the generic runtime path from approval metadata/read-model context` |
| `assistant_tool_call` | `ToolInvocationStarted | ToolInvocationCompleted | ToolInvocationFailed` | `{ taskId, toolName, phase, args?, result? }` | `start_fixture_exact_runtime_phase_followup_remaining` | `preserve_start_builder_expand_completed_failed_runtime_mapping` | `preserve the existing exact start shape, keep the now-stable completed/failed taskId/toolName/phase semantics, and add richer result/error detail next` |
| `agent_progress` | `ExternalWorkerReported | SubagentSpawned` | `{ agentId, message }` | `covered_for_current_fixture_contract` | `keep_existing_agent_progress_shaper` | `none for the current minimal fixture contract; richer subagent/progress semantics can be deferred until the product contract expands` |
| `feed_update` | `build_feed_update_frame(item, eventId)` | `{ item: { id, source, kind, title, body, url, author, avatarUrl, repoFullName, isRead, isArchived, groupKey, createdAt } }` | `covered` | `keep_existing_feed_update_builder` | `none` |
| `poll_completed` | `build_poll_completed_frame(source, newCount, eventId)` | `{ source, newCount }` | `covered` | `keep_existing_poll_completed_builder` | `none` |
| `assistant_delta` | `build_streaming_sse_frame(StreamingOutput::AssistantDelta, taskId, eventId)` | `{ taskId, deltaText }` | `covered` | `keep_existing_streaming_builder` | `none` |
| `assistant_end` | `build_streaming_sse_frame(StreamingOutput::AssistantEnd, taskId, eventId)` | `{ taskId, messageText }` | `covered` | `keep_existing_streaming_builder` | `none` |
| `assistant_reasoning` | `build_streaming_sse_frame(StreamingOutput::AssistantReasoning, taskId, eventId)` | `{ taskId, round, thought }` | `covered` | `keep_existing_streaming_builder` | `none` |
| `memory_proposed` | `build_memory_proposed_frame(item, eventId)` via `SseMemoryProposalHook` | `{ memory: { id, category, status, content, source, confidence, createdAt } }` | `covered` | `keep_existing_memory_proposed_builder` | `none` |
