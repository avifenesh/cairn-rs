# STATUS: retention_window

**Task:** RFC 002 event log retention window hardening  
**Tests passed:** 5/5  
**File:** `crates/cairn-store/tests/retention_window.rs`

Tests:
- `fifty_events_read_stream_correct_count` — limit=100/10/1, after=40, after=50(empty)
- `head_position_equals_fifty_after_fifty_appends` — head=50, advances to 51
- `events_older_than_cutoff_are_identifiable` — stored_at boundary checks, 72h window
- `retention_doesnt_break_entity_scoped_reads` — apply_retention prunes 3 of 5 per-entity events; unrelated session and surviving events still entity-readable
- `seventy_two_hour_sse_replay_window_contract` — sequential positions 1-50; reconnect at P → (50-P) events; first event at P+1; at head → empty; all stored_at within 72h window
