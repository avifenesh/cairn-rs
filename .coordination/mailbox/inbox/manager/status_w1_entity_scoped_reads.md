# STATUS: entity_scoped_reads

**Task:** RFC 002 entity-scoped event read hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/entity_scoped_reads.rs`

Tests:
- `read_by_entity_returns_only_matching_events`
- `cursor_based_pagination_skips_prior_events`
- `head_position_reflects_latest_append`
- `read_stream_returns_all_events_in_order`
- `entity_ref_matching_all_types`
- `same_type_different_id_events_are_independent`
