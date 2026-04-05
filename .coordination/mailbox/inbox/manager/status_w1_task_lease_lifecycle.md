# STATUS: task_lease_lifecycle

**Task:** RFC 002 task lease lifecycle test  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/task_lease_lifecycle.rs`

Tests:
- `task_lease_claimed_updates_record`
- `task_lease_heartbeat_extends_expiry`
- `expired_leases_detected_at_read_time`
- `expired_lease_allows_reclaim_after_recovery`
- `list_by_parent_run_returns_tasks_with_lease_state`
- `concurrent_leases_on_different_tasks`

Key: TaskLeaseClaimed sets metadata only — TaskStateChanged(Leased) transitions state.
Expiry is detected at READ TIME via list_expired_leases(now_ms), not via event projection.
