# Phase 0 SSE Publisher Gap Report

Status: generated  
Purpose: show how the current Rust SSE publisher surface relates to the preserved Phase 0 SSE contract.

Current reading:

- this report is based on the current `cairn-api/src/sse_publisher.rs` implementation plus the preserved Phase 0 SSE requirement set
- it does not claim backend parity with the legacy Go app; it highlights where the Rust-side runtime publisher is already aligned and where it still needs explicit compatibility work
- Worker 1 should use this report to keep payload-shape drift visible while Worker 8 tightens the SSE publisher

Interpretation:

- `supported_via_ready_frame`: already covered by a dedicated publisher path
- `supported_via_dedicated_builder`: covered by a dedicated non-runtime builder path (feed, poll, or assistant streaming)
- `supported_via_dedicated_builder_followup_remaining`: a dedicated builder path exists, but one preserved payload field still needs to be populated correctly
- `runtime_mapping_followup_remaining_exact_dedicated_builder_present`: an exact dedicated builder already exists, but the generic runtime-event mapping path is still thinner than the preserved fixture contract
- `runtime_mapping_followup_remaining_enriched_builder_present`: an enriched builder exists for this family, but the generic runtime-event mapping path still drops preserved phase/result detail
- `mapped_with_shaped_payload_exact_current_contract`: event name is mapped and the current shaped payload already matches the preserved contract that the Phase 0 fixtures exercise today
- `no_runtime_or_dedicated_publisher_mapping_yet`: preserved SSE event exists in the frontend contract, but no equivalent runtime or dedicated non-runtime publisher mapping is visible yet in the current Rust source

## Phase 0 SSE Status

| Event | Current Status | Notes | Next Step |
|---|---|---|---|
| `ready` | `supported_via_ready_frame` | `build_ready_frame()` covers connection bootstrap with `{ clientId }`. | `keep` |
| `feed_update` | `supported_via_dedicated_builder` | Covered by `build_feed_update_frame(...)` in `sse_payloads.rs`; the preserved feed item envelope now matches the current string-ID fixture contract. | `keep` |
| `poll_completed` | `supported_via_dedicated_builder` | Covered by `build_poll_completed_frame(...)` in `sse_payloads.rs`; this SSE family is available through the dedicated polling publisher path rather than runtime-event mapping. | `keep` |
| `task_update` | `runtime_mapping_followup_remaining_exact_dedicated_builder_present` | An exact dedicated task-update builder exists (`build_enriched_task_update_frame(...)`), and the new `build_sse_frame_with_current_state(...)` helper can thread current-state task records into that path; the plain runtime-event fallback is still thinner when no store context is supplied. | `expand_shaped_payload_to_preserved_fixture` |
| `approval_required` | `runtime_mapping_followup_remaining_exact_dedicated_builder_present` | An exact dedicated approval builder exists (`build_enriched_approval_frame(...)`), and the new `build_sse_frame_with_current_state(...)` helper can thread current-state approval records into that path; the plain runtime-event fallback is still thinner when no store context is supplied. | `expand_shaped_payload_to_preserved_fixture` |
| `assistant_delta` | `supported_via_dedicated_builder` | Covered by `build_streaming_sse_frame(StreamingOutput::AssistantDelta, ...)`; streaming token updates are available through the dedicated assistant-streaming builder path. | `keep` |
| `assistant_end` | `supported_via_dedicated_builder_followup_remaining` | Covered by `build_streaming_sse_frame(StreamingOutput::AssistantEnd, ...)`, but the builder still emits an empty `messageText` placeholder unless the caller supplies the assembled final reply text. | `pass_assembled_final_message_text_into_streaming_builder` |
| `assistant_reasoning` | `supported_via_dedicated_builder` | Covered by `build_streaming_sse_frame(StreamingOutput::AssistantReasoning, ...)`; reasoning trace updates are available through the dedicated assistant-streaming builder path. | `keep` |
| `assistant_tool_call` | `runtime_mapping_followup_remaining_enriched_builder_present` | The exact start-phase payload exists and there is an enriched builder path (`build_enriched_tool_call_frame(...)`), and the runtime-event completed/failed paths in `shape_event_payload(...)` now preserve `taskId`, `toolName`, and `phase`; the remaining follow-up is richer result/error payload semantics. | `expand_shaped_payload_to_preserved_fixture` |
| `memory_proposed` | `no_runtime_or_dedicated_publisher_mapping_yet` | Required by preserved frontend SSE contract, but no runtime-event mapping or dedicated non-runtime builder is visible yet in the current Rust API slice. | `decide_runtime_or_non_runtime_publisher_owner` |
| `agent_progress` | `mapped_with_shaped_payload_exact_current_contract` | Name is mapped and the current `sse_payloads` builder already matches the preserved minimal `{ agentId, message }` contract used by the frontend fixture; richer progress semantics are a later product concern, not a current contract mismatch. | `keep` |
