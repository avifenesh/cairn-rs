# STATUS: cost_tracking

**Task:** RFC 009 + GAP-003 per-provider cost tracking integration test  
**Tests passed:** 7/7  
**File:** `crates/cairn-store/tests/cost_tracking.rs`

Tests:
- `run_cost_accumulates_across_calls`
- `run_costs_are_independent_per_run`
- `session_cost_accumulates_across_calls`
- `derived_run_cost_updated_event_in_log`
- `derived_session_cost_updated_event_in_log`
- `budget_spend_accumulates_from_session_cost_events`
- `zero_cost_call_increments_count_without_inflating_totals`
