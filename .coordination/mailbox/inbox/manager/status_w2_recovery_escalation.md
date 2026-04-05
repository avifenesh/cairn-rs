# STATUS: recovery_escalation

**Task:** RFC 002 recovery escalation hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/recovery_escalation.rs`

Tests:
- `recovery_attempted_is_stored_in_event_log`
- `recovery_completed_success_marks_recovered`
- `recovery_escalated_stored_with_all_fields`
- `escalation_chain_is_queryable_in_causal_order`
- `recovery_completed_false_is_distinct_from_success`
- `recovery_events_are_isolated_by_run`

Key: RecoveryAttempted/RecoveryCompleted/RecoveryEscalated are no-op projections NOT in event_matches_entity — read_by_entity doesn't surface them. Tests use read_stream filtered by run_id via a helper. Helper: recovery_events_for_run().
