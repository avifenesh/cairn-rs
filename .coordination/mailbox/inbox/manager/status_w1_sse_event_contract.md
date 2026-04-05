# STATUS: sse_event_contract

**Task:** RFC 002 SSE event publishing contract hardening  
**Tests passed:** 15/15  
**File:** `crates/cairn-api/tests/sse_event_contract.rs`

Tests:
- `preserved_sse_catalog_contains_all_event_names`
- `preserved_sse_catalog_names_are_valid_snake_case`
- `preserved_sse_catalog_has_preserved_and_transitional_events`
- `sse_frame_serializes_event_name_data_and_id`
- `sse_frame_round_trips_through_serde`
- `sse_frame_id_none_serializes_correctly`
- `sse_event_name_variants_are_distinct_and_snake_case`
- `sse_replay_query_after_position_filtering`
- `parse_last_event_id_edge_cases`
- `ready_frame_is_emitted_on_connection`
- `ready_event_is_preserve_classified`
- `ready_frames_are_client_scoped`
- `keepalive_comment_format_follows_sse_spec`
- `keepalive_frames_do_not_advance_replay_position`
- `runtime_events_map_to_correct_sse_surfaces`
