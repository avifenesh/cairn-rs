# Phase 0 SSE Publisher Gap Report

Status: generated  
Purpose: show how the current Rust SSE publisher surface relates to the preserved Phase 0 SSE contract.

Current reading:

- this report is based on the current `cairn-api/src/sse_publisher.rs` implementation plus the preserved Phase 0 SSE requirement set
- it does not claim backend parity with the legacy Go app; it highlights where the Rust-side runtime publisher is already aligned and where it still needs explicit compatibility work
- Worker 1 should use this report to keep payload-shape drift visible while Worker 8 tightens the SSE publisher

Interpretation:

- `supported_via_ready_frame`: already covered by a dedicated publisher path
- `mapped_name_only_raw_payload_followup`: event name is present, but current frame data still reflects raw runtime-event serialization instead of the preserved frontend payload shape
- `no_runtime_publisher_mapping_yet`: preserved SSE event exists in the frontend contract, but no equivalent runtime-publisher mapping is visible yet in the current Rust source

## Phase 0 SSE Status

| Event | Current Status | Notes | Next Step |
|---|---|---|---|
| `ready` | `supported_via_ready_frame` | `build_ready_frame()` covers connection bootstrap with `{ clientId }`. | `keep` |
| `feed_update` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`. | `decide_runtime_or_non_runtime_publisher_owner` |
| `poll_completed` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`. | `decide_runtime_or_non_runtime_publisher_owner` |
| `task_update` | `mapped_name_only_raw_payload_followup` | Name is mapped, but current publisher serializes raw `RuntimeEvent` payload instead of preserved `{ task }` wrapper. | `align_payload_shape_to_preserved_fixture` |
| `approval_required` | `mapped_name_only_raw_payload_followup` | Name is mapped, but current publisher serializes raw `ApprovalRequested` payload instead of preserved `{ approval }` wrapper. | `align_payload_shape_to_preserved_fixture` |
| `assistant_delta` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; streaming token events are not yet represented by the current runtime-event publisher mapping. | `decide_runtime_or_non_runtime_publisher_owner` |
| `assistant_end` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; final assistant text event is not yet represented by the current runtime-event publisher mapping. | `decide_runtime_or_non_runtime_publisher_owner` |
| `assistant_reasoning` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; reasoning trace event is not yet represented by the current runtime-event publisher mapping. | `decide_runtime_or_non_runtime_publisher_owner` |
| `assistant_tool_call` | `mapped_name_only_raw_payload_followup` | Name is mapped, but current publisher serializes raw tool invocation events instead of preserved `{ taskId, toolName, phase, args?, result? }` shape. | `align_payload_shape_to_preserved_fixture` |
| `memory_proposed` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`. | `decide_runtime_or_non_runtime_publisher_owner` |
| `agent_progress` | `mapped_name_only_raw_payload_followup` | Name is mapped, but current publisher serializes raw worker/subagent events instead of preserved `{ agentId, message }` shape. | `align_payload_shape_to_preserved_fixture` |
