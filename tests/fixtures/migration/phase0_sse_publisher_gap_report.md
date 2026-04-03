# Phase 0 SSE Publisher Gap Report

Status: generated  
Purpose: show how the current Rust SSE publisher surface relates to the preserved Phase 0 SSE contract.

Current reading:

- this report is based on the current `cairn-api/src/sse_publisher.rs` implementation plus the preserved Phase 0 SSE requirement set
- it does not claim backend parity with the legacy Go app; it highlights where the Rust-side runtime publisher is already aligned and where it still needs explicit compatibility work
- Worker 1 should use this report to keep payload-shape drift visible while Worker 8 tightens the SSE publisher

Interpretation:

- `supported_via_ready_frame`: already covered by a dedicated publisher path
- `mapped_with_shaped_payload_followup_remaining`: event name is present and the publisher now uses `sse_payloads`, but the emitted field set still needs alignment with the preserved frontend fixture contract
- `no_runtime_publisher_mapping_yet`: preserved SSE event exists in the frontend contract, but no equivalent runtime-publisher mapping is visible yet in the current Rust source

## Phase 0 SSE Status

| Event | Current Status | Notes | Next Step |
|---|---|---|---|
| `ready` | `supported_via_ready_frame` | `build_ready_frame()` covers connection bootstrap with `{ clientId }`. | `keep` |
| `feed_update` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`. | `decide_runtime_or_non_runtime_publisher_owner` |
| `poll_completed` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`. | `decide_runtime_or_non_runtime_publisher_owner` |
| `task_update` | `mapped_with_shaped_payload_followup_remaining` | Name is mapped and payload shaping exists, but current `sse_payloads` output is still narrower than the preserved fixture contract (`taskId/state/eventType` instead of the fuller `{ task: { id, type, status, title, description, progress, createdAt, updatedAt } }` shape). | `expand_shaped_payload_to_preserved_fixture` |
| `approval_required` | `mapped_with_shaped_payload_followup_remaining` | Name is mapped and payload shaping exists, but current `sse_payloads` output only carries `approvalId/runId/taskId` instead of the fuller preserved `{ approval: { id, type, status, title, description, context, createdAt } }` shape. | `expand_shaped_payload_to_preserved_fixture` |
| `assistant_delta` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; streaming token events are not yet represented by the current runtime-event publisher mapping. | `decide_runtime_or_non_runtime_publisher_owner` |
| `assistant_end` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; final assistant text event is not yet represented by the current runtime-event publisher mapping. | `decide_runtime_or_non_runtime_publisher_owner` |
| `assistant_reasoning` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; reasoning trace event is not yet represented by the current runtime-event publisher mapping. | `decide_runtime_or_non_runtime_publisher_owner` |
| `assistant_tool_call` | `mapped_with_shaped_payload_followup_remaining` | Name is mapped and payload shaping exists, but current `sse_payloads` output still needs preserved-field alignment across phases (for example completed/failed events currently collapse toward invocation identifiers instead of preserving the frontend tool-call envelope consistently). | `expand_shaped_payload_to_preserved_fixture` |
| `memory_proposed` | `no_runtime_publisher_mapping_yet` | Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`. | `decide_runtime_or_non_runtime_publisher_owner` |
| `agent_progress` | `mapped_with_shaped_payload_followup_remaining` | Name is mapped and payload shaping exists, but current builder still needs frontend-contract tightening for subagent/runtime progress semantics beyond the minimal `{ agentId, message }` fields. | `expand_shaped_payload_to_preserved_fixture` |
