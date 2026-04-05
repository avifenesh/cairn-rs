# STATUS: idempotency

**Task:** RFC 002 event log idempotency hardening  
**Tests passed:** 7/7  
**File:** `crates/cairn-store/tests/idempotency.rs`

Tests:
- `causation_id_lookup_returns_correct_position`
- `same_causation_id_is_rejected_idempotently`
- `different_causation_id_appends_new_event`
- `none_causation_id_events_always_accepted`
- `tagged_and_untagged_events_coexist`
- `exactly_once_guarantee_under_repeated_redelivery`
- `find_by_causation_id_returns_first_match`
